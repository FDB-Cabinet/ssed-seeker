use crate::gitlab::{Gitlab, PayloadBuilder};
use crate::seed::{merge_user_defined_seeds, SeedIterator};
use clap::Parser;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;
use subprocess::{PopenConfig, Redirection};
use tracing::{debug, info, warn};

mod gitlab;
mod seed;

const DEFAULT_CHUNK_SIZE: usize = 10;

fn default_fdbserver_path() -> String {
    String::from("/usr/sbin/fdbserver")
}

#[derive(clap::Parser, Debug)]
struct Cli {
    /// Path to fdbserver binary
    #[clap(long, default_value_t = default_fdbserver_path())]
    fdbserver_path: String,
    /// Path to test file to run
    #[clap(long, short = 'f')]
    test_file: String,
    /// Max iterations to run
    #[clap(long)]
    max_iterations: Option<u64>,
    /// Gitlab token to use
    #[clap(long, env = "GITLAB_TOKEN", hide_env_values = true)]
    token: String,
    /// Gitlab endpoint to use
    #[clap(long, env = "GITLAB_URL", default_value = "gitlab.com")]
    gitlab_url: String,
    /// Gitlab project id where to create the issue
    #[clap(long, env = "GITLAB_PROJECT_ID")]
    gitlab_project_id: u64,
    /// Git commit ID
    #[clap(long)]
    commit_id: Option<String>,
    /// Seed file to use
    #[clap(long)]
    seed_file: Option<String>,
    /// Seeds to use
    #[clap(long)]
    seeds: Option<Vec<u32>>,
    /// Number of seeds to run in parallel
    #[clap(long)]
    chunk_size: Option<usize>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let api = gitlab::GitlabBuilder::default()
        .token(cli.token.as_str())
        .endpoint(cli.gitlab_url.as_str())
        .project_id(cli.gitlab_project_id)
        .build()?;

    let user_defined_seeds = merge_user_defined_seeds(cli.seeds.clone(), &cli.seed_file)?;

    let seed_iterator = SeedIterator::new(user_defined_seeds);

    if let Some(max_iteration) = cli.max_iterations {
        run_seeds(
            seed_iterator.take(max_iteration as usize),
            &cli,
            &api,
            cli.chunk_size,
        )?;
    } else {
        run_seeds(seed_iterator, &cli, &api, cli.chunk_size)?;
    }

    Ok(())
}

fn run_seeds(
    seed_iterator: impl Iterator<Item = u32>,
    cli: &Cli,
    api: &Gitlab,
    chunk_size: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut accumulator = vec![];

    let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);

    let size = seed_iterator.size_hint();

    let end = if let Some(end) = size.1 {
        format!("{end}")
    } else {
        "inf".to_string()
    };

    let mut checked_seeds = 0;

    for seed in seed_iterator {
        info!(seed, "Preparing to check seed");
        accumulator.push(seed);
        if accumulator.len() >= chunk_size {
            debug!(?accumulator, "Running seeds");
            info!("Running seeds [{checked_seeds}/{end}]");
            accumulator.par_iter().for_each(|seed| {
                run_seed(*seed, cli, api).expect("failed to run seed");
            });
            checked_seeds += accumulator.len();
            accumulator.clear();
        }
    }
    if !accumulator.is_empty() {
        info!("Tearing down remaining seeds");
        info!("Running seeds [{checked_seeds}/{end}]");
        accumulator.par_iter().for_each(|seed| {
            run_seed(*seed, cli, api).expect("failed to run seed");
        })
    }

    Ok(())
}

fn run_seed(seed: u32, cli: &Cli, api: &Gitlab) -> Result<(), Box<dyn std::error::Error>> {
    info!(seed, "Starting to check seed");

    let data_dir = tempfile::tempdir()?;

    let simfdb_data_dir = data_dir.path().join("simfdb");
    let logs_dir = data_dir.path().join("logs");

    std::fs::create_dir_all(&logs_dir)?;

    let config = PopenConfig {
        stdout: Redirection::Pipe,
        stderr: Redirection::Pipe,
        ..Default::default()
    };

    let mut process = subprocess::Popen::create(
        &[
            cli.fdbserver_path.as_str(),
            "-r",
            "simulation",
            "-b",
            "on",
            "--trace-format",
            "json",
            "-f",
            cli.test_file.as_str(),
            "-d",
            simfdb_data_dir
                .to_str()
                .expect("failed to get simfdb data dir path"),
            "-L",
            logs_dir.to_str().expect("failed to get logs dir path"),
            "-s",
            &seed.to_string(),
        ],
        config,
    )?;

    let (stdout, stderr) = process.communicate(None)?;

    let Ok(Some(exit_status)) = process.wait_timeout(Duration::from_secs(120)) else {
        process.terminate()?;
        return Err("Failed to terminate process".into());
    };

    if !exit_status.success() {
        handle_faulty_seed(&logs_dir, stdout, stderr, seed, cli.commit_id.clone(), &api)?;
    } else {
        info!(seed, "Finished check seed no error found");
    }

    Ok(())
}

fn handle_faulty_seed(
    logs_dir: &PathBuf,
    stdout: Option<String>,
    stderr: Option<String>,
    seed: u32,
    commit_id: Option<String>,
    api: &Gitlab,
) -> Result<(), Box<dyn std::error::Error>> {
    warn!(seed, "Faulty seed found");
    let mut compiled = jq_rs::compile(r#"select(.Layer=="Rust") | select(.Severity=="40")"#)?;

    let mut filtered_output = "".to_string();

    for file in walkdir::WalkDir::new(logs_dir.clone()) {
        let file = file?;
        if file.path().extension().unwrap_or_default() == "json" {
            let file = std::fs::File::open(file.path())?;
            let reader = std::io::BufReader::new(file);

            for line in reader.lines() {
                let logs = compiled.run(&line?)?;
                if logs.is_empty() {
                    continue;
                }
                let pretty = jsonxf::pretty_print(&logs)?;
                // println!("{}", pretty.to_colored_json_auto()?);
                filtered_output.push_str(&pretty);
                filtered_output.push('\n');
            }
        }
    }

    let payload = PayloadBuilder::default()
        .logs(logs_dir)
        .filtered_output(filtered_output)
        .stdout(stdout)
        .stderr(stderr)
        .seed(seed)
        .commit_id(commit_id)
        .build()?;

    api.create_issue(payload)?;
    Ok(())
}
