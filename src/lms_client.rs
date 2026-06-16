//! LM Studio client — communicates via HTTP API and SSH CLI commands.
//!
//! When `LMS_LOCAL=1` env var is set, commands run locally (no SSH).
//! Otherwise, commands are sent over SSH with multiplexing.

use reqwest::Client;
use crate::models::{Model, ModelsResponse, RuntimeInfo};
use std::process::Command;

fn ssh_host() -> String {
    let user = std::env::var("ENV_USER_JUMP_231_HOST").expect("ENV_USER_JUMP_231_HOST must be set");
    let ip = std::env::var("ENV_IP_JUMP_231_HOST").expect("ENV_IP_JUMP_231_HOST must be set");
    format!("{}@{}", user, ip)
}
const SSH_MUX_PATH: &str = "/tmp/lms-gui-ssh-mux";

/// HTTP + SSH client for LM Studio operations.
///
/// Uses SSH multiplexing (ControlMaster) for low-latency CLI commands
/// and reqwest for the REST API. Set `LMS_LOCAL=1` to run commands
/// locally without SSH (when deployed on the LMS host).
#[derive(Clone)]
pub struct LmsClient {
    pub base_url: String,
    client: Client,
    local_mode: bool,
}

/// Strip ANSI escape sequences and carriage returns from terminal output.
fn strip_ansi(input: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07|\r").unwrap()
    });
    RE.replace_all(input, "").to_string()
}

impl LmsClient {
    pub fn new(base_url: String) -> Self {
        let local_mode = std::env::var("LMS_LOCAL").unwrap_or_default() == "1";

        if !local_mode {
            // Start SSH ControlMaster in background
            let _ = Command::new("ssh")
                .args([
                    "-o", "ControlMaster=auto",
                    "-o", &format!("ControlPath={}", SSH_MUX_PATH),
                    "-o", "ControlPersist=600",
                    "-o", "ConnectTimeout=5",
                    "-fN",
                    &ssh_host(),
                ])
                .spawn();
        }

        if local_mode {
            tracing::info!("Running in LOCAL mode (no SSH)");
        }

        Self {
            base_url,
            client: Client::new(),
            local_mode,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<Model>, String> {
        let resp = self.client
            .get(format!("{}/v1/models", self.base_url))
            .send()
            .await
            .map_err(|e| format!("Failed to connect to LMS: {}", e))?;

        let body: ModelsResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(body.data)
    }

    pub async fn list_local_models(&self) -> Result<String, String> {
        self.run_cmd("lms ls").await
    }

    pub async fn list_loaded_models(&self) -> Result<String, String> {
        self.run_cmd("lms ps").await
    }

    pub async fn download_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        // Kill any existing download for this model before starting fresh
        let _ = self.run_cmd(&format!("pkill -f 'lms get {}' 2>/dev/null; rm -f {}", safe_name, log_file)).await;
        self.run_cmd(&format!(
            "(lms get {} -y > {} 2>&1 &) && echo \"started\"",
            safe_name, log_file
        )).await
    }

    pub async fn delete_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        // Model name like "google/gemma-4-e2b" → search for directory containing "gemma-4-e2b"
        let search_part = safe_name.split('/').next_back().unwrap_or(&safe_name);
        self.run_cmd(&format!(
            "lms unload {} 2>/dev/null; dir=$(find $HOME/.lmstudio/models -maxdepth 2 -type d -iname '*{}*' | head -1); if [ -n \"$dir\" ]; then rm -rf \"$dir\" && echo 'Deleted'; else echo 'Not found'; fi",
            safe_name, search_part
        )).await
    }

    pub async fn download_status(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        self.run_cmd(&format!("tail -c 300 {} 2>/dev/null || echo 'no log'", log_file)).await
    }

    pub async fn select_runtime(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        self.run_cmd(&format!("lms runtime select {}", safe_name)).await
    }

    pub async fn load_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        self.run_cmd(&format!(
            "(lms load {} --gpu max -y > /tmp/lms-load.log 2>&1 &) && echo 'Loading started'",
            safe_name
        )).await
    }

    pub async fn load_status(&self) -> Result<String, String> {
        self.run_cmd("tail -3 /tmp/lms-load.log 2>/dev/null || echo 'idle'").await
    }

    pub async fn unload_model(&self, name: &str) -> Result<String, String> {
        if name == "--all" {
            self.run_cmd("lms unload --all").await
        } else {
            let safe_name = Self::sanitize(name);
            self.run_cmd(&format!("lms unload {}", safe_name)).await
        }
    }

    pub async fn runtime_status(&self) -> Result<RuntimeInfo, String> {
        let (ls, update) = tokio::join!(
            self.run_cmd("lms runtime ls"),
            self.run_cmd("lms runtime update 2>&1"),
        );
        Ok(RuntimeInfo {
            runtimes: ls?,
            update_status: update.unwrap_or_default(),
        })
    }

    pub async fn host_info(&self) -> Result<String, String> {
        self.run_cmd(
            "echo \"$(hostname)|$(lscpu | grep 'Model name' | sed 's/.*: *//')|$(lscpu | grep 'Socket' | awk '{print $NF}')x$(lscpu | grep 'Core(s) per socket' | awk '{print $NF}')c|$(free -h | awk '/Mem/{print $2}' | sed 's/i$//')|$(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader 2>/dev/null | head -1 || echo 'N/A')|$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | wc -l)|$(uptime -p)|$(df -h $HOME/.lmstudio/models 2>/dev/null | awk 'NR==2{print $2\"/\"$4}' || df -h $HOME | awk 'NR==2{print $2\"/\"$4}')|$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1 || echo 'N/A')|$(nvidia-smi | grep 'CUDA Version' | awk '{print $9}' 2>/dev/null || echo 'N/A')|$(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d'\"' -f2 || echo 'Unknown')\""
        ).await
    }

    pub async fn search_models(&self, query: &str) -> Result<String, String> {
        let mut cmd = String::from("timeout 15 lms get");
        if !query.is_empty() {
            cmd.push_str(&format!(" {}", Self::sanitize(query)));
        }
        cmd.push_str(" 2>&1 || true");
        self.run_cmd(&cmd).await
    }

    /// Search HuggingFace Hub for GGUF models via their API.
    pub async fn search_huggingface(&self, query: &str, sort: &str) -> Result<Vec<crate::models::HfModel>, String> {
        let sort_param = match sort {
            "likes" => "likes",
            "newest" => "lastModified",
            _ => "downloads",
        };
        let url = format!(
            "https://huggingface.co/api/models?filter=gguf&search={}&sort={}&direction=-1&limit=20",
            query, sort_param
        );
        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("HF API error: {}", e))?;
        let models: Vec<crate::models::HfModel> = resp
            .json()
            .await
            .map_err(|e| format!("HF parse error: {}", e))?;
        Ok(models)
    }

    pub async fn recent_logs(&self) -> Result<String, String> {
        self.run_cmd("timeout 3 lms log stream 2>&1; true").await
    }

    pub async fn gpu_memory(&self) -> Result<String, String> {
        self.run_cmd("nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits 2>/dev/null | awk -F', ' '{u+=$1;t+=$2} END{printf \"%dG / %dG\", u/1024, t/1024}'").await
    }

    pub async fn app_log(&self) -> Result<String, String> {
        self.run_cmd("tail -100 /home/hts/lms-gui-rs/lms-gui-rs.log 2>/dev/null || echo 'No app log'").await
    }

    pub async fn download_logs(&self) -> Result<String, String> {
        // List available download log files
        self.run_cmd("for f in /tmp/lms-dl-*.log; do [ -f \"$f\" ] && basename \"$f\" .log | sed 's/lms-dl-//'; done 2>/dev/null").await
    }

    pub async fn download_log_content(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        self.run_cmd(&format!("tail -c 500 {} 2>/dev/null || echo 'No log found'", log_file)).await
    }

    fn sanitize(input: &str) -> String {
        input.replace(|c: char| !c.is_alphanumeric() && c != '/' && c != '-' && c != '_' && c != '.' && c != ':' && c != '@', "")
    }

    async fn run_cmd(&self, cmd: &str) -> Result<String, String> {
        let local_mode = self.local_mode;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            tokio::task::spawn_blocking({
                let cmd = cmd.to_string();
                move || {
                    if local_mode {
                        Command::new("bash")
                            .args(["-c", &format!("export PATH=\"$HOME/.lmstudio/bin:$PATH\" && {}", cmd)])
                            .output()
                    } else {
                        Command::new("ssh")
                            .args([
                                "-o", "ControlMaster=auto",
                                "-o", &format!("ControlPath={}", SSH_MUX_PATH),
                                "-o", "ControlPersist=600",
                                "-o", "ConnectTimeout=5",
                                "-o", "ServerAliveInterval=5",
                                &ssh_host(),
                                &format!("export PATH=\"$HOME/.lmstudio/bin:$PATH\" && {}", cmd),
                            ])
                            .output()
                    }
                }
            }),
        )
        .await
        .map_err(|_| "Command timed out after 20s".to_string())?
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("Command error: {}", e))?;

        if result.status.success() {
            Ok(strip_ansi(&String::from_utf8_lossy(&result.stdout)))
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();
            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            if !stdout.is_empty() {
                Ok(strip_ansi(&stdout))
            } else {
                Err(format!("Command failed: {}", stderr))
            }
        }
    }
}
