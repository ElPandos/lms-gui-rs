use axum::extract::State;
use axum::Json;

use crate::models::*;
use crate::stats::TrafficStats;
use crate::AppState;

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
