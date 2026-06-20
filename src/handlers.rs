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

// JSON API endpoints
/// JSON API: list all models from LMS.
pub async fn api_models(State(state): State<AppState>) -> Json<Vec<Model>> {
    let mut s = state.stats.write().await;
    s.record_api_call("api_models");
    drop(s);
    Json(state.lms.list_models().await.unwrap_or_default())
}

/// JSON API: currently loaded models.
pub async fn api_loaded_models(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut s = state.stats.write().await;
    s.record_api_call("api_loaded_models");
    drop(s);
    let output = state.lms.list_loaded_models().await.unwrap_or_else(|e| e);
    Json(serde_json::json!({ "output": output }))
}

/// JSON API: traffic statistics.
pub async fn api_stats(State(state): State<AppState>) -> Json<TrafficStats> {
    let stats = state.stats.read().await.clone();
    Json(stats)
}

/// JSON API: whether running in local mode.
pub async fn api_mode(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "local": state.lms.local_mode }))
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

// === Chat ===

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

/// Request body for chat completions.
#[derive(Debug, serde::Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub message: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub frequency_penalty: Option<f64>,
    #[serde(default)]
    pub presence_penalty: Option<f64>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

fn default_temperature() -> f64 { 0.7 }
fn default_max_tokens() -> u32 { 1024 }

/// Response from a chat completion call.
#[derive(Debug, serde::Serialize)]
pub struct ChatResponse {
    pub success: bool,
    pub content: String,
    pub duration_ms: u64,
    pub tokens_used: u32,
}

/// Send a chat completion request to the loaded model.
pub async fn chat_send(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let start = std::time::Instant::now();
    match state.lms.chat_completion(&req).await {
        Ok((content, tokens)) => Json(ChatResponse {
            success: true,
            content,
            duration_ms: start.elapsed().as_millis() as u64,
            tokens_used: tokens,
        }),
        Err(e) => Json(ChatResponse {
            success: false,
            content: e,
            duration_ms: start.elapsed().as_millis() as u64,
            tokens_used: 0,
        }),
    }
}

/// Request body for the latency speed test.
#[derive(Debug, serde::Deserialize)]
pub struct SpeedTestRequest {
    pub model: String,
    pub num_calls: u32,
    pub sigma: u32, // 0=no filter, 1/2/3 = sigma levels
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

/// Aggregated speed test output with per-call data and statistics.
#[derive(Debug, serde::Serialize)]
pub struct SpeedTestResult {
    pub success: bool,
    pub results: Vec<SpeedTestCall>,
    pub stats: SpeedTestStats,
}

/// A single call in a speed test run.
#[derive(Debug, serde::Serialize, Clone)]
pub struct SpeedTestCall {
    pub index: u32,
    pub duration_ms: u64,
    pub tokens: u32,
    pub prompt: String,
    pub response: String,
    pub outlier: bool,
}

/// Summary statistics for a speed test run.
#[derive(Debug, serde::Serialize)]
pub struct SpeedTestStats {
    pub mean_ms: f64,
    pub median_ms: f64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub std_dev_ms: f64,
    pub filtered_mean_ms: f64,
    pub total_calls: u32,
    pub filtered_calls: u32,
    pub sigma: u32,
}

const SPEED_TEST_PROMPTS: &[&str] = &[
    "What is 2+2? Reply with only the number.",
    "Name the capital of France. Reply with only the city name.",
    "What color is the sky on a clear day? Reply with one word.",
    "How many legs does a cat have? Reply with only the number.",
    "What is the chemical symbol for water? Reply with only the formula.",
    "Name the largest planet in our solar system. Reply with only the name.",
    "What is the boiling point of water in Celsius? Reply with only the number.",
    "How many continents are there? Reply with only the number.",
    "What is the square root of 64? Reply with only the number.",
    "Name the first element on the periodic table. Reply with only the name.",
    "What year did World War 2 end? Reply with only the number.",
    "How many days are in a week? Reply with only the number.",
    "What is the speed of light in km/s? Reply with only the number.",
    "Name the author of Romeo and Juliet. Reply with only the name.",
    "What is the freezing point of water in Fahrenheit? Reply with only the number.",
    "How many letters in the English alphabet? Reply with only the number.",
    "What is 10 multiplied by 10? Reply with only the number.",
    "Name the smallest prime number. Reply with only the number.",
    "What gas do humans breathe in? Reply with only the name.",
    "How many hours in a day? Reply with only the number.",
];

/// Run a latency speed test against a model with multiple calls.
pub async fn chat_speedtest(
    State(state): State<AppState>,
    Json(req): Json<SpeedTestRequest>,
) -> Json<SpeedTestResult> {
    let mut results: Vec<SpeedTestCall> = Vec::new();

    // Warmup call (not counted)
    let warmup_req = ChatRequest {
        model: req.model.clone(),
        message: "Say hello. Reply with one word.".to_string(),
        temperature: 0.0,
        max_tokens: req.max_tokens,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        system_prompt: Some("You are a precise assistant. Always give the shortest possible answer.".to_string()),
    };
    let _ = state.lms.chat_completion(&warmup_req).await;

    // Actual test calls
    for i in 0..req.num_calls {
        let prompt = SPEED_TEST_PROMPTS[i as usize % SPEED_TEST_PROMPTS.len()];
        let chat_req = ChatRequest {
            model: req.model.clone(),
            message: prompt.to_string(),
            temperature: 0.0,
            max_tokens: req.max_tokens,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            system_prompt: Some("You are a precise assistant. Always give the shortest possible answer.".to_string()),
        };

        let start = std::time::Instant::now();
        let (response, tokens) = state.lms.chat_completion(&chat_req).await.unwrap_or_default();
        let duration = start.elapsed().as_millis() as u64;

        results.push(SpeedTestCall {
            index: i + 1,
            duration_ms: duration,
            tokens,
            prompt: prompt.to_string(),
            response,
            outlier: false,
        });
    }

    // Calculate stats
    let durations: Vec<f64> = results.iter().map(|r| r.duration_ms as f64).collect();
    let n = durations.len() as f64;
    let mean = durations.iter().sum::<f64>() / n;
    let variance = durations.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();

    // Mark outliers based on sigma
    if req.sigma > 0 {
        let threshold = std_dev * req.sigma as f64;
        for r in &mut results {
            if (r.duration_ms as f64 - mean).abs() > threshold {
                r.outlier = true;
            }
        }
    }

    let filtered: Vec<f64> = results.iter()
        .filter(|r| !r.outlier)
        .map(|r| r.duration_ms as f64)
        .collect();
    let filtered_mean = if filtered.is_empty() { mean } else { filtered.iter().sum::<f64>() / filtered.len() as f64 };

    let mut sorted = durations.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if sorted.len() % 2 == 0 {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };

    let stats = SpeedTestStats {
        mean_ms: mean,
        median_ms: median,
        min_ms: *durations.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&0.0) as u64,
        max_ms: *durations.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&0.0) as u64,
        std_dev_ms: std_dev,
        filtered_mean_ms: filtered_mean,
        total_calls: req.num_calls,
        filtered_calls: filtered.len() as u32,
        sigma: req.sigma,
    };

    Json(SpeedTestResult { success: true, results, stats })
}

// === Database API Handlers ===

/// JSON API: get all persisted settings.
pub async fn api_get_settings(State(state): State<AppState>) -> Json<serde_json::Value> {
    let settings = state.db.get_all_settings();
    Json(serde_json::json!(settings))
}

/// Request body for saving a setting.
#[derive(Debug, serde::Deserialize)]
pub struct SettingRequest {
    pub key: String,
    pub value: String,
}

/// JSON API: save a key-value setting.
pub async fn api_set_setting(State(state): State<AppState>, Json(req): Json<SettingRequest>) -> Json<CommandResult> {
    match state.db.set_setting(&req.key, &req.value) {
        Ok(()) => Json(CommandResult { success: true, message: "Saved".to_string() }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// JSON API: retrieve chat message history.
pub async fn api_chat_history(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!(state.db.get_chat_history(200)))
}

/// Request body for persisting a chat message.
#[derive(Debug, serde::Deserialize)]
pub struct ChatSaveRequest {
    pub role: String,
    pub model: String,
    pub content: String,
    pub settings_json: Option<String>,
    pub response_json: Option<String>,
    pub duration_ms: Option<u64>,
    pub tokens: Option<u32>,
}

/// JSON API: persist a chat message.
pub async fn api_chat_save(State(state): State<AppState>, Json(req): Json<ChatSaveRequest>) -> Json<CommandResult> {
    match state.db.save_chat_message(&req.role, &req.model, &req.content, req.settings_json.as_deref(), req.response_json.as_deref(), req.duration_ms, req.tokens) {
        Ok(_) => Json(CommandResult { success: true, message: "Saved".to_string() }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// JSON API: clear all chat history.
pub async fn api_chat_clear(State(state): State<AppState>) -> Json<CommandResult> {
    match state.db.clear_chat_history() {
        Ok(()) => Json(CommandResult { success: true, message: "Cleared".to_string() }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// Request body for saving a test result.
#[derive(Debug, serde::Deserialize)]
pub struct TestSaveRequest {
    pub test_type: String,
    pub model: String,
    pub num_calls: u32,
    pub max_tokens: u32,
    pub sigma: u32,
    pub results_json: String,
    pub stats_json: String,
}

/// JSON API: save a speed test result.
pub async fn api_test_save(State(state): State<AppState>, Json(req): Json<TestSaveRequest>) -> Json<CommandResult> {
    match state.db.save_test_result(&req.test_type, &req.model, req.num_calls, req.max_tokens, req.sigma, &req.results_json, &req.stats_json) {
        Ok(_) => Json(CommandResult { success: true, message: "Saved".to_string() }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}

/// JSON API: retrieve past speed test results.
pub async fn api_test_history(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!(state.db.get_test_results(50)))
}

/// JSON API: export all database data.
pub async fn api_export(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.db.export_all())
}

/// JSON API: import data from a previous export.
pub async fn api_import(State(state): State<AppState>, Json(data): Json<serde_json::Value>) -> Json<CommandResult> {
    match state.db.import_all(&data) {
        Ok(()) => Json(CommandResult { success: true, message: "Imported".to_string() }),
        Err(e) => Json(CommandResult { success: false, message: e }),
    }
}
