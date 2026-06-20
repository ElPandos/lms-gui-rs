//! LMS GUI — Web dashboard for managing LM Studio models.
//!
//! Provides a web interface at `http://0.0.0.0:3000` for:
//! - Viewing and managing loaded models
//! - Searching and downloading models from LM Studio Hub
//! - Monitoring runtime status and host hardware
//! - Tracking traffic statistics

mod handlers;
mod lms_client;
mod models;
mod stats;
mod db;

use axum::{routing::get, routing::post, Router};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing_subscriber::fmt::writer::MakeWriterExt;

/// Shared application state passed to all request handlers.
#[derive(Clone)]
pub struct AppState {
    pub lms: lms_client::LmsClient,
    pub stats: Arc<RwLock<stats::TrafficStats>>,
    pub db: Arc<db::Database>,
}

#[tokio::main]
async fn main() {
    // Set up logging: stdout + file (lms-gui-rs.log)
    let file_appender = tracing_appender::rolling::daily(".", "lms-gui-rs.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .with_writer(non_blocking.and(std::io::stdout))
        .with_target(true)
        .with_thread_ids(false)
        .init();

    let local_mode = std::env::var("LMS_LOCAL").unwrap_or_default() == "1";
    let base_url = if local_mode {
        "http://localhost:8010".to_string()
    } else {
        format!("http://{}:8010", std::env::var("ENV_IP_JUMP_155_HOST").expect("ENV_IP_JUMP_155_HOST must be set"))
    };

    tracing::info!(mode = if local_mode { "local" } else { "remote" }, base_url = %base_url, "Starting LMS GUI");

    let state = AppState {
        lms: lms_client::LmsClient::new(base_url),
        stats: Arc::new(RwLock::new(stats::TrafficStats::default())),
        db: Arc::new(db::Database::new("lms-gui.db").expect("Failed to open database")),
    };

    tracing::info!("Database initialized successfully");

    let app = Router::new()
        .route("/", get(handlers::dashboard))
        .route("/models", get(handlers::list_models))
        .route("/models/download", post(handlers::download_model))
        .route("/models/download/status", get(handlers::download_status))
        .route("/models/load", post(handlers::load_model))
        .route("/models/load/status", get(handlers::load_status))
        .route("/models/unload", post(handlers::unload_model))
        .route("/models/delete", post(handlers::delete_model))
        .route("/runtime/status", get(handlers::runtime_status))
        .route("/runtime/select", post(handlers::select_runtime))
        .route("/runtime/update", post(handlers::update_runtime))
        .route("/chat", get(handlers::chat_page))
        .route("/chat/send", post(handlers::chat_send))
        .route("/chat/speedtest", post(handlers::chat_speedtest))
        .route("/logs", get(handlers::server_logs))
        .route("/stats", get(handlers::traffic_stats))
        .route("/api", get(handlers::api_docs))
        .route("/changelog", get(handlers::changelog))
        .route("/api/models", get(handlers::api_models))
        .route("/api/models/loaded", get(handlers::api_loaded_models))
        .route("/api/stats", get(handlers::api_stats))
        .route("/api/stats/reset", post(handlers::api_stats_reset))
        .route("/api/mode", get(handlers::api_mode))
        .route("/api/settings", get(handlers::api_get_settings))
        .route("/api/settings", post(handlers::api_set_setting))
        .route("/api/chat/history", get(handlers::api_chat_history))
        .route("/api/chat/save", post(handlers::api_chat_save))
        .route("/api/chat/clear", post(handlers::api_chat_clear))
        .route("/api/tests/save", post(handlers::api_test_save))
        .route("/api/tests/history", get(handlers::api_test_history))
        .route("/api/export", get(handlers::api_export))
        .route("/api/import", post(handlers::api_import))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await
        .expect("Failed to bind to 0.0.0.0:3000 — is the port already in use?");
    tracing::info!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.expect("Server error");
}
