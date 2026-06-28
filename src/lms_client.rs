//! LM Studio client — communicates via HTTP API and SSH CLI commands.
//!
//! When `LMS_LOCAL=1` env var is set, commands run locally (no SSH).
//! Otherwise, commands are sent over SSH with multiplexing.

use crate::models::{ActiveDownload, Model, ModelsResponse, RuntimeInfo};
use reqwest::Client;
use std::process::Command;

/// Build the `user@host` SSH target from `ENV_USER_JUMP_155_HOST` and
/// `ENV_IP_JUMP_155_HOST` environment variables (panics if unset).
fn ssh_host() -> String {
    let user = std::env::var("ENV_USER_JUMP_155_HOST").expect("ENV_USER_JUMP_155_HOST must be set");
    let ip = std::env::var("ENV_IP_JUMP_155_HOST").expect("ENV_IP_JUMP_155_HOST must be set");
    format!("{}@{}", user, ip)
}

/// Filesystem path for the SSH ControlMaster multiplexing socket.
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
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07|\r").unwrap());
    RE.replace_all(input, "").to_string()
}

/// Classify a reqwest error into a user-friendly message.
fn classify_http_error(e: &reqwest::Error, url: &str) -> String {
    if e.is_connect() {
        format!("Cannot reach LMS API at {} — is LM Studio running?", url)
    } else if e.is_timeout() {
        format!("LMS API timed out at {} — server may be overloaded", url)
    } else {
        format!("Failed to connect to LMS API at {}: {}", url, e)
    }
}

/// Classify a CLI command failure (stderr from run_cmd) into a user-friendly message.
fn classify_cli_error(stderr: &str) -> String {
    if stderr.contains("Invalid passkey") {
        "LMS CLI authentication failed — run 'lms server stop && lms server start' on the host"
            .to_string()
    } else if stderr.contains("command not found") || stderr.contains("not found") {
        "lms CLI not installed on host — verify $HOME/.lmstudio/bin is in PATH".to_string()
    } else {
        format!("Command failed: {}", stderr)
    }
}

impl LmsClient {
    /// Create a new client, establishing an SSH ControlMaster if in remote mode.
    pub fn new(base_url: String) -> Self {
        let local_mode = std::env::var("LMS_LOCAL").unwrap_or_default() == "1";

        if !local_mode {
            tracing::info!("Establishing SSH ControlMaster connection");
            let _ = Command::new("ssh")
                .args([
                    "-o",
                    "ControlMaster=auto",
                    "-o",
                    &format!("ControlPath={}", SSH_MUX_PATH),
                    "-o",
                    "ControlPersist=600",
                    "-o",
                    "ConnectTimeout=5",
                    "-fN",
                    &ssh_host(),
                ])
                .status();
            // Wait for the master socket to be ready (spawn is non-blocking)
            for i in 0..20 {
                if std::path::Path::new(SSH_MUX_PATH).exists() {
                    tracing::debug!(attempts = i, "SSH ControlMaster socket ready");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        tracing::info!(local_mode, base_url = %base_url, "LmsClient initialized");

        Self {
            base_url,
            client: Client::new(),
            local_mode,
        }
    }

    /// Probe LMS API and CLI reachability. Returns (api_ok, cli_ok, api_error, cli_error).
    pub async fn health_check(&self) -> (bool, bool, String, String) {
        // API check: GET {base_url}/v1/models with 3s timeout
        let api_result = {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap_or_else(|_| self.client.clone());
            let url = format!("{}/v1/models", self.base_url);
            client.get(&url).send().await
        };
        let (api_ok, api_error) = match api_result {
            Ok(resp) if resp.status().is_success() => (true, String::new()),
            Ok(resp) => (false, format!("HTTP {}", resp.status())),
            Err(e) => (
                false,
                classify_http_error(&e, &format!("{}/v1/models", self.base_url)),
            ),
        };

        // CLI check: run "lms --version" (lightweight, always works if CLI installed)
        let cli_result = self.run_cmd("lms --version 2>&1 || true").await;
        let (cli_ok, cli_error) = match cli_result {
            Ok(output) if output.contains("lms") || output.contains("version") => {
                (true, String::new())
            }
            Ok(output) => (false, output),
            Err(e) => (false, e),
        };

        (api_ok, cli_ok, api_error, cli_error)
    }

    /// Fetch the model list from the LMS REST API.
    pub async fn list_models(&self) -> Result<Vec<Model>, String> {
        tracing::debug!("Fetching model list from LMS API");
        let url = format!("{}/v1/models", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to connect to LMS API");
            classify_http_error(&e, &url)
        })?;

        let body: ModelsResponse = resp.json().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to parse LMS models response");
            format!("Failed to parse response: {}", e)
        })?;

        tracing::debug!(count = body.data.len(), "Fetched models from API");
        Ok(body.data)
    }

    /// Fetch the `max_context_length` for a model from the LMS v0 REST API.
    ///
    /// Queries `GET /api/v0/models` and locates the entry whose `id` matches
    /// `model_id`. Returns `Ok(Some(n))` if found, `Ok(None)` if the model
    /// isn't listed or has no `max_context_length`, and `Err` on HTTP/parse failure.
    pub async fn fetch_max_context(&self, model_id: &str) -> Result<Option<u64>, String> {
        let safe_id = Self::sanitize(model_id);
        tracing::debug!(model = %safe_id, "Querying v0 API for max_context_length");

        let url = format!("{}/api/v0/models", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            tracing::error!(model = %safe_id, error = %e, "Failed to connect to LMS v0 API");
            classify_http_error(&e, &url)
        })?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            tracing::error!(model = %safe_id, error = %e, "Failed to parse LMS v0 models response");
            format!("Failed to parse v0 response: {}", e)
        })?;

        let result = body["data"].as_array().and_then(|arr| {
            arr.iter().find_map(|m| {
                let id = m["id"].as_str()?;
                if id == safe_id {
                    m["max_context_length"].as_u64()
                } else {
                    None
                }
            })
        });

        tracing::debug!(model = %safe_id, result = ?result, "v0 API max_context_length lookup complete");
        Ok(result)
    }

    /// Fetch all models with rich metadata from LM Studio v0 REST API.
    pub async fn fetch_v0_models(&self) -> Result<Vec<crate::models::LmsV0Model>, String> {
        tracing::debug!("Fetching v0 model metadata from LMS API");
        let url = format!("{}/api/v0/models", self.base_url);
        let resp = self.client.get(&url).send().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch v0 models");
            classify_http_error(&e, &url)
        })?;
        let body: crate::models::LmsV0ModelsResponse = resp.json().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to parse v0 models response");
            format!("Failed to parse v0 response: {}", e)
        })?;
        tracing::debug!(count = body.data.len(), "Fetched v0 models");
        Ok(body.data)
    }

    /// List models with context metadata, merging `/v1/models` with `/api/v0/models`.
    ///
    /// The OpenAI-compatible `/v1/models` endpoint omits `max_context_length`
    /// (always null), so we join it with the v0 internal API which returns
    /// the real `max_context_length` and `loaded_context_length` for each model.
    /// Falls back to the bare v1 list if the v0 endpoint is unavailable.
    pub async fn list_models_with_context(&self) -> Result<Vec<Model>, String> {
        let (v1_result, v0_result) = tokio::join!(self.list_models(), self.fetch_v0_models());
        let mut v1 = v1_result?;
        let v0 = v0_result.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "v0 API unavailable; returning v1 models without context metadata");
            vec![]
        });
        if v0.is_empty() {
            return Ok(v1);
        }
        let v0_map: std::collections::HashMap<&str, &crate::models::LmsV0Model> =
            v0.iter().map(|m| (m.id.as_str(), m)).collect();
        for model in &mut v1 {
            if let Some(v0m) = v0_map.get(model.id.as_str()) {
                if model.max_context_length.is_none() {
                    model.max_context_length = v0m.max_context_length;
                }
            }
        }
        Ok(v1)
    }

    /// List locally-downloaded models via the v0 REST API (fast, structured, daemon-backed).
    /// Falls back to `lms ls` text parsing if the API is unavailable.
    pub async fn list_local_models_v0(&self) -> Result<Vec<crate::models::LocalModel>, String> {
        match self.fetch_v0_models().await {
            Ok(v0_models) => {
                // Resolve case-preserved filesystem paths with a single find command
                let find_output = self
                    .run_cmd("find $HOME/.lmstudio/models -maxdepth 2 -type d 2>/dev/null")
                    .await
                    .unwrap_or_default();
                let find_lines: Vec<&str> = find_output
                    .lines()
                    .filter(|l| !l.is_empty() && !l.ends_with("/models"))
                    .collect();

                let models: Vec<crate::models::LocalModel> = v0_models
                    .into_iter()
                    .map(|v0| {
                        // Match the v0 id (lowercase) against directory names (case-insensitive)
                        let file_path = find_lines
                            .iter()
                            .find(|line| {
                                let dir_name =
                                    line.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                                dir_name.eq_ignore_ascii_case(&v0.id)
                                    || dir_name.to_lowercase().contains(&v0.id.to_lowercase())
                            })
                            .map(|line| line.trim().to_string())
                            .unwrap_or_default();

                        crate::models::LocalModel {
                            name: v0.id,
                            params: String::new(), // v0 API doesn't provide param count
                            arch: v0.arch,
                            size: String::new(), // v0 API doesn't provide file size
                            device: String::new(),
                            status: if v0.state == "loaded" {
                                "LOADED".to_string()
                            } else {
                                String::new()
                            },
                            model_type: match v0.r#type.as_str() {
                                "llm" | "vlm" => "LLM".to_string(),
                                "embeddings" => "Embedding".to_string(),
                                _ => "LLM".to_string(),
                            },
                            max_context: v0
                                .max_context_length
                                .map(|n| n.to_string())
                                .unwrap_or_default(),
                            quantization: v0.quantization,
                            publisher: v0.publisher,
                            compat_type: v0.compatibility_type,
                            model_vtype: v0.r#type,
                            file_path,
                        }
                    })
                    .collect();
                tracing::debug!(count = models.len(), "Built local models from v0 API");
                Ok(models)
            }
            Err(e) => {
                tracing::warn!(error = %e, "v0 API failed, falling back to lms ls");
                let raw = self.list_local_models().await?;
                Ok(crate::models::parse_local_models(&raw))
            }
        }
    }

    /// List locally-downloaded models via `lms ls`.
    pub async fn list_local_models(&self) -> Result<String, String> {
        self.run_cmd("lms ls").await
    }

    /// List currently loaded models via `lms ps`.
    pub async fn list_loaded_models(&self) -> Result<String, String> {
        self.run_cmd("lms ps").await
    }

    /// Start a background model download via `lms get`, with PID tracking.
    pub async fn download_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let dl_id = LmsClient::dl_id(name);
        let log_file = format!("/tmp/lms-dl-{}.log", dl_id);
        let pid_file = format!("/tmp/lms-dl-{}.pid", dl_id);
        let name_file = format!("/tmp/lms-dl-{}.name", dl_id);
        tracing::info!(model = %safe_name, dl_id = %dl_id, "Starting model download");
        // Kill any existing download for this model before starting fresh.
        // The PID file now collides across publisher variants (dl_id normalizes
        // to the model basename), so a cross-publisher download of the same
        // model is stopped via its PID. The pkill fallback matches on the
        // repo basename so unrelated model downloads are left alone.
        // Also delete old .part files — lms get does NOT clean these up
        // automatically, so without this, every download attempt leaves
        // a stale .part file behind (especially when switching quants).
        let pkill_name = safe_name
            .rsplit('/')
            .next()
            .unwrap_or(&safe_name)
            .split('@')
            .next()
            .unwrap_or(&safe_name);
        let pkill_no_gguf = pkill_name
            .strip_suffix("-GGUF")
            .or_else(|| pkill_name.strip_suffix("-gguf"))
            .unwrap_or(pkill_name);
        let _ = self
            .run_cmd(&format!(
                "pid=$(cat {} 2>/dev/null); if [ -n \"$pid\" ]; then kill $pid 2>/dev/null; fi; \
                 pkill -f '[l]ms get.*{}' 2>/dev/null; \
                 sleep 0.5; rm -f {} {} {}; \
                 find $HOME/.lmstudio/models -maxdepth 3 -name 'downloading_*{}*.part' -delete 2>/dev/null; \
                 find $HOME/.lmstudio/models -maxdepth 3 -name 'downloading_*{}*.part' -delete 2>/dev/null",
                pid_file, pkill_name, log_file, pid_file, name_file,
                pkill_name, pkill_no_gguf
            ))
            .await;
        tracing::debug!(model = %safe_name, pkill = %pkill_name, "Pre-start cleanup done");
        // Start download in background, write PID and model name to files
        self.run_cmd(&format!(
            "(lms get {} -y > {} 2>&1 & echo $! > {}; echo '{}' > {}) && echo 'started'",
            safe_name, log_file, pid_file, safe_name, name_file
        ))
        .await
    }

    /// Cancel an in-progress download by killing the tracked PID.
    pub async fn cancel_download(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let dl_id = LmsClient::dl_id(name);
        let pid_file = format!("/tmp/lms-dl-{}.pid", dl_id);
        let log_file = format!("/tmp/lms-dl-{}.log", dl_id);
        let name_file = format!("/tmp/lms-dl-{}.name", dl_id);
        tracing::info!(model = %safe_name, dl_id = %dl_id, "Cancelling download");
        // 1. Kill the download process (by PID file, then pkill fallback)
        // 2. Remove PID and log files
        // 3. Delete partial download files (.part) left behind by lms get.
        //    `lms get` writes directly to the models directory with a .part suffix.
        //    If killed mid-download, these partial files remain on disk indefinitely.
        //    Use the basename from the model key as a case-insensitive wildcard to
        //    find the download directory (e.g. "gpt-oss-120b" matches "gpt-oss-120b-GGUF").
        //    Strip the @quant suffix — it's a download selector, not part of the
        //    on-disk directory name, otherwise the find won't match.
        let search_part = safe_name
            .split('/')
            .next_back()
            .unwrap_or(&safe_name)
            .split('@')
            .next()
            .unwrap_or(&safe_name);
        let search_no_gguf = search_part
            .strip_suffix("-GGUF")
            .or_else(|| search_part.strip_suffix("-gguf"))
            .unwrap_or(search_part);
        let cmd = format!(
            "pid=$(cat {} 2>/dev/null); if [ -n \"$pid\" ]; then kill $pid 2>/dev/null; fi; \
             pkill -f '[l]ms get.*{}' 2>/dev/null; \
             sleep 1; \
             rm -f {} {} {}; \
             find $HOME/.lmstudio/models -maxdepth 3 -name 'downloading_*{}*.part' -delete 2>/dev/null; \
             dir=$(find $HOME/.lmstudio/models -maxdepth 3 -type d -iname '*{}*' -o -type d -iname '*{}*' | head -1); \
             if [ -n \"$dir\" ]; then \
                 rm -rf \"$dir\"; \
                 echo 'cancelled and partial files deleted'; \
             else \
                 echo 'cancelled'; \
             fi",
            pid_file, search_part, pid_file, log_file, name_file,
            search_part, search_part, search_no_gguf
        );
        self.run_cmd(&cmd).await
    }

    /// Kill any orphaned download processes (call on server startup).
    /// Also cleans up stale .part files for downloads our app was tracking
    /// (identified by .name files). Downloads started from LM Studio's own
    /// UI (no .name file) are left alone.
    pub async fn reap_orphaned_downloads(&self) -> Result<String, String> {
        tracing::info!("Reaping orphaned download processes");
        // 1. Kill all lms get processes (our app's + any stale ones)
        // 2. Remove PID files
        // 3. For each .name file, delete stale .part files matching that model
        //    (these are leftovers from downloads that were interrupted by the
        //    app restarting — lms get doesn't clean up .part files on kill)
        let cmd = "pkill -f '[l]ms get' 2>/dev/null; \
                   for nf in /tmp/lms-dl-*.name; do \
                       [ -f \"$nf\" ] || continue; \
                       mname=$(cat \"$nf\" 2>/dev/null | tr -d '[:space:]'); \
                       [ -z \"$mname\" ] && continue; \
                       base=$(echo \"$mname\" | rev | cut -d/ -f1 | rev | cut -d@ -f1); \
                       noguf=$(echo \"$base\" | sed 's/-GGUF$//;s/-gguf$//'); \
                       find $HOME/.lmstudio/models -maxdepth 3 -name \"downloading_*${base}*.part\" -delete 2>/dev/null; \
                       find $HOME/.lmstudio/models -maxdepth 3 -name \"downloading_*${noguf}*.part\" -delete 2>/dev/null; \
                       rm -f \"$nf\"; \
                   done; \
                   rm -f /tmp/lms-dl-*.pid /tmp/lms-dl-*.log 2>/dev/null; \
                   echo 'reaped'";
        match self.run_cmd(cmd).await {
            Ok(msg) => Ok(msg),
            Err(e) => {
                // pkill exits non-zero when no process matches — that's the normal case here.
                // As long as SSH itself succeeded, treat as success.
                tracing::warn!(error = %e, "Reaper command non-zero (likely no orphans — this is normal)");
                Ok("reaped (no orphans)".to_string())
            }
        }
    }

    /// Unload and delete a model from disk (hardened: exact match, requires unload success).
    pub async fn delete_model(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        let dl_id = LmsClient::dl_id(name);
        let pid_file = format!("/tmp/lms-dl-{}.pid", dl_id);
        tracing::warn!(model = %safe_name, "Deleting model from disk");
        // Refuse to delete if a download for this model is still running
        let pid_check = self
            .run_cmd(&format!(
                "test -f {} && echo 'running' || echo 'ok'",
                pid_file
            ))
            .await
            .unwrap_or_default();
        if pid_check.contains("running") {
            return Err(
                "Cannot delete: a download for this model is still in progress. Cancel it first."
                    .to_string(),
            );
        }
        // Step 1: Unload from memory if loaded (use ; not && — model may not be in memory,
        // and that's fine; we still need to delete the files).
        // Step 2: Find and delete the model directory from disk.
        //   The LMS model identifier (e.g. "qwen3-4b") does NOT match the directory name
        //   on disk (e.g. "Qwen3-4B-GGUF"). Use case-insensitive wildcard match (-iname)
        //   to find the directory. The basename without the publisher prefix is used as
        //   the search term (e.g. "qwen3-4b" from "Qwen/qwen3-4b").
        // Step 3: Restart the LMS daemon so its internal index rescans and drops
        //         the deleted model. Without this, `lms ls` still shows it (there is
        //         no `lms remove` command — see lmstudio-ai/lms#579).
        let search_part = safe_name.split('/').next_back().unwrap_or(&safe_name);
        self.run_cmd(&format!(
            "lms unload {} 2>/dev/null; \
             dir=$(find $HOME/.lmstudio/models -maxdepth 3 -type d -iname '*{}*' | while read d; do \
                 if find \"$d\" -maxdepth 1 -name '*.gguf' | head -1 | grep -qi '{}'; then \
                     echo \"$d\"; break; \
                 fi; \
             done); \
             if [ -n \"$dir\" ]; then \
                 rm -rf \"$dir\"; \
                 echo 'Deleted'; \
             else \
                 echo 'Not found'; \
             fi",
            safe_name, search_part, search_part
        )).await
    }

    /// Get the tail of a model's download log file, plus the on-disk download
    /// directory if a `.part` file exists. The directory is determined by
    /// scanning `~/.lmstudio/models/` for a `.part` file matching the model
    /// basename — LM Studio may store under a different publisher than the
    /// URL passed to `lms get`, so this shows the *actual* on-disk path.
    pub async fn download_status(&self, name: &str) -> Result<String, String> {
        let log_file = format!("/tmp/lms-dl-{}.log", LmsClient::dl_id(name));
        // Find the on-disk download directory by looking for .part files
        // matching the model basename. Strip @quant and take the last path
        // segment. Also try without a trailing "-GGUF" suffix since `lms get`
        // drops it from the .part filename (e.g. "Qwen3-Coder-Next-GGUF" →
        // "downloading_Qwen3-Coder-Next-Q4_K_M.gguf.part").
        let safe_name = Self::sanitize(name);
        let search_part = safe_name
            .split('/')
            .next_back()
            .unwrap_or(&safe_name)
            .split('@')
            .next()
            .unwrap_or(&safe_name);
        let search_no_gguf = search_part
            .strip_suffix("-GGUF")
            .or_else(|| search_part.strip_suffix("-gguf"))
            .unwrap_or(search_part);
        self.run_cmd(&format!(
            "log=$(tail -c 500 {log} 2>/dev/null || echo 'no log'); \
             dir=$(find $HOME/.lmstudio/models -maxdepth 3 -name 'downloading_*{part}*.part' -print -quit 2>/dev/null); \
             if [ -z \"$dir\" ]; then \
                 dir=$(find $HOME/.lmstudio/models -maxdepth 3 -name 'downloading_*{part_noguf}*.part' -print -quit 2>/dev/null); \
             fi; \
             if [ -n \"$dir\" ]; then \
                 dpath=$(dirname \"$dir\"); \
                 echo \"PATH:$dpath\"; \
             fi; \
             echo \"$log\"",
            log = log_file,
            part = search_part,
            part_noguf = search_no_gguf
        ))
        .await
    }

    /// Switch the active inference runtime via `lms runtime select`.
    pub async fn select_runtime(&self, name: &str) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        self.run_cmd(&format!("lms runtime select {}", safe_name))
            .await
    }

    /// Apply pending runtime updates.
    pub async fn update_runtime(&self) -> Result<String, String> {
        self.run_cmd("lms runtime update --apply 2>&1 || lms runtime update -y 2>&1")
            .await
    }

    /// Load a model into GPU memory in the background.
    ///
    /// `context_length` and `parallel` are optional load-time overrides.
    /// `parallel` controls continuous-batching slots (default 4 in LMS);
    /// set to 1 for full per-request context (best for agents/IDEs).
    pub async fn load_model(
        &self,
        name: &str,
        context_length: Option<u64>,
        parallel: Option<u32>,
    ) -> Result<String, String> {
        let safe_name = Self::sanitize(name);
        tracing::info!(model = %safe_name, "Loading model into GPU memory");
        tracing::debug!(model = %safe_name, context_length = ?context_length, parallel = ?parallel, "Loading model with context length and parallel slots");
        let ctx_flag = match context_length {
            Some(n) if n > 0 => format!(" --context-length {}", n),
            _ => String::new(),
        };
        let parallel_flag = match parallel {
            Some(n) if n > 0 => format!(" --parallel {}", n),
            _ => String::new(),
        };
        self.run_cmd(&format!(
            "(lms load {} --gpu max{}{} > /tmp/lms-load.log 2>&1 &) && echo 'Loading started'",
            safe_name, ctx_flag, parallel_flag
        ))
        .await
    }

    /// Check the current model load status from the log file.
    pub async fn load_status(&self) -> Result<String, String> {
        self.run_cmd("tail -3 /tmp/lms-load.log 2>/dev/null || echo 'idle'")
            .await
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
    pub async fn search_models(
        &self,
        query: &str,
        format_filter: Option<&str>,
    ) -> Result<String, String> {
        let mut cmd = String::from("timeout 15 lms get");
        if !query.is_empty() {
            cmd.push_str(&format!(" {}", Self::sanitize(query)));
        }
        match format_filter {
            Some("gguf") => cmd.push_str(" --gguf"),
            Some("mlx") => cmd.push_str(" --mlx"),
            _ => {}
        }
        cmd.push_str(" 2>&1 || true");
        self.run_cmd(&cmd).await
    }

    /// Search HuggingFace Hub for GGUF models via their API.
    pub async fn search_huggingface(
        &self,
        query: &str,
        sort: &str,
        pipeline_tag: Option<&str>,
    ) -> Result<Vec<crate::models::HfModel>, String> {
        let sort_param = match sort {
            "likes" => "likes",
            "newest" => "lastModified",
            "trending" => "trendingScore",
            _ => "downloads",
        };
        let mut url = format!(
            "https://huggingface.co/api/models?filter=gguf&search={}&sort={}&direction=-1&limit=20&full=true",
            query, sort_param
        );
        if let Some(tag) = pipeline_tag {
            url.push_str(&format!("&pipeline_tag={}", tag));
        }
        let mut req = self.client.get(&url);
        if let Ok(token) = std::env::var("HF_TOKEN") {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let resp = req.send().await.map_err(|e| {
            tracing::error!(query = %query, error = %e, "HF API request failed");
            classify_http_error(&e, &url)
        })?;
        let status = resp.status();
        let models: Vec<crate::models::HfModel> = resp
            .json()
            .await
            .map_err(|e| {
                tracing::error!(query = %query, status = %status, error = %e, "HF API response parse failed");
                format!("HF parse error: {}", e)
            })?;
        tracing::debug!(query = %query, count = models.len(), "HF search returned models");
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
        let log_file = format!("/tmp/lms-dl-{}.log", LmsClient::dl_id(name));
        self.run_cmd(&format!(
            "tail -c 500 {} 2>/dev/null || echo 'No log found'",
            log_file
        ))
        .await
    }

    /// List all active (or recently-dead) downloads by scanning PID + name files.
    /// Returns tuples of (model_name, pid_string, is_running).
    pub async fn list_active_downloads(&self) -> Result<Vec<ActiveDownload>, String> {
        tracing::debug!("Listing active downloads");
        let raw = self
            .run_cmd(
                "for pf in /tmp/lms-dl-*.pid; do \
                    [ -f \"$pf\" ] || continue; \
                    did=$(basename \"$pf\" .pid | sed 's/lms-dl-//'); \
                    pid=$(cat \"$pf\" 2>/dev/null | tr -d '[:space:]'); \
                    name=$(cat \"/tmp/lms-dl-${did}.name\" 2>/dev/null | tr -d '[:space:]'); \
                    [ -z \"$name\" ] && name=\"unknown\"; \
                    if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null 2>&1; then \
                        echo \"${name}|${pid}|1\"; \
                    else \
                        echo \"${name}|${pid}|0\"; \
                    fi; \
                done 2>/dev/null",
            )
            .await?;
        let downloads = raw
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() == 3 {
                    Some(ActiveDownload {
                        name: parts[0].to_string(),
                        pid: parts[1].to_string(),
                        running: parts[2] == "1",
                    })
                } else {
                    None
                }
            })
            .collect();
        Ok(downloads)
    }

    /// Deterministic filename-safe hash for a model name (avoids / vs _ collisions).
    /// Keys on the *normalized* model basename so that the same model downloaded
    /// from different publisher repos (e.g. lmstudio-community/Foo-GGUF and
    /// unsloth/Foo-GGUF) shares one tracking slot — the second download kills
    /// the first instead of running concurrently into a different folder.
    fn dl_id(name: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let key = LmsClient::normalize_dl_key(name);
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Normalize a model identifier to a canonical download-tracking key.
    ///
    /// Strips a leading `https://huggingface.co/` (or `http...`) scheme,
    /// drops the publisher/namespace segment, and removes a trailing `@quant`
    /// suffix. Result examples:
    ///   "https://huggingface.co/lmstudio-community/Qwen3-Coder-Next-GGUF"
    ///     -> "qwen3-coder-next-gguf"
    ///   "https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF@Q4_K_M"
    ///     -> "qwen3-coder-next-gguf"
    ///   "Qwen/Qwen3-Coder-Next-GGUF" -> "qwen3-coder-next-gguf"
    ///   "Qwen3-Coder-Next-GGUF"      -> "qwen3-coder-next-gguf"
    ///
    /// Two differently-prefixed URLs for the same model collapse to the same
    /// key, so `dl_id` (and thus PID/log files + pkill patterns) collide and
    /// the pre-start kill actually stops the prior download.
    fn normalize_dl_key(name: &str) -> String {
        let trimmed = name
            .strip_prefix("https://")
            .or_else(|| name.strip_prefix("http://"))
            .unwrap_or(name);
        // Drop host segment if an HF-style URL was passed (huggingface.co/<org>/<repo>)
        let path = match trimmed.find('/') {
            Some(_) if trimmed.contains("huggingface.co") => {
                trimmed.splitn(3, '/').nth(2).unwrap_or(trimmed)
            }
            _ => trimmed,
        };
        // Take the last path segment as the basename (drops publisher/org).
        let base = path.rsplit('/').next().unwrap_or(path);
        // Drop a trailing @quant selector (e.g. "@Q4_K_M").
        let no_quant = base.split('@').next().unwrap_or(base);
        no_quant.to_lowercase()
    }

    /// Strip any character that is not alphanumeric or one of `/ - _ . : @` from
    /// user-supplied input before interpolating it into a shell command.
    fn sanitize(input: &str) -> String {
        input.replace(
            |c: char| {
                !c.is_alphanumeric()
                    && c != '/'
                    && c != '-'
                    && c != '_'
                    && c != '.'
                    && c != ':'
                    && c != '@'
            },
            "",
        )
    }

    /// Send a chat completion request to the LMS API.
    pub async fn chat_completion(
        &self,
        req: &crate::handlers::ChatRequest,
    ) -> Result<(String, u32), String> {
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

        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(model = %req.model, error = %e, "Chat completion request failed");
                classify_http_error(&e, &url)
            })?;

        let data: serde_json::Value = resp.json().await.map_err(|e| {
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

    /// Execute a shell command either locally (bash, when `LMS_LOCAL=1`) or over
    /// SSH (with ControlMaster multiplexing), stripping ANSI escapes from output.
    ///
    /// Times out after 20 seconds. On non-zero exit with stdout present, returns
    /// the stdout; otherwise classifies the stderr via [`classify_cli_error`].
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
                            .args([
                                "-c",
                                &format!("export PATH=\"$HOME/.lmstudio/bin:$PATH\" && {}", cmd),
                            ])
                            .output()
                    } else {
                        Command::new("ssh")
                            .args([
                                "-o",
                                "ControlMaster=auto",
                                "-o",
                                &format!("ControlPath={}", SSH_MUX_PATH),
                                "-o",
                                "ControlPersist=600",
                                "-o",
                                "ConnectTimeout=5",
                                "-o",
                                "ServerAliveInterval=5",
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
                Err(classify_cli_error(&stderr))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Characterization tests for `normalize_dl_key` — the function that
    /// collapses cross-publisher download URLs to a single tracking key so
    /// the same model from `lmstudio-community/` and `unsloth/` shares one
    /// PID/log slot and the pre-start kill actually stops the prior download.
    #[test]
    fn normalize_hf_url_with_publisher() {
        assert_eq!(
            LmsClient::normalize_dl_key(
                "https://huggingface.co/lmstudio-community/Qwen3-Coder-Next-GGUF"
            ),
            "qwen3-coder-next-gguf"
        );
    }

    #[test]
    fn normalize_hf_url_different_publisher_collapses() {
        // The core fix: two different publishers of the same model must
        // produce the SAME normalized key.
        let a = LmsClient::normalize_dl_key(
            "https://huggingface.co/lmstudio-community/Qwen3-Coder-Next-GGUF",
        );
        let b = LmsClient::normalize_dl_key("https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF");
        assert_eq!(a, b);
        assert_eq!(a, "qwen3-coder-next-gguf");
    }

    #[test]
    fn normalize_hf_url_with_quant_suffix() {
        assert_eq!(
            LmsClient::normalize_dl_key(
                "https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF@Q4_K_M"
            ),
            "qwen3-coder-next-gguf"
        );
    }

    #[test]
    fn normalize_plain_org_repo_form() {
        assert_eq!(
            LmsClient::normalize_dl_key("Qwen/Qwen3-Coder-Next-GGUF"),
            "qwen3-coder-next-gguf"
        );
    }

    #[test]
    fn normalize_bare_name() {
        assert_eq!(
            LmsClient::normalize_dl_key("Qwen3-Coder-Next-GGUF"),
            "qwen3-coder-next-gguf"
        );
    }

    #[test]
    fn normalize_http_scheme() {
        assert_eq!(
            LmsClient::normalize_dl_key("http://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF"),
            "qwen3-coder-next-gguf"
        );
    }

    /// `dl_id` must collide for cross-publisher URLs of the same model.
    /// This is the property that makes the pre-start PID kill work.
    #[test]
    fn dl_id_collides_across_publishers() {
        let id_a =
            LmsClient::dl_id("https://huggingface.co/lmstudio-community/Qwen3-Coder-Next-GGUF");
        let id_b = LmsClient::dl_id("https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF");
        assert_eq!(id_a, id_b, "cross-publisher dl_id must collide");
    }

    #[test]
    fn dl_id_collides_with_and_without_quant() {
        let id_a = LmsClient::dl_id("https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF");
        let id_b = LmsClient::dl_id("https://huggingface.co/unsloth/Qwen3-Coder-Next-GGUF@Q4_K_M");
        assert_eq!(id_a, id_b, "quant suffix must not change dl_id");
    }

    #[test]
    fn dl_id_differs_for_different_models() {
        let id_a = LmsClient::dl_id("Qwen/Qwen3-Coder-Next-GGUF");
        let id_b = LmsClient::dl_id("Qwen/gpt-oss-120b-GGUF");
        assert_ne!(id_a, id_b, "different models must have different dl_id");
    }
}
