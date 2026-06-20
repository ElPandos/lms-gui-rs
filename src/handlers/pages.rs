use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use axum::Json;

use crate::models::*;
use crate::stats::TrafficStats;
use crate::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    models: Vec<LocalModel>,
    loaded_models: Vec<LoadedModel>,
    runtimes: Vec<RuntimeEntry>,
    host: HostInfo,
    gpu_mem: String,
}

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
}

#[derive(Template)]
#[template(path = "runtime.html")]
struct RuntimeTemplate {
    runtimes: Vec<RuntimeEntry>,
    update_status: String,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    stats: TrafficStats,
}

/// Render the main dashboard with loaded models, runtimes, and host info.
pub async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_request();
    drop(s);

    let (loaded, rt, host_raw, gpu_mem, local_output) = tokio::join!(
        state.lms.list_loaded_models(),
        state.lms.runtime_status(),
        state.lms.host_info(),
        state.lms.gpu_memory(),
        state.lms.list_local_models(),
    );

    let local_models = parse_local_models(&local_output.unwrap_or_default());

    let mut models: Vec<LocalModel> = local_models.into_iter()
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

    let tmpl = DashboardTemplate { models, loaded_models, runtimes, host, gpu_mem };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

/// List local models and optionally search for new ones.
pub async fn list_models(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_api_call("list_models");
    drop(s);

    let local_output = state.lms.list_local_models().await.unwrap_or_default();

    let has_query_param = params.q.is_some();
    let query = params.q.unwrap_or_default();
    let search_source = params.source.unwrap_or_else(|| "lms".to_string());
    let search_sort = params.sort.unwrap_or_else(|| "downloads".to_string());
    let local_models: Vec<LocalModel> = parse_local_models(&local_output)
        .into_iter()
        .filter(|m| m.model_type == "LLM")
        .collect();

    // Get local model names for filtering
    let local_names: std::collections::HashSet<&str> = local_models.iter()
        .map(|m| m.name.as_str())
        .collect();

    let (search_results, hf_results) = if !query.is_empty() || has_query_param {
        if search_source == "hf" {
            let hf = state.lms.search_huggingface(&query, &search_sort).await.unwrap_or_default()
                .into_iter()
                .filter(|m| m.model_id.contains("GGUF") || m.model_id.contains("gguf"))
                .collect();
            (vec![], hf)
        } else {
            let raw = state.lms.search_models(&query).await.unwrap_or_default();
            let results = parse_search_results(&raw)
                .into_iter()
                .filter(|r| !local_names.contains(r.name.as_str()))
                .collect();
            (results, vec![])
        }
    } else {
        (vec![], vec![])
    };

    let disk_summary = local_output.lines()
        .find(|l| l.contains("models") && l.contains("GB"))
        .unwrap_or("")
        .to_string();

    let disk_total = state.lms.host_info().await
        .map(|h| parse_host_info(&h).disk)
        .unwrap_or_default();

    let tmpl = ModelsTemplate { local_models, search_results, hf_results, query, disk_summary, disk_total, search_source, search_sort };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

/// Start a model download in the background.
pub async fn download_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    let mut s = state.stats.write().await;
    s.record_download(&form.model_name);
    drop(s);

    match state.lms.download_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => {
            let mut s = state.stats.write().await;
            s.record_error(&e);
            Json(CommandResult { success: false, message: e })
        }
    }
}

/// Load a model into GPU memory.
pub async fn load_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("load:{}", form.model_name));
    drop(s);

    match state.lms.load_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Unload a model from memory.
pub async fn unload_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("unload:{}", form.model_name));
    drop(s);

    match state.lms.unload_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Delete a model from disk.
pub async fn delete_model(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("delete:{}", form.model_name));
    drop(s);

    match state.lms.delete_model(&form.model_name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Render runtime status and update check page.
pub async fn runtime_status(State(state): State<AppState>) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_api_call("runtime_status");
    drop(s);

    let rt = state.lms.runtime_status().await;
    let (runtimes, update_status) = match rt {
        Ok(r) => (parse_runtimes(&r.runtimes), r.update_status),
        Err(e) => (vec![], e),
    };
    let tmpl = RuntimeTemplate { runtimes, update_status };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

/// Render the traffic statistics page.
pub async fn traffic_stats(State(state): State<AppState>) -> impl IntoResponse {
    let mut s = state.stats.write().await;
    s.record_request();
    drop(s);
    let stats = state.stats.read().await.clone();
    let tmpl = StatsTemplate { stats };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

#[derive(Template)]
#[template(path = "api.html")]
struct ApiTemplate {}

/// Render the API documentation page.
pub async fn api_docs() -> impl IntoResponse {
    let tmpl = ApiTemplate {};
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

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
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

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

    let log_type = params.q.as_deref().unwrap_or("lms");
    let selected_dl = params.dl.unwrap_or_default();

    let (output, download_logs) = match log_type {
        "app" => (state.lms.app_log().await.unwrap_or_else(|e| e), vec![]),
        "downloads" => {
            let logs_raw = state.lms.download_logs().await.unwrap_or_default();
            let logs: Vec<String> = logs_raw.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect();
            let content = if !selected_dl.is_empty() {
                state.lms.download_log_content(&selected_dl).await.unwrap_or_else(|e| e)
            } else if let Some(first) = logs.first() {
                state.lms.download_log_content(first).await.unwrap_or_else(|e| e)
            } else {
                "No download logs".to_string()
            };
            (content, logs)
        }
        _ => (state.lms.recent_logs().await.unwrap_or_else(|e| e), vec![]),
    };

    let selected_dl_idx = download_logs.iter().position(|l| l == &selected_dl).unwrap_or(0);

    let tmpl = LogsTemplate {
        output,
        active_tab: log_type.to_string(),
        download_logs,
        selected_dl_idx,
    };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}

/// Switch the active inference runtime.
pub async fn select_runtime(
    State(state): State<AppState>,
    Json(form): Json<DownloadRequest>,
) -> Json<CommandResult> {
    let mut s = state.stats.write().await;
    s.record_api_call(&format!("select_runtime:{}", form.model_name));
    drop(s);

    match state.lms.select_runtime(&form.model_name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Get download progress for a model.
pub async fn download_status(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Json<CommandResult> {
    let name = params.q.unwrap_or_default();
    if name.is_empty() {
        return Json(CommandResult { success: false, message: "No model name".to_string() });
    }
    match state.lms.download_status(&name).await {
        Ok(msg) => Json(CommandResult { success: true, message: msg }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Check if a model has finished loading.
pub async fn load_status(State(state): State<AppState>) -> Json<CommandResult> {
    // Check if a model actually loaded (lms ps) regardless of log errors
    let ps = state.lms.list_loaded_models().await.unwrap_or_default();
    if !ps.is_empty() && !ps.contains("No models") {
        return Json(CommandResult { success: true, message: "Model loaded".to_string() });
    }
    match state.lms.load_status().await {
        Ok(msg) => {
            let failed = msg.contains("CAUSE") || msg.contains("error");
            Json(CommandResult { success: !failed, message: msg })
        }
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

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
    let loaded_set: std::collections::HashSet<&str> = loaded_models.iter().map(|s| s.as_str()).collect();
    let all_models: Vec<(String, bool)> = local.iter()
        .filter(|m| m.model_type == "LLM")
        .map(|m| (m.name.clone(), loaded_set.contains(m.name.as_str())))
        .collect();
    let tmpl = ChatTemplate { all_models };
    Html(tmpl.render().unwrap_or_else(|e| format!("Template error: {}", e)))
}
