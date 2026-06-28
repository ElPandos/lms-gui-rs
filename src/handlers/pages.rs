//! HTML page handlers — render Askama templates for dashboard, models, runtime,
//! logs, stats, chat, and changelog views.

use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use axum::Json;

use crate::models::*;
use crate::stats::TrafficStats;
use crate::AppState;

/// Render an Askama template to HTML, returning an error page on failure.
fn render_or_error<T: Template>(tmpl: T) -> Html<String> {
    tmpl.render()
        .map(Html)
        .unwrap_or_else(|e| Html(format!("Template error: {}", e)))
}

/// Askama template for the main dashboard view.
#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    models: Vec<LocalModel>,
    loaded_models: Vec<LoadedModel>,
    runtimes: Vec<RuntimeEntry>,
    host: HostInfo,
    gpu_mem: String,
}

/// Askama template for the models listing + search view.
#[derive(Template)]
#[template(path = "models.html")]
struct ModelsTemplate {
    local_models: Vec<LocalModel>,
    search_results: Vec<SearchResult>,
    hf_results: Vec<HfModel>,
    query: String,
    disk_summary: String,
    disk_total: String,
    search_source: String,
    search_sort: String,
    search_format: String,
    search_pipeline: String,
    gpu_vram_gb: u64,
}

/// Askama template for the runtime status view.
#[derive(Template)]
#[template(path = "runtime.html")]
struct RuntimeTemplate {
    runtimes: Vec<RuntimeEntry>,
    update_status: String,
    cuda_version: String,
    nvidia_driver: String,
    gpu: String,
}

/// Askama template for the traffic statistics view.
#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    stats: TrafficStats,
    uptime: String,
    rate_per_min: u64,
}

/// Render the main dashboard with loaded models, runtimes, and host info.
pub async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    tracing::debug!("Rendering dashboard");
    let mut s = state.stats.write().await;
    s.record_request();
    drop(s);

    let (loaded, rt, host_raw, gpu_mem, local_v0) = tokio::join!(
        state.lms.list_loaded_models(),
        state.lms.runtime_status(),
        state.lms.host_info(),
        state.lms.gpu_memory(),
        state.lms.list_local_models_v0(),
    );

    let local_models = local_v0.unwrap_or_default();

    let mut models: Vec<LocalModel> = local_models
        .into_iter()
        .filter(|m| m.model_type == "LLM")
        .collect();
    models.sort_by_key(|a| a.name.to_lowercase());
    let loaded_models = parse_loaded_models(&loaded.unwrap_or_default());
    let runtimes = match rt {
        Ok(r) => parse_runtimes(&r.runtimes),
        Err(_) => vec![],
    };
    let host = parse_host_info(&host_raw.unwrap_or_default());
    let gpu_mem = gpu_mem.unwrap_or_default();

    let tmpl = DashboardTemplate {
        models,
        loaded_models,
        runtimes,
        host,
        gpu_mem,
    };
    render_or_error(tmpl)
}

/// List local models and optionally search for new ones.
pub async fn list_models(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_api_call("list_models");
    drop(s);

    let has_query_param = params.q.is_some();
    let query = params.q.unwrap_or_default();
    let search_source = params.source.unwrap_or_else(|| "lms".to_string());
    let search_sort = params.sort.unwrap_or_else(|| "downloads".to_string());
    let search_format = params.format.clone();
    let search_pipeline = params.pipeline_tag.clone();

    // Fire all independent I/O concurrently (previously these were 5 serial
    // SSH/HTTP round-trips). The search call does not depend on local_models
    // to execute — only the post-filter uses local_names.
    let (local_models_result, search_raw, disk_summary_raw, host_raw, gpu_mem_raw) = tokio::join!(
        state.lms.list_local_models_v0(),
        async {
            if !query.is_empty() || has_query_param {
                if search_source == "hf" {
                    state
                        .lms
                        .search_huggingface(&query, &search_sort, search_pipeline.as_deref())
                        .await
                        .map(|v| {
                            v.into_iter()
                                .filter(|m| {
                                    m.model_id.contains("GGUF") || m.model_id.contains("gguf")
                                })
                                .collect::<Vec<_>>()
                        })
                        .map(SearchOutcome::Hf)
                        .unwrap_or(SearchOutcome::HfEmpty)
                } else {
                    state
                        .lms
                        .search_models(&query, search_format.as_deref())
                        .await
                        .map(SearchOutcome::Lms)
                        .unwrap_or(SearchOutcome::LmsEmpty)
                }
            } else {
                SearchOutcome::Empty
            }
        },
        state.lms.list_local_models(),
        state.lms.host_info(),
        state.lms.gpu_memory(),
    );

    let local_models: Vec<LocalModel> = local_models_result
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.model_type == "LLM")
        .collect();

    // Get local model names for filtering search results
    let local_names: std::collections::HashSet<&str> =
        local_models.iter().map(|m| m.name.as_str()).collect();

    let (search_results, hf_results) = match search_raw {
        SearchOutcome::Hf(hf) => (vec![], hf),
        SearchOutcome::HfEmpty => (vec![], vec![]),
        SearchOutcome::Lms(raw) => (
            parse_search_results(&raw)
                .into_iter()
                .filter(|r| !local_names.contains(r.name.as_str()))
                .collect(),
            vec![],
        ),
        SearchOutcome::LmsEmpty => (vec![], vec![]),
        SearchOutcome::Empty => (vec![], vec![]),
    };

    // Disk summary from lms ls (lightweight — just the summary line)
    let disk_summary = disk_summary_raw
        .unwrap_or_default()
        .lines()
        .find(|l| l.contains("models") && l.contains("GB"))
        .unwrap_or("")
        .to_string();

    let disk_total = host_raw
        .as_deref()
        .map(parse_host_info)
        .map(|h| h.disk)
        .unwrap_or_default();

    // Fetch total GPU VRAM for quantization recommendations
    let gpu_vram_gb = gpu_mem_raw
        .ok()
        .and_then(|g| {
            // Parse "12G / 32G" → extract total (second number)
            g.split('/')
                .nth(1)
                .and_then(|s| s.trim().trim_end_matches('G').parse::<u64>().ok())
        })
        .unwrap_or(0);

    let tmpl = ModelsTemplate {
        local_models,
        search_results,
        hf_results,
        query,
        disk_summary,
        disk_total,
        search_source,
        search_sort,
        search_format: search_format.unwrap_or_default(),
        search_pipeline: search_pipeline.unwrap_or_default(),
        gpu_vram_gb,
    };
    render_or_error(tmpl)
}

/// Internal enum to unify the two search branches for concurrent dispatch.
enum SearchOutcome {
    Hf(Vec<HfModel>),
    HfEmpty,
    Lms(String),
    LmsEmpty,
    Empty,
}

/// Start a model download in the background.
pub async fn download_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::info!(model = %form.model_name, "Download model request");
    let mut s = state.stats.write().await;
    s.record_download(&form.model_name);
    drop(s);

    match state.lms.download_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            let mut s = state.stats.write().await;
            s.record_error(&e);
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Load a model into GPU memory.
pub async fn load_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::info!(model = %form.model_name, parallel = ?form.parallel, "Load model request");
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("load:{}", form.model_name));
    drop(s);

    let setting_ctx: Option<u64> = state
        .db
        .get_setting("models_settings")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("default-context-length").and_then(|n| {
                n.as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| n.as_u64())
            })
        });
    // Resolve parallel: request body override > persisted setting > None (LMS default = 4)
    let setting_parallel: Option<u32> = state
        .db
        .get_setting("models_settings")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("default-parallel").and_then(|n| {
                n.as_str()
                    .and_then(|s| s.parse::<u32>().ok())
                    .or_else(|| n.as_u64().map(|x| x as u32))
            })
        });
    let parallel = form.parallel.or(setting_parallel);
    tracing::debug!(model = %form.model_name, setting_ctx = ?setting_ctx, parallel = ?parallel, "Computing load context length and parallel slots");
    let ctx_len: Option<u64> = match setting_ctx {
        Some(sc) => match state.lms.fetch_max_context(&form.model_name).await {
            Ok(Some(max)) => Some(sc.min(max)),
            Ok(None) => Some(sc),
            Err(e) => {
                tracing::warn!(model = %form.model_name, error = %e, "Failed to fetch max context; using setting value");
                Some(sc)
            }
        },
        None => None,
    };

    match state
        .lms
        .load_model(&form.model_name, ctx_len, parallel)
        .await
    {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            tracing::error!(model = %form.model_name, error = %e, "Failed to load model");
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Unload a model from memory.
pub async fn unload_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::info!(model = %form.model_name, "Unload model request");
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("unload:{}", form.model_name));
    drop(s);

    match state.lms.unload_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            tracing::error!(model = %form.model_name, error = %e, "Failed to unload model");
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Delete a model from disk.
pub async fn delete_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::warn!(model = %form.model_name, "Delete model request");
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("delete:{}", form.model_name));
    drop(s);

    match state.lms.delete_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            tracing::error!(model = %form.model_name, error = %e, "Failed to delete model");
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Render runtime status and update check page.
pub async fn runtime_status(State(state): State<AppState>) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_api_call("runtime_status");
    drop(s);

    let (rt, host_raw) = tokio::join!(state.lms.runtime_status(), state.lms.host_info(),);
    let (runtimes, update_status) = match rt {
        Ok(r) => (parse_runtimes(&r.runtimes), r.update_status),
        Err(e) => (vec![], e),
    };
    let host = parse_host_info(&host_raw.unwrap_or_default());
    let tmpl = RuntimeTemplate {
        runtimes,
        update_status,
        cuda_version: host.cuda_version,
        nvidia_driver: host.nvidia_driver,
        gpu: host.gpu,
    };
    render_or_error(tmpl)
}

/// Render the traffic statistics page.
pub async fn traffic_stats(State(state): State<AppState>) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_request();
    drop(s);
    let stats = state.stats.read().await.clone();
    let secs = stats.uptime_secs();
    let uptime = format!("{}h {}m", secs / 3600, (secs % 3600) / 60);
    let rate_per_min = if secs > 0 {
        stats.total_requests * 60 / secs as u64
    } else {
        0
    };
    let tmpl = StatsTemplate {
        stats,
        uptime,
        rate_per_min,
    };
    render_or_error(tmpl)
}

/// Askama template for the API documentation view.
#[derive(Template)]
#[template(path = "api.html")]
struct ApiTemplate {}

/// Render the API documentation page.
pub async fn api_docs() -> impl IntoResponse {
    let tmpl = ApiTemplate {};
    render_or_error(tmpl)
}

/// Askama template for the changelog view (build metadata from build.rs).
#[derive(Template)]
#[template(path = "changelog.html")]
struct ChangelogTemplate {
    git_hash: &'static str,
    git_log: &'static str,
    build_time: &'static str,
    version: &'static str,
}

/// Render the changelog page with build metadata.
pub async fn changelog() -> impl IntoResponse {
    let tmpl = ChangelogTemplate {
        git_hash: env!("GIT_HASH"),
        git_log: env!("GIT_LOG"),
        build_time: env!("BUILD_TIME"),
        version: env!("CARGO_PKG_VERSION"),
    };
    render_or_error(tmpl)
}

/// Askama template for the logs view (app, LMS, or download logs).
#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    output: String,
    active_tab: String,
    download_logs: Vec<String>,
    selected_dl_idx: usize,
}

/// Render the logs page (LMS, app, or download logs).
pub async fn server_logs(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_api_call("server_logs");
    drop(s);

    let log_type = params.q.as_deref().unwrap_or("app");
    let selected_dl = params.dl.unwrap_or_default();

    let (output, download_logs) = match log_type {
        "downloads" => {
            let logs_raw = state.lms.download_logs().await.unwrap_or_default();
            let logs: Vec<String> = logs_raw
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect();
            let content = if !selected_dl.is_empty() {
                state
                    .lms
                    .download_log_content(&selected_dl)
                    .await
                    .unwrap_or_else(|e| e)
            } else if let Some(first) = logs.first() {
                state
                    .lms
                    .download_log_content(first)
                    .await
                    .unwrap_or_else(|e| e)
            } else {
                "No download logs".to_string()
            };
            (content, logs)
        }
        _ => (state.lms.app_log().await.unwrap_or_else(|e| e), vec![]),
    };

    let selected_dl_idx = download_logs
        .iter()
        .position(|l| l == &selected_dl)
        .unwrap_or(0);

    let tmpl = LogsTemplate {
        output,
        active_tab: log_type.to_string(),
        download_logs,
        selected_dl_idx,
    };
    render_or_error(tmpl)
}

/// Switch the active inference runtime.
pub async fn select_runtime(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::info!(runtime = %form.model_name, "Select runtime request");
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("select_runtime:{}", form.model_name));
    drop(s);

    match state.lms.select_runtime(&form.model_name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            tracing::error!(runtime = %form.model_name, error = %e, "Failed to select runtime");
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Get download progress for a model.
pub async fn download_status(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Json<CommandResult> {
    let name = params.q.unwrap_or_default();
    if name.is_empty() {
        return Json(CommandResult {
            success: false,
            message: "No model name".to_string(),
        });
    }
    // If clear=1, cancel the download and clean up (kill PID + delete log)
    if params.clear.as_deref() == Some("1") {
        tracing::info!(model = %name, "Clearing download (cancel + cleanup)");
        match state.lms.cancel_download(&name).await {
            Ok(msg) => {
                return Json(CommandResult {
                    success: true,
                    message: msg,
                })
            }
            Err(e) => {
                return Json(CommandResult {
                    success: false,
                    message: e,
                })
            }
        }
    }
    match state.lms.download_status(&name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => Json(CommandResult {
            success: false,
            message: e,
        }),
    }
}

/// Cancel an in-progress download by killing the tracked PID.
pub async fn cancel_download(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    tracing::info!(model = %form.model_name, "Cancel download request");
    match state.lms.cancel_download(&form.model_name).await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => {
            tracing::error!(model = %form.model_name, error = %e, "Failed to cancel download");
            Json(CommandResult {
                success: false,
                message: e,
            })
        }
    }
}

/// Check if a model has finished loading.
pub async fn load_status(State(state): State<AppState>) -> Json<CommandResult> {
    // Check if a model actually loaded (lms ps) regardless of log errors
    let ps = state.lms.list_loaded_models().await.unwrap_or_default();
    if !ps.is_empty() && !ps.contains("No models") {
        return Json(CommandResult {
            success: true,
            message: "Model loaded".to_string(),
        });
    }
    match state.lms.load_status().await {
        Ok(msg) => {
            let failed = msg.contains("CAUSE") || msg.contains("error");
            Json(CommandResult {
                success: !failed,
                message: msg,
            })
        }
        Err(e) => Json(CommandResult {
            success: false,
            message: e,
        }),
    }
}

/// Askama template for the chat interface view.
#[derive(Template)]
#[template(path = "chat.html")]
struct ChatTemplate {
    all_models: Vec<(String, bool)>, // (name, is_loaded)
}

/// Render the chat interface page.
pub async fn chat_page(State(state): State<AppState>) -> impl IntoResponse {
    let (ps, local_output) = tokio::join!(
        state.lms.list_loaded_models(),
        state.lms.list_local_models(),
    );
    let loaded = crate::models::parse_loaded_models(&ps.unwrap_or_default());
    let loaded_models: Vec<String> = loaded.iter().map(|m| m.identifier.clone()).collect();
    let local = crate::models::parse_local_models(&local_output.unwrap_or_default());
    let loaded_set: std::collections::HashSet<&str> =
        loaded_models.iter().map(|s| s.as_str()).collect();
    let all_models: Vec<(String, bool)> = local
        .iter()
        .filter(|m| m.model_type == "LLM")
        .map(|m| (m.name.clone(), loaded_set.contains(m.name.as_str())))
        .collect();
    let tmpl = ChatTemplate { all_models };
    render_or_error(tmpl)
}

/// Apply runtime updates.
pub async fn update_runtime(State(state): State<AppState>) -> Json<CommandResult> {
    tracing::info!("Apply runtime update request");
    match state.lms.update_runtime().await {
        Ok(msg) => Json(CommandResult {
            success: true,
            message: msg,
        }),
        Err(e) => Json(CommandResult {
            success: false,
            message: e,
        }),
    }
}
