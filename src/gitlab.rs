use derive_builder::Builder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::trace;

#[derive(Debug, Builder)]
#[builder(setter(into))]
pub struct Gitlab {
    endpoint: String,
    token: String,
    project_id: u64,
}

#[derive(Debug, Builder)]
#[builder(setter(into))]
pub struct Payload {
    /// Json files filtered by Layer and Severity
    filtered_output: String,
    /// raw stdout output
    stdout: Option<String>,
    /// raw stderr output
    stderr: Option<String>,
    /// seed used for the test
    seed: u32,
    /// commit id of the tested workload if any
    commit_id: Option<String>,
    /// path to the logs folder
    logs: PathBuf,
}

impl Gitlab {
    pub fn upload_file(&self, path_buf: PathBuf) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::blocking::Client::new();
        let request = client
            .post(format!(
                "https://{}/api/v4/projects/{}/uploads",
                self.endpoint, self.project_id
            ))
            .multipart(reqwest::blocking::multipart::Form::new().file("file", path_buf)?)
            .header("PRIVATE-TOKEN", &self.token)
            .build()?;

        let response = client.execute(request)?;
        let text_response = response.text()?;
        let url = serde_json::from_str::<UploadResponse>(&text_response)?.url;
        Ok(url)
    }

    pub fn upload_from_string(
        &self,
        name: &str,
        string: &String,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        let path = tempdir.path().join(name);
        std::fs::write(&path, string)?;
        self.upload_file(path)
    }

    pub fn upload_file_from_path(
        &self,
        name: &str,
        path: &PathBuf,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        let tar_path = tempdir.path().join(name);
        let tar = File::create(&tar_path).unwrap();
        let enc = GzEncoder::new(tar, Compression::default());
        let mut tar_builder = tar::Builder::new(enc);
        tar_builder.append_dir_all("", path)?;
        let mut gzip_encoder = tar_builder.into_inner().unwrap();
        gzip_encoder.try_finish()?;

        self.upload_file(tar_path)
    }

    pub fn create_issue(&self, payload: Payload) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::blocking::Client::new();
        let seed = payload.seed;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let upload_url_stdout = self.upload_from_string(
            &format!("simulation_stdout_seed_{seed}_{now}.txt"),
            &payload.stdout.unwrap_or_default(),
        )?;
        let upload_url_stderr = self.upload_from_string(
            &format!("simulation_stderr_seed_{seed}_{now}.txt"),
            &payload.stderr.unwrap_or_default(),
        )?;
        let upload_url_logs = self.upload_file_from_path(
            &format!("simulation_logs_seed_{seed}_{now}.tar.gz"),
            &payload.logs,
        )?;

        let commit_id = payload.commit_id.unwrap_or("Non specified".to_string());
        let filtered_output = payload.filtered_output;

        let params = HashMap::from([
            (
                "title",
                format!("Investigate Faulty Seed #{}", payload.seed),
            ),
            (
                "description",
                format!(
                    r#"- Commit ID: {commit_id}
- Output: [simulation.out]({upload_url_stdout})
- Stderr : [simulation.err]({upload_url_stderr})
- Full logs: [logs.tar.gz]({upload_url_logs})
- Layer errors:
```json
{filtered_output}
```
"#,
                ),
            ),
        ]);

        let params = serde_json::to_string(&params)?;

        let request = client
            .post(format!(
                "https://{}/api/v4/projects/{}/issues",
                self.endpoint, self.project_id
            ))
            .body(params)
            .header("PRIVATE-TOKEN", &self.token)
            .header("Content-Type", "application/json")
            .build()?;

        let response = client.execute(request)?;
        trace!(?response, "Gitlab create issue response");

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    url: String,
}
