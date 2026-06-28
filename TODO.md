# Refactor Plan — lms-gui-rs

 TRACE: T-20260627-0001-01
 Created: 2026-06-27
 Mode: Full project refactor + error-handling improvements
 Baseline: see SCRATCHPAD.md (2,607 LOC, 10 .rs files, 0% test coverage)

## Constraints
- Behavior-preserving (no logic changes)
- Depth 1-2 only (no architectural rewrites)
- No new abstractions unless 3+ concrete uses exist
- Leaf modules first, then work inward
- Preserve all tracing/logging — may improve messages, never remove
- Public API (route paths, JSON shapes) unchanged

---

## Phase A: Safety Net (characterization tests)

Zero test coverage = every refactor is blind. Build the safety net first
against CURRENT behavior, then refactor underneath it.

### A1: Add test dependencies
- [x] [STATUS: complete]
 TARGET: Cargo.toml
 SMELL: no test deps available
 PROPOSED: add `tempfile` as dev-dependency for in-memory SQLite DB tests
 BOUNDARY_IMPACT: none
 COMPLEXITY: simple

### A2: Characterization tests for models.rs parsers
- [x] [STATUS: complete]
 TARGET: src/models.rs (5 public parse fns + 1 private parse_model_columns + HfModel methods)
 SMELL: 688 LOC, 0% coverage, brittle CLI text parsing
 PROPOSED: add `#[cfg(test)] mod tests` with captured CLI fixtures covering:
   - parse_loaded_models: header-based + keyword-fallback paths, empty/no-models input
   - parse_local_models: LLM/Embedding sections, variant suffix stripping, column edge cases
   - parse_model_columns: <5 cols edge case, size with "GB"/"MB" suffix, "12 GB" split form
   - parse_search_results: em-dash/en-dash/double-hyphen separators, dedup, bare-name path
   - parse_runtimes: selected/unselected, version extraction, non-engine lines
   - parse_host_info: full 11-field pipe-delimited, partial/empty input
   - HfModel::param_count: "7B", "3.2B", "12x4B" MoE, no-match
   - HfModel::recommended_quant: fits-in-vram, nothing-fits, zero-vram
   - HfModel::quantizations: GGUF filename extraction, dedup, sort
 TECHNIQUE: characterization tests (capture current output, not desired output)
 BOUNDARY_IMPACT: none (test-only code)
 COMPLEXITY: standard
 KNOWN_BUG_LOCKED_IN: HfModel::param_count("12x4B") returns 4.0, not 48.0 — the first
   regex `(\d+\.?\d*)[bB]` matches "4B" before the MoE regex runs. Characterization
   tests MUST assert the current (buggy) value 4.0. Do NOT fix this bug in this refactor
   — fixing it is a logic change, not behavior-preserving. Note it as follow-up tech debt.

### A3: Characterization tests for db.rs
- [x] [STATUS: complete]
 TARGET: src/db.rs
 SMELL: 0% coverage, 9x .lock().unwrap(), panic-on-schema-drift
 PROPOSED: add `#[cfg(test)] mod tests` using in-memory SQLite (`Database::new(":memory:")`):
   - set_setting/get_setting round-trip, get missing key
   - get_all_settings ordering
   - save_chat_message → get_chat_history (role, model, content, optional fields)
   - clear_chat_history
   - save_test_result → get_test_results
   - export_all → import_all round-trip (settings + chat_history ONLY — import_all does
     NOT import test_results; do not "fix" this, it's current behavior)
 TECHNIQUE: characterization tests
 BOUNDARY_IMPACT: none (test-only code)
 COMPLEXITY: standard

---

## Phase B: Error-Handling Improvements (user-requested)

Runtime logs showed: v0 API unreachable, lms CLI auth failure (passkey mismatch).
Current error paths work (graceful fallback) but messages are opaque to the user.

### B1: Add /api/health endpoint (FEATURE, not refactor)
- [x] [STATUS: complete]
 TARGET: src/handlers/api.rs + src/main.rs (route) + src/lms_client.rs (probe method)
 SMELL: no way for user/UI to check if LMS is reachable
 NOTE: This is a feature addition, not a behavior-preserving refactor. Included
       because the user explicitly requested error-handling improvements. It does
       not modify existing routes — new endpoint only.
 PROPOSED: add `GET /api/health` returning structured JSON:
   ```json
   {"api_reachable": bool, "cli_reachable": bool, "api_error": "...", "cli_error": "...", "base_url": "..."}
   ```
   - API check: use `Client::builder().timeout(Duration::from_secs(3)).build()` then
     GET `base_url + "/v1/models"` (NOT `reqwest::get()` — it has no timeout API)
   - CLI check: `lms --version` via run_cmd (lightweight, always works if CLI installed)
   - Both checks run concurrently via `tokio::join!`, errors captured as strings
 TECHNIQUE: extract health-probe method in lms_client + thin handler
 BOUNDARY_IMPACT: new route in main.rs, new method in lms_client.rs
 COMPLEXITY: standard

### B2: Classify connection errors in lms_client.rs
- [x] [STATUS: complete]
 TARGET: src/lms_client.rs (list_local_models_v0, chat_completion, run_cmd error paths)
 SMELL: generic "error sending request" / "Command failed" messages — user can't tell
         if LMS is down, CLI auth broken, or network unreachable
 VERIFIED: run_cmd (L579) returns `Err(format!("Command failed: {}", stderr))` — stderr
           IS propagated in the error string, so pattern-matching on it is viable.
 PROPOSED: map reqwest errors to typed messages:
   - `is_connect()` → "Cannot reach LMS API at <url> — is LM Studio running?"
   - `is_timeout()` → "LMS API timed out — server may be overloaded"
   - stderr contains "Invalid passkey" → "LMS CLI authentication failed — run 'lms server stop && lms server start' on the host"
   - stderr contains "command not found" → "lms CLI not installed on host"
   Keep existing fallback behavior (v0 fail → lms ls). Only improve message strings.
 TECHNIQUE: introduce error-classification helper fn, use at error mapping sites
 BOUNDARY_IMPACT: error message strings change (behavior preserved — same Ok/Err results)
 COMPLEXITY: standard
 PRESERVES_BEHAVIOR: yes-with-caveat:error message text changes, but success/failure outcomes unchanged

---

## Phase C: Complexity Refactoring (leaf modules first)

### C1: Consolidate quant lookup tables in models.rs
- [x] [STATUS: complete]
 TARGET: src/models.rs (quant_vram_bytes L249-268 + quant_desc L271-290)
 SMELL: two parallel `match q.as_str()` over same 14 quantization tokens —
         adding a new quant requires editing both; drift risk
 PROPOSED: single `static QUANTS: &[(name, bytes_per_param, description)]` table,
           both methods index into it.
 ALIAS_HANDLING: quant_vram_bytes collapses aliases ("F16"|"FP16", "Q5_0"|"Q5_1",
   "Q4_0"|"Q4_1") to one bytes value; quant_desc gives DIFFERENT descriptions for
   some aliases (F16 vs FP16 share desc, but Q5_0 vs Q5_1 share). Design:
   - Table has one row per canonical name with aliases listed
   - `quant_vram_bytes`: normalize input via alias map → look up bytes
   - `quant_desc`: normalize input via alias map → look up desc
   - Since all aliased pairs share both bytes AND desc in current code, duplicate
     rows (one per alias) is simplest and avoids a separate normalization step
 TECHNIQUE: replace parallel match arms with lookup table
 BOUNDARY_IMPACT: none (private method + same public signatures)
 COMPLEXITY: simple
 PRESERVES_BEHAVIOR: yes

### C2: Extract speedtest stats computation from chat.rs
- [x] [STATUS: complete]
 TARGET: src/handlers/chat.rs::chat_speedtest (L191-232, ~42 LOC of pure math)
 SMELL: 100-LOC god method mixing test execution + stats + outlier filtering
 PROPOSED: extract `compute_speedtest_stats(results: &mut [SpeedTestCall], total_calls: u32, sigma: u32) -> SpeedTestStats`
           as a pure function. chat_speedtest calls it after collecting results.
           - Takes `&mut [SpeedTestCall]` because outlier marking mutates results (L201-205)
           - Takes `total_calls: u32` because stats.total_calls comes from req.num_calls (L229),
             not from results.len() (which could differ if calls failed)
           - Computes: mean, variance, std_dev, marks outliers on results, filtered_mean,
             median (even/odd), min, max
           Then add unit tests for the extracted fn (edge cases: empty, 1 call,
           even/odd count, all-outliers, sigma=0)
 PREREQUISITE: add characterization test for chat_speedtest's current stats output
              BEFORE extracting (was missing from Phase A — add a #[cfg(test)] mod
              in chat.rs that verifies current stats values for a fixed input set)
 TECHNIQUE: extract function
 BOUNDARY_IMPACT: none (internal refactor, same handler signature)
 COMPLEXITY: standard
 PRESERVES_BEHAVIOR: yes

### C3: Replace 7-param save_chat_message signature with record struct
- [x] [STATUS: complete]
 TARGET: src/db.rs::save_chat_message (L102, 7 params)
 SMELL: 7 positional parameters — call site at api.rs L84 is 120+ chars,
        easy to swap role/model/content by mistake
 PROPOSED: introduce `ChatMessage` parameter struct:
   ```rust
   pub struct ChatMessage<'a> {
       pub role: &'a str, pub model: &'a str, pub content: &'a str,
       pub settings_json: Option<&'a str>, pub response_json: Option<&'a str>,
       pub duration_ms: Option<u64>, pub tokens: Option<u32>,
   }
   ```
   Update save_chat_message signature to `save_chat_message(&self, msg: &ChatMessage)`.
   Update call sites: api.rs L84 (api_chat_save), db.rs L201 (import_all).
 NOTE: save_test_result is NOT changed — it has only 1 call site (api.rs L120),
       which does not meet the 3+ uses threshold for introducing an abstraction.
 TECHNIQUE: introduce parameter object
 BOUNDARY_IMPACT: callers in src/handlers/api.rs + src/db.rs::import_all must update
 COMPLEXITY: standard
 PRESERVES_BEHAVIOR: yes

### C4: Extract render_or_error helper in pages.rs
- [x] [STATUS: complete]
 TARGET: src/handlers/pages.rs (8x `Html(tmpl.render().unwrap_or_else(...))`)
 SMELL: identical 1-line error-handling pattern repeated 8 times
 PROPOSED: `fn render_or_error<T: Template>(tmpl: T) -> Html<String>`
           used at all 8 sites. Reduces duplication, centralizes error format.
 TECHNIQUE: extract function
 BOUNDARY_IMPACT: none (internal helper, same return type)
 COMPLEXITY: simple
 PRESERVES_BEHAVIOR: yes

### C5: Harden db.rs panic-on-unwrap patterns
- [x] [STATUS: complete]
 TARGET: src/db.rs (9x .lock().unwrap(), 6x SQL .unwrap() on stmt prepare)
 SMELL: single DB hiccup panics the whole web server
 PROPOSED:
   - Read methods returning Vec (get_all_settings, get_chat_history, get_test_results):
     restructure each from `let mut stmt = conn.prepare(...).unwrap(); stmt.query_map(...).unwrap()...`
     to `match conn.prepare(...) { Ok(mut stmt) => stmt.query_map(...).filter_map(...).collect(),
     Err(e) => { tracing::error!(error=%e, "Stmt prep failed"); Vec::new() } }`.
     Cannot use inline .unwrap_or_else because `stmt` is used on the next line.
   - Mutex lock (9 sites): replace `.unwrap()` with `.unwrap_or_else(|e| {
     tracing::error!("DB mutex poisoned: {}", e); e.into_inner() })` —
     recover from poison by taking the guard anyway (std::sync::PoisonError::into_inner)
   - Do NOT change Result-returning methods (set_setting, save_chat_message, clear_chat_history,
     save_test_result) — they already map errors properly, only the `.lock().unwrap()` changes there
 TECHNIQUE: replace unwrap with logged fallback / poison recovery
 BOUNDARY_IMPACT: none (same public signatures, same return values on success)
 COMPLEXITY: moderate (requires restructuring 3 read methods, not just inline replacement)
 PRESERVES_BEHAVIOR: yes-with-caveat:on mutex poisoning, now recovers instead of panicking
   (strictly more robust, not a behavior change for the non-poisoned path)

---

## Phase D: Verification

### D1: Final build + test + clippy verification
- [x] [STATUS: complete]
 TARGET: whole project
 PROPOSED: run `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build`
           Confirm: all characterization tests pass, no new warnings, build clean.
           Verify public API unchanged (same routes, same JSON response shapes).
 COMPLEXITY: simple

---

## Self-Consistency Check (revised after review)
 1. Conflicts? C3 (record struct) and C5 (harden unwrap) both touch db.rs —
    execute C3 first (signature change), then C5 (internal hardening). No conflict.
 2. Ordering? A1→A2,A3 (deps before tests). B1,B2 independent of C. C1 (models.rs)
    before C2 (chat.rs) — no dependency but leaf-first. C3 before C5 (both db.rs).
    C2 has prerequisite: add chat.rs characterization test before extraction.
    C4 independent. D1 last.
 3. Every extract has caller updates? C3: yes (api.rs L84 + import_all L201).
    C2: yes (chat_speedtest calls extracted fn). C4: yes (8 call sites in pages.rs).
    B1: yes (main.rs route + new handler + new lms_client method).
 4. Circular deps? None — all extracts are downward (into helpers/structs).
 5. Over-engineering? Skipped: LmsClient god-object split (depth 3, not requested),
    models.rs module split (depth 3), action-handler wrapper (abstraction risk — 6 handlers
    differ in stats/error handling, not truly identical), TestResultRecord (only 1 call site).
 6. Known bugs locked in by characterization tests: param_count MoE regex (A2).
    Documented as follow-up tech debt, NOT fixed in this refactor.

## Follow-up Tech Debt (NOT in this refactor)
 - HfModel::param_count: "12x4B" returns 4.0 instead of 48.0 (regex order bug)
 - db.rs::import_all: does not import test_results (export/import asymmetry)
 - LmsClient god-object split (depth 3 architectural)
 - models.rs module split into api/cli/hf/host (depth 3 architectural)
 - pages.rs action-handler wrapper (6 handlers, abstraction risk)
