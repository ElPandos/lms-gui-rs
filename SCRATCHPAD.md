- **Perf profile 2026-06-28**: Hoisted 3 regexes to `LazyLock` in `models.rs` (quantizations 3147×, param_count 609× faster), switched `stats.rs` event log to `VecDeque` (1.7×), made `list_models` handler I/O concurrent via `tokio::join!` (5 serial SSH round-trips → 1). Search page CPU: 310ms→68μs. Report: `docs/reports/profile-lms-gui-rs-2026-06-28.md`.
- `models.rs::parse_model_columns` returns a 5-tuple `(String, String, String, String, String)` — unnamed return values; callers can't tell params/arch/size/device/status apart. Should return a small struct.
- `db.rs::save_chat_message` takes 7 positional `&str`/`Option<&str>`/`Option<u64>` params — call site at `api.rs` L84 is a 120-char positional argument list.
- `CommandResult { success, message }` is the generic envelope for every action endpoint; `message` is overloaded as either a status string, an error string, or command stdout — ambiguous contract.
- `handlers/api.rs` handlers prefixed `api_*` is redundant with the `/api/` route prefix (e.g. `api_models` mounted at `/api/models`).

### Risk patterns (unwrap/expect)
- `db.rs`: 8× `.lock().unwrap()` (poisoned-mutex panic) + 6× `.unwrap()` on SQL statement prep (schema-drift panic). A single corrupted DB or lock panic kills the whole web server.
- `lms_client.rs` L34: `Regex::new(...).unwrap()` — static regex, fail-on-startup if pattern invalid (acceptable but worth a `LazyLock` + `expect` with message).
- `lms_client.rs` L11-12, `main.rs` L48, L62, L110, L112: `.expect(...)` on env vars and bind — acceptable at startup, but `ENV_IP_JUMP_155_HOST` referenced in 3 places (lms_client + main) — should be resolved once and passed in.
- `handlers/chat.rs` L225-226: `partial_cmp(b).unwrap()` on `f64` — NaN would panic; safe only because durations come from `as_millis()`.

### Security smells (noted, not audited)
- `lms_client.rs::sanitize` (L460) — allowlist sanitizer for shell args, but commands are assembled via `format!` and passed to `bash -c` / `ssh ... bash -c`. Correct-by-inspection but fragile; a single missed character class = shell injection. Should use parameterized invocation where possible.
- `lms_client.rs::download_model` / `delete_model` — user-supplied `name` flows into `pkill -f`, `find ... -name`, `rm -rf "$dir"`. Sanitization is present but the `rm -rf` on a resolved directory is high-blast-radius; worth defense-in-depth (e.g. constrain to `$HOME/.lmstudio/models`).

## Top Refactoring Targets (ranked top 10)

1. **`models.rs::parse_loaded_models`** — hotspot_score=9 — 140-LOC function, 5+ nesting, two duplicated parsing strategies, zero tests; any `lms ps` format change breaks silently.
2. **`lms_client.rs::LmsClient` (god-object split)** — hotspot_score=9 — 30 methods / 583 LOC mixing HTTP, SSH, process mgmt, chat, host probe; split into 4 focused structs.
3. **`pages.rs` action-handler duplication** — hotspot_score=8 — 6 near-identical `download/load/unload/delete/select_runtime/cancel` handlers (~90 LOC) collapsible to one `wrap_action` helper + table.
4. **`models.rs` module split** — hotspot_score=8 — 688 LOC, 11 structs, 6 parsers in one file; split by concern (api/cli/hf/host).
5. **`handlers/chat.rs::chat_speedtest`** — hotspot_score=7 — 100-LOC god method mixing test execution + stats + outlier filtering; extract `compute_stats` + `mark_outliers` (both pure-fn, unit-testable).
6. **`db.rs` lock/unwrap hardening** — hotspot_score=7 — 8× `.lock().unwrap()` + 6× stmt `.unwrap()`; one DB hiccup panics the server. Wrap in `with_conn` + `Result`-returning helpers.
7. **`pages.rs::list_models`** — hotspot_score=7 — 70-LOC handler with 4 nesting + 12-field template constructor; extract search dispatcher and a builder for `ModelsTemplate`.
8. **`lms_client.rs::run_cmd`** — hotspot_score=7 — 57 LOC, 5 nesting, triple-`map_err` + `spawn_blocking` + timeout; extract SSH-vs-local strategy + error-mapping helper.
9. **`models.rs::quant_vram_bytes` + `quant_desc` merge** — hotspot_score=6 — two parallel `match` over same 14 tokens; consolidate into one `static QUANTS: &[(name, bytes, desc)]` table.
10. **`db.rs::save_chat_message` / `save_test_result` (7-param signatures)** — hotspot_score=6 — positional arg lists >120 chars at call sites; introduce `ChatMessage` / `TestResult` record structs.

### Cross-cutting recommendation
Introduce a test scaffold before any refactor: `tests/models_parse.rs` covering the 6 `parse_*` fns with captured CLI fixtures, and `#[cfg(test)] mod tests` in `db.rs` (in-memory `:memory:` SQLite). These two files alone cover the highest-risk surface (text parsing + persistence) and give the refactor a safety net. The `stats.rs` and `models.rs` pure functions are the cheapest wins for coverage.

## Learning Analysis (2026-06-28 05:01)

### Observation Patterns
```
analyze: only 11 observation(s) (min=20)
scope: lms-gui-rs (75c9b909266a)
status: insufficient data for pattern analysis
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:01)

### Observation Patterns
```
analyze: only 11 observation(s) (min=20)
scope: lms-gui-rs (75c9b909266a)
status: insufficient data for pattern analysis
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:04)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 24
parsed: 24
parse_errors: 0

tool_usage:
  bash: 19
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  edit: 1
  sequential-thinking_sequentialthinking: 1

error_resolutions: 0

user_corrections: 0

repeated_workflows (min 3):
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:04)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 24
parsed: 24
parse_errors: 0

tool_usage:
  bash: 19
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  edit: 1
  sequential-thinking_sequentialthinking: 1

error_resolutions: 0

user_corrections: 0

repeated_workflows (min 3):
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:05)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 32
parsed: 32
parse_errors: 0

tool_usage:
  bash: 19
  edit: 8
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1
  task: 1

error_resolutions: 0

user_corrections: 6
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - bash -> bash -> bash (x14)
  - edit -> edit -> edit (x5)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:05)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 32
parsed: 32
parse_errors: 0

tool_usage:
  bash: 19
  edit: 8
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1
  task: 1

error_resolutions: 0

user_corrections: 6
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - bash -> bash -> bash (x14)
  - edit -> edit -> edit (x5)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:05)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 50
parsed: 50
parse_errors: 0

tool_usage:
  edit: 24
  bash: 20
  task: 2
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1

error_resolutions: 0

user_corrections: 21
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - edit -> edit -> edit (x19)
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:05)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 50
parsed: 50
parse_errors: 0

tool_usage:
  edit: 24
  bash: 20
  task: 2
  todowrite: 1
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1

error_resolutions: 0

user_corrections: 21
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - edit -> edit -> edit (x19)
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:06)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 61
parsed: 61
parse_errors: 0

tool_usage:
  bash: 25
  edit: 24
  task: 3
  write: 3
  todowrite: 2
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1
  invalid: 1

error_resolutions: 0

user_corrections: 21
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - edit -> edit -> edit (x19)
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._

## Learning Analysis (2026-06-28 05:06)

### Observation Patterns
```
analyze: observation summary
scope: lms-gui-rs (75c9b909266a)
total_observations: 61
parsed: 61
parse_errors: 0

tool_usage:
  bash: 25
  edit: 24
  task: 3
  write: 3
  todowrite: 2
  filesystem_read_multiple_files: 1
  filesystem_search_files: 1
  sequential-thinking_sequentialthinking: 1
  invalid: 1

error_resolutions: 0

user_corrections: 21
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/mod.rs","oldString":"mod api;\nmod chat
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/api.rs","oldString":"use axum::extract:
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/chat.rs","oldString":"use axum::extract
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/src/handlers/pages.rs","oldString":"use askama::Temp
  - consecutive-edits-same-dir: {"filePath":"/home/emvekta/_PROJECTS/lms-gui-rs/LESSONS.md","oldString":"- **Prevention**: When load

repeated_workflows (min 3):
  - edit -> edit -> edit (x19)
  - bash -> bash -> bash (x14)

status: review complete - no instincts auto-created
next: orchestrator or specialist should review patterns and create instincts
```

### LESSONS.md Proposals
```
evolve: no instincts to cluster (scope=all)
```

_Review and merge applicable proposals into LESSONS.md. Run
`instinct-cli.py promote` to promote project instincts to global._
