use crate::gitlab::{Gitlab, PayloadBuilder};
use crate::seed::{SeedIterator, merge_user_defined_seeds};
use clap::Parser;
use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;
use subprocess::{PopenConfig, Redirection};
use tracing::{info, warn};

mod gitlab;
mod seed;

const DEFAULT_CHUNK_SIZE: usize = 10;
const DEFAULT_TIMEOUT_SECS: u64 = 120;

fn default_fdbserver_path() -> String {
    String::from("/usr/sbin/fdbserver")
}

#[derive(clap::Parser, Debug, Clone)]
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
    token: Option<String>,
    /// Gitlab endpoint to use
    #[clap(long, env = "GITLAB_URL", default_value = "gitlab.com")]
    gitlab_url: String,
    /// Gitlab project id where to create the issue
    /// Optional; required only when a token is provided
    #[clap(long, env = "GITLAB_PROJECT_ID")]
    gitlab_project_id: Option<u64>,
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
    /// Stop the run after the first faulty seed is found
    #[clap(long)]
    fail_fast: bool,
    /// Timeout (in seconds) to wait for each simulation before terminating it
    #[clap(long = "timeout-secs", env = "TIMEOUT_SECS", default_value_t = DEFAULT_TIMEOUT_SECS)]
    timeout_secs: u64,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Build GitLab API client only if token and project_id are provided
    let api: Option<Gitlab> = match (&cli.token, &cli.gitlab_project_id) {
        (Some(token), Some(project_id)) => {
            info!(
                host = cli.gitlab_url,
                project_id, "Export reports to GitLab"
            );

            Some(
                gitlab::GitlabBuilder::default()
                    .token(token.as_str())
                    .endpoint(cli.gitlab_url.as_str())
                    .project_id(*project_id)
                    .build()?,
            )
        }
        _ => {
            info!("No GitLab API configured, skipping GitLab export");
            None
        }
    };

    let user_defined_seeds = merge_user_defined_seeds(cli.seeds.clone(), &cli.seed_file)?;

    let seed_iterator = SeedIterator::new(user_defined_seeds);

    if let Some(max_iteration) = cli.max_iterations {
        run_seeds(
            seed_iterator.take(max_iteration as usize),
            &cli,
            api.as_ref(),
            cli.chunk_size,
        )?;
    } else {
        run_seeds(seed_iterator, &cli, api.as_ref(), cli.chunk_size)?;
    }

    Ok(())
}

fn run_seeds(
    seed_iterator: impl Iterator<Item = u32>,
    cli: &Cli,
    api: Option<&Gitlab>,
    chunk_size: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use a small worker pool pattern by throttling the number of in-flight tasks to chunk_size.
    use std::sync::mpsc;

    let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);

    let size = seed_iterator.size_hint();

    let end = if let Some(end) = size.1 {
        format!("{end}")
    } else {
        "inf".to_string()
    };

    let (tx, rx) = mpsc::channel::<()>();
    let mut inflight = 0usize;
    let mut checked_seeds = 0usize;

    // Shared references for threads
    let cli_arc = std::sync::Arc::new(cli.clone());
    let api_arc: Option<std::sync::Arc<Gitlab>> = api.cloned().map(std::sync::Arc::new);

    for seed in seed_iterator {
        // If we already have max parallel jobs running, wait for one to finish.
        if inflight >= chunk_size {
            if rx.recv().is_ok() {
                inflight -= 1;
                checked_seeds += 1;
                info!("Progress [{checked_seeds}/{end}]");
            }
        }

        let tx_cloned = tx.clone();
        let cli_for_thread = std::sync::Arc::clone(&cli_arc);
        let api_for_thread = api_arc.as_ref().map(std::sync::Arc::clone);
        info!(seed, "Preparing to check seed");
        std::thread::spawn(move || {
            // Note: run_seed may exit the process on faulty seed according to settings.
            if let Err(e) = run_seed(seed, &cli_for_thread, api_for_thread) {
                warn!(seed, error = ?e, "failed to run seed");
            }
            // Notify completion; ignore send errors if receiver is dropped due to early exit
            let _ = tx_cloned.send(());
        });
        inflight += 1;
    }

    // Wait for all in-flight tasks to finish
    while inflight > 0 {
        if rx.recv().is_ok() {
            inflight -= 1;
            checked_seeds += 1;
            info!("Progress [{checked_seeds}/{end}]");
        }
    }

    Ok(())
}

fn run_seed(seed: u32, cli: &std::sync::Arc<Cli>, api: Option<std::sync::Arc<Gitlab>>) -> Result<(), Box<dyn std::error::Error>> {
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

    match process.wait_timeout(Duration::from_secs(cli.timeout_secs)) {
        Ok(Some(exit_status)) => {
            // Process finished within timeout; now read stdout/stderr
            let (stdout, stderr) = process.communicate(None)?;
            if !exit_status.success() {
                handle_faulty_seed(
                    &logs_dir,
                    stdout,
                    stderr,
                    seed,
                    cli.commit_id.clone(),
                    api.as_deref(),
                    cli.fail_fast,
                )?;
            } else {
                info!(seed, "Finished check seed no error found");
            }
        }
        Ok(None) => {
            // Timed out
            warn!(
                seed,
                timeout_secs = cli.timeout_secs,
                "Timeout reached; terminating process and continuing"
            );
            if let Err(e) = process.terminate() {
                warn!(seed, error = ?e, "Failed to terminate process");
            }
            // Do not treat as error; continue with next seeds
        }
        Err(e) => {
            // An actual error while waiting; try to terminate and bubble up the error
            warn!(seed, error = ?e, "Error while waiting for process; terminating");
            if let Err(e2) = process.terminate() {
                warn!(seed, error = ?e2, "Failed to terminate process");
            }
            return Err(Box::<dyn std::error::Error>::from(e));
        }
    }

    Ok(())
}

fn handle_faulty_seed(
    logs_dir: &PathBuf,
    stdout: Option<String>,
    stderr: Option<String>,
    seed: u32,
    commit_id: Option<String>,
    api: Option<&Gitlab>,
    fail_fast: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    warn!(seed, "Faulty seed found");

    // Build filtered_output from logs (Rust layer, severity 40)
    let mut compiled = jq_rs::compile(r#"select(.Layer=="Rust") | select(.Severity=="40")"#)?;

    let mut filtered_output = String::new();

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
                filtered_output.push_str(&pretty);
                filtered_output.push('\n');
            }
        }
    }

    // If no GitLab API is configured, display stdout, stderr, and filtered_output then exit faulty
    if api.is_none() {
        println!("stdout:\n");
        if let Some(out) = &stdout {
            println!("{}", out);
        }
        println!("stderr:\n");
        if let Some(err) = &stderr {
            eprintln!("{}", err);
        }
        println!("layer errors (filtered_output):\n");
        if !filtered_output.is_empty() {
            println!("{}", filtered_output);
        }
        std::process::exit(1)
    }

    let payload = PayloadBuilder::default()
        .logs(logs_dir)
        .filtered_output(filtered_output)
        .stdout(stdout)
        .stderr(stderr)
        .seed(seed)
        .commit_id(commit_id)
        .build()?;

    if let Some(api) = api {
        api.create_issue(payload)?;
        if fail_fast {
            std::process::exit(1)
        }
    }
    Ok(())
}
