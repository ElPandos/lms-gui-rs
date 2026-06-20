//! LM Studio client — communicates via HTTP API and SSH CLI commands.
//!
//! When `LMS_LOCAL=1` env var is set, commands run locally (no SSH).
//! Otherwise, commands are sent over SSH with multiplexing.

use reqwest::Client;
use crate::models::{Model, ModelsResponse, RuntimeInfo};
use std::process::Command;

fn ssh_host() -> String {
    let user = std::env::var("ENV_USER_JUMP_155_HOST").expect("ENV_USER_JUMP_155_HOST must be set");
    let ip = std::env::var("ENV_IP_JUMP_155_HOST").expect("ENV_IP_JUMP_155_HOST must be set");
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
    pub local_mode: bool,
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
    /// Create a new client, establishing an SSH ControlMaster if in remote mode.
    pub fn new(base_url: String) -> Self {
        let local_mode = std::env::var("LMS_LOCAL").unwrap_or_default() == "1";

        if !local_mode {
            tracing::info!("Establishing SSH ControlMaster connection");
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

        tracing::info!(local_mode, base_url = %base_url, "LmsClient initialized");

        Self {
            base_url,
            client: Client::new(),
            local_mode,
        }
    }

    /// Fetch the model list from the LMS REST API.
    pub async fn list_models(&self) -> Result<Vec<Model>, String> {
        tracing::debug!("Fetching model list from LMS API");
        let resp = self.client
            .get(format!("{}/v1/models", self.base_url))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to connect to LMS API");
                format!("Failed to connect to LMS: {}", e)
            })?;

        let body: ModelsResponse = resp
            .json()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to parse LMS models response");
                format!("Failed to parse response: {}", e)
            })?;

        tracing::debug!(count = body.data.len(), "Fetched models from API");
        Ok(body.data)
    }

    /// List locally-downloaded models via `lms ls`.
    pub async fn list_local_models(&self) -> Result<String, String> {
        self.run_cmd("lms ls").await
    }

    /// List currently loaded models via `lms ps`.
    pub async fn list_loaded_models(&self) -> Result<String, String> {
        self.run_cmd("lms ps").await
    }

    /// Start a background model download via `lms get`.
    pub async fn download_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        tracing::info!(model = %safe_name, "Starting model download");
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        // Kill any existing download for this model before starting fresh
        let _ = self.run_cmd(&format!("pkill -f 'lms get {}' 2>/dev/null; rm -f {}", safe_name, log_file)).await;
        self.run_cmd(&format!(
            "(lms get {} -y > {} 2>&1 &) && echo \"started\"",
            safe_name, log_file
        )).await
    }

    /// Unload and delete a model from disk.
    pub async fn delete_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        tracing::warn!(model = %safe_name, "Deleting model from disk");
        // Model name like "google/gemma-4-e2b" → search for directory containing "gemma-4-e2b"
        let search_part = safe_name.split('/').next_back().unwrap_or(&safe_name);
        self.run_cmd(&format!(
            "lms unload {} 2>/dev/null; dir=$(find $HOME/.lmstudio/models -maxdepth 2 -type d -iname '*{}*' | head -1); if [ -n \"$dir\" ]; then rm -rf \"$dir\" && echo 'Deleted'; else echo 'Not found'; fi",
            safe_name, search_part
        )).await
    }

    /// Get the tail of a model's download log file.
    pub async fn download_status(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        self.run_cmd(&format!("tail -c 500 {} 2>/dev/null || echo 'no log'", log_file)).await
    }

    /// Switch the active inference runtime via `lms runtime select`.
    pub async fn select_runtime(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        self.run_cmd(&format!("lms runtime select {}", safe_name)).await
    }

    /// Apply pending runtime updates.
    pub async fn update_runtime(&self) -> Result<String, String> {
        self.run_cmd("lms runtime update --apply 2>&1 || lms runtime update -y 2>&1").await
    }


    /// Load a model into GPU memory in the background.
    pub async fn load_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        tracing::info!(model = %safe_name, "Loading model into GPU memory");
        self.run_cmd(&format!(
            "(lms load {} --gpu max -y > /tmp/lms-load.log 2>&1 &) && echo 'Loading started'",
            safe_name
        )).await
    }

    /// Check the current model load status from the log file.
    pub async fn load_status(&self) -> Result<String, String> {
        self.run_cmd("tail -3 /tmp/lms-load.log 2>/dev/null || echo 'idle'").await
    }

    /// Unload a model (or all models with `--all`) from memory.
    pub async fn unload_model(&self, name: &str) -> Result<String, String> {
        tracing::info!(model = %name, "Unloading model");
        if name == "--all" {
            self.run_cmd("lms unload --all").await
        } else {
            let safe_name = Self::sanitize(name);
            self.run_cmd(&format!("lms unload {}", safe_name)).await
        }
    }

    /// Fetch runtime list and update status concurrently.
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

    /// Gather host hardware info (CPU, RAM, GPU, disk, uptime).
    pub async fn host_info(&self) -> Result<String, String> {
        self.run_cmd(
            "echo \"$(hostname)|$(lscpu | grep 'Model name' | sed 's/.*: *//')|$(lscpu | grep 'Socket' | awk '{print $NF}')x$(lscpu | grep 'Core(s) per socket' | awk '{print $NF}')c|$(free -h | awk '/Mem/{print $2}' | sed 's/i$//')|$(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader 2>/dev/null | head -1 || echo 'N/A')|$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | wc -l)|$(uptime -p)|$(df -h $HOME/.lmstudio/models 2>/dev/null | awk 'NR==2{print $2\"/\"$4}' || df -h $HOME | awk 'NR==2{print $2\"/\"$4}')|$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1 || echo 'N/A')|$(nvidia-smi | grep 'CUDA Version' | awk '{print $9}' 2>/dev/null || echo 'N/A')|$(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d'\"' -f2 || echo 'Unknown')\""
        ).await
    }

    /// Search for models on LM Studio Hub via `lms get`.
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

    /// Stream recent LMS inference logs.
    pub async fn recent_logs(&self) -> Result<String, String> {
        self.run_cmd("timeout 5 lms log stream 2>&1; true").await
    }

    /// Query current GPU memory usage via `nvidia-smi`.
    pub async fn gpu_memory(&self) -> Result<String, String> {
        self.run_cmd("nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits 2>/dev/null | awk -F', ' '{u+=$1;t+=$2} END{printf \"%dG / %dG\", u/1024, t/1024}'").await
    }

    /// Read the last 500 lines of the application log.
    pub async fn app_log(&self) -> Result<String, String> {
        self.run_cmd("tail -500 $HOME/lms-gui-rs/lms-gui-rs.log* 2>/dev/null | tail -500 || echo 'No app log'").await
    }

    /// List available download log file names.
    pub async fn download_logs(&self) -> Result<String, String> {
        // List available download log files
        self.run_cmd("for f in /tmp/lms-dl-*.log; do [ -f \"$f\" ] && basename \"$f\" .log | sed 's/lms-dl-//'; done 2>/dev/null").await
    }

    /// Read the tail of a specific download log.
    pub async fn download_log_content(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let log_file = format!("/tmp/lms-dl-{}.log", safe_name.replace('/', "_"));
        self.run_cmd(&format!("tail -c 500 {} 2>/dev/null || echo 'No log found'", log_file)).await
    }

    fn sanitize(input: &str) -> String {
        input.replace(|c: char| !c.is_alphanumeric() && c != '/' && c != '-' && c != '_' && c != '.' && c != ':' && c != '@', "")
    }

    /// Send a chat completion request to the LMS API.
    pub async fn chat_completion(&self, req: &crate::handlers::ChatRequest) -> Result<(String, u32), String> {
        tracing::debug!(model = %req.model, temperature = req.temperature, max_tokens = req.max_tokens, "Sending chat completion request");
        let mut messages = Vec::new();
        if let Some(ref sys) = req.system_prompt {
            if !sys.is_empty() {
                messages.push(serde_json::json!({"role": "system", "content": sys}));
            }
        }
        messages.push(serde_json::json!({"role": "user", "content": req.message}));

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
            "stream": false,
        });

        if let Some(top_p) = req.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if let Some(fp) = req.frequency_penalty {
            body["frequency_penalty"] = serde_json::json!(fp);
        }
        if let Some(pp) = req.presence_penalty {
            body["presence_penalty"] = serde_json::json!(pp);
        }

        let resp = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(model = %req.model, error = %e, "Chat completion request failed");
                format!("Request failed: {}", e)
            })?;

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| {
                tracing::error!(model = %req.model, error = %e, "Chat completion parse error");
                format!("Parse error: {}", e)
            })?;

        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .or_else(|| data["choices"][0]["delta"]["content"].as_str())
            .or_else(|| data["choices"][0]["text"].as_str())
            .unwrap_or("")
            .to_string();

        let tokens = data["usage"]["total_tokens"]
            .as_u64()
            .or_else(|| data["usage"]["completion_tokens"].as_u64())
            .unwrap_or(0) as u32;

        Ok((content, tokens))
    }

    async fn run_cmd(&self, cmd: &str) -> Result<String, String> {
        tracing::debug!(cmd = %cmd, local = self.local_mode, "Executing command");
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
        .map_err(|_| {
            tracing::error!(cmd = %cmd, "Command timed out after 20s");
            "Command timed out after 20s".to_string()
        })?
        .map_err(|e| {
            tracing::error!(cmd = %cmd, error = %e, "Task join error");
            format!("Task join error: {}", e)
        })?
        .map_err(|e| {
            tracing::error!(cmd = %cmd, error = %e, "Command execution error");
            format!("Command error: {}", e)
        })?;

        if result.status.success() {
            tracing::debug!(cmd = %cmd, "Command completed successfully");
            Ok(strip_ansi(&String::from_utf8_lossy(&result.stdout)))
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();
            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            if !stdout.is_empty() {
                tracing::debug!(cmd = %cmd, "Command returned non-zero but has stdout");
                Ok(strip_ansi(&stdout))
            } else {
                tracing::warn!(cmd = %cmd, stderr = %stderr, "Command failed");
                Err(format!("Command failed: {}", stderr))
            }
        }
    }
}
