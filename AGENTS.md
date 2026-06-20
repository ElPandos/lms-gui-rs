# LMS GUI (Rust)

Web dashboard for managing LM Studio models via CLI and REST API.

## Stack

- **Rust** + **Axum** (async web framework)
- **Askama** (compile-time HTML templates)
- **reqwest** (HTTP client for LMS API)
- **rusqlite** (SQLite persistence)
- **regex** (ANSI escape stripping)
- **htmx** (dynamic UI updates)
- **Tailwind CSS** (styling via CDN)

## Structure

| Path | Purpose |
|------|---------|
| `src/main.rs` | App entry, Axum router setup, shared state |
| `src/handlers/mod.rs` | Handler module exports |
| `src/handlers/pages.rs` | HTML page handlers (dashboard, models, runtime, logs, stats, chat, changelog) |
| `src/handlers/api.rs` | JSON API endpoints (models, settings, chat history, test results, export/import) |
| `src/handlers/chat.rs` | Chat completion + speed test handlers |
| `src/lms_client.rs` | HTTP + SSH client for LM Studio |
| `src/models.rs` | Data models and CLI output parsers |
| `src/stats.rs` | In-memory traffic statistics (atomic counters) |
| `src/db.rs` | SQLite persistence (settings, chat, test results) |
| `build.rs` | Build script (git hash, log, build time injection) |
| `deploy.py` | Deployment (build + upload + health check) |
| `templates/` | 8 Askama HTML templates |
| `static/` | Static assets (JS, CSS) |

## Key Patterns

- **Server-rendered + htmx**: Askama templates with htmx for partial page updates
- **Shared state**: `Arc<AppState>` holding DB pool, HTTP client, stats counters
- **Dual access**: SSH for CLI commands, HTTP for LMS REST API
- **Environment-driven config**: `LMS_LOCAL=1` switches between remote (SSH) and local mode
- **No hardcoded hosts**: All IPs/credentials via environment variables (see `.kiro/steering/no-hardcoded-hosts.md`)

## Running

```bash
cargo run                     # Remote mode (SSH to LMS host)
LMS_LOCAL=1 cargo run         # Local mode (direct CLI, localhost API)
python deploy.py              # Full deploy (build + upload + start)
```
