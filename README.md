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
- **Server logs**: View LMS inference logs, app logs, and per-model download logs
- **Traffic stats**: Live request/error/download counters with event log (auto-refresh)
- **Host info**: CPU, RAM, GPU, disk, and uptime displayed on dashboard

## Stack

- **Rust** + **Axum** (async web framework)
- **Askama** (compile-time HTML templates)
- **reqwest** (HTTP client for LMS API)
- **regex** (ANSI escape code stripping)
- **Tailwind CSS** (via CDN) + **htmx** (dynamic updates)
- **SSH** (executes `lms` CLI commands on remote host, or local with `LMS_LOCAL=1`)

## Configuration

The service connects to:
- LMS API: `http://137.58.231.231:8010/v1/models` (or `localhost:8010` in local mode)
- SSH: `hts@137.58.231.231` (for CLI commands, skipped in local mode)

Set `LMS_LOCAL=1` when running on the LMS host directly (no SSH, uses localhost).

## Running

```bash
cargo run                    # Remote mode (SSH to 137.58.231.231)
LMS_LOCAL=1 cargo run        # Local mode (direct commands)
```

## Deployment

```bash
python deploy.py              # Full deploy (build + upload + start + health check)
python deploy.py --build-only # Build only
python deploy.py --skip-build # Deploy without rebuilding
python deploy.py --diagnostics # Remote diagnostics
```

## Endpoints

### Web UI

| Path | Method | Description |
|------|--------|-------------|
| `/` | GET | Dashboard overview |
| `/models` | GET | Search + local models (accepts `?q=query`) |
| `/runtime/status` | GET | Runtime info + update check |
| `/logs` | GET | Logs (tabs: LMS/App/Downloads, accepts `?q=tab&dl=model`) |
| `/stats` | GET | Traffic statistics (auto-refresh 10s) |

### Actions

| Path | Method | Body | Description |
|------|--------|------|-------------|
| `/models/download` | POST | `{"model_name": "..."}` | Download a model |
| `/models/load` | POST | `{"model_name": "..."}` | Load model into memory |
| `/models/unload` | POST | `{"model_name": "..."}` | Unload model (`--all` supported) |
| `/models/delete` | POST | `{"model_name": "..."}` | Delete model from disk |
| `/runtime/select` | POST | `{"model_name": "..."}` | Select active runtime |

### JSON API

| Path | Method | Description |
|------|--------|-------------|
| `/api/models` | GET | Model list as JSON |
| `/api/models/loaded` | GET | Loaded models output |
| `/api/stats` | GET | Traffic stats as JSON |
| `/models/download/status` | GET | Download progress (`?q=model_name`) |
| `/models/load/status` | GET | Load status |
