# LMS GUI (Rust)

Web dashboard for managing LM Studio models via the CLI and REST API.

## Features

- **View models**: Lists models from the LMS API endpoint and local disk
- **Loaded models**: Shows currently loaded models (`lms ps`) with auto-refresh
- **Load/Unload models**: Load and unload models from memory via the web UI
- **Download models**: Trigger model downloads (`lms get <name> -y`) with progress tracking
- **Search models**: Search available models on LM Studio Hub (filters already-downloaded)
- **Delete models**: Remove models from disk
- **Runtime status**: Check inference runtime versions, updates, and switch active runtime
- **Chat**: Full chat interface with configurable settings (temperature, top_p, penalties)
- **Speed tests**: Benchmark models with speed, latency, throughput, stability, and concurrency tests
- **Multi-model testing**: Test all loaded or all available models with automatic load/unload/restore
- **Test results**: Statistical analysis with graphs, outlier detection (sigma filtering), and comparison reports
- **Server logs**: View LMS inference logs, app logs, and per-model download logs
- **Traffic stats**: Live request/error/download counters with event log (auto-refresh)
- **Host info**: CPU, RAM, GPU, disk, and uptime displayed on dashboard
- **Health check**: `/api/health` endpoint with concurrent API + CLI reachability checks
- **Database**: SQLite persistence for settings, chat history, and test results
- **Export/Import**: Full data export and import via JSON
- **Changelog**: Build info with git commit hash and recent commits

## Stack

- **Rust** + **Axum** (async web framework)
- **Askama** (compile-time HTML templates)
- **reqwest** (HTTP client for LMS API)
- **rusqlite** (SQLite database for persistence)
- **regex** (ANSI escape code stripping)
- **Tailwind CSS** (via CDN) + **htmx** (dynamic updates)
- **SSH** (executes `lms` CLI commands on remote host, or local with `LMS_LOCAL=1`)

## Configuration

All host credentials are provided via environment variables — no hardcoded IPs or passwords.

| Variable | Purpose |
|----------|---------|
| `ENV_IP_JUMP_155_HOST` | Remote host IP address |
| `ENV_USER_JUMP_155_HOST` | Remote host SSH username |
| `ENV_PASS_JUMP_155_HOST` | Remote host SSH password |

Set `LMS_LOCAL=1` when running on the LMS host directly (no SSH, uses localhost).

## Running

```bash
cargo run                    # Remote mode (SSH to LMS host via env vars)
LMS_LOCAL=1 cargo run        # Local mode (direct commands, localhost API)
RUST_LOG=debug cargo run     # With debug logging
```

## Deployment

```bash
python deploy.py              # Full deploy (build + upload + start + health check)
python deploy.py --build-only # Build only
python deploy.py --skip-build # Deploy without rebuilding
python deploy.py --diagnostics # Remote diagnostics
```

## Testing

```bash
cargo test                   # Run all 110 tests
cargo test --release bench_ -- --nocapture  # Run benchmarks with p50/p95/p99
```

Tests cover CLI output parsers (`models.rs`), SQLite persistence (`db.rs`), and speed-test stats computation (`chat.rs`). See `docs/reports/` for benchmark baselines.

## Endpoints

### Web UI

| Path | Method | Description |
|------|--------|-------------|
| `/` | GET | Dashboard overview |
| `/models` | GET | Search + local models (accepts `?q=query`) |
| `/runtime/status` | GET | Runtime info + update check |
| `/logs` | GET | Logs (tabs: LMS/App/Downloads, accepts `?q=tab&dl=model`) |
| `/stats` | GET | Traffic statistics (auto-refresh 10s) |
| `/chat` | GET | Chat interface |
| `/api-docs` | GET | API reference page |

### Actions

| Path | Method | Body | Description |
|------|--------|------|-------------|
| `/models/download` | POST | `{"model_name": "..."}` | Download a model |
| `/models/load` | POST | `{"model_name": "...", "parallel": 4}` | Load model into memory |
| `/models/unload` | POST | `{"model_name": "..."}` | Unload model (`--all` supported) |
| `/models/delete` | POST | `{"model_name": "..."}` | Delete model from disk |
| `/runtime/select` | POST | `{"model_name": "..."}` | Select active runtime |

### JSON API

| Path | Method | Description |
|------|--------|-------------|
| `/api/models` | GET | Model list with context metadata (v1+v0 merged) |
| `/api/models/loaded` | GET | Loaded models output |
| `/api/v0/models` | GET | LMS v0 models proxy (context metadata) |
| `/api/health` | GET | API + CLI reachability check |
| `/api/stats` | GET | Traffic stats as JSON |
| `/api/settings` | GET/POST | Get/set settings |
| `/api/chat/history` | GET | Chat history |
| `/api/chat/save` | POST | Save chat message |
| `/api/export` | GET | Export all data as JSON |
| `/api/import` | POST | Import data from JSON |
| `/models/download/status` | GET | Download progress (`?q=model_name`) |
| `/models/load/status` | GET | Load status |

## Project Structure

| Path | Purpose |
|------|---------|
| `src/` | Rust source (main, handlers, models, lms_client, db, stats) |
| `templates/` | Askama HTML templates |
| `static/` | Static asset mount point (JS/CSS via CDN) |
| `scripts/` | Utility scripts (e.g. `archive-bench-to-vault.sh`) |
| `docs/` | Reports and scratchpad archives |
| `deploy.py` | Deployment script (build + upload + health check) |
| `build.rs` | Build script (git hash, build time injection) |
