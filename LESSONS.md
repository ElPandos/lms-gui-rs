# Lessons Learned

## [ssh] SSH non-interactive PATH not set

- **Last seen**: 2026-06-16
- **Times seen**: 1
- **Symptom**: `lms: command not found` when running via SSH
- **Root cause**: Non-interactive SSH sessions don't source `.bashrc`/`.profile`, so `~/.lmstudio/bin` isn't in PATH
- **Fix**: Prepend `export PATH="$HOME/.lmstudio/bin:$PATH"` to every SSH command
- **Prevention**: Always set PATH explicitly in SSH commands for non-standard binaries

---

## [ssh] SSH connection latency dominates page load

- **Last seen**: 2026-06-16
- **Times seen**: 1
- **Symptom**: Dashboard takes 2.7s to load despite 17ms ping
- **Root cause**: Each SSH command opens a new TCP+auth handshake (~600ms). Dashboard makes 4 parallel SSH calls
- **Fix**: SSH ControlMaster multiplexing (`ControlPersist=600`) reduces per-call latency to ~90ms
- **Prevention**: Always use SSH multiplexing for apps that make frequent SSH calls to the same host

---

## [cli] ANSI escape codes in CLI output

- **Last seen**: 2026-06-16
- **Times seen**: 1
- **Symptom**: Search results show garbled `[34m?[39m` characters
- **Root cause**: `lms` CLI outputs ANSI color codes and cursor movement sequences even when piped
- **Fix**: Strip ANSI with regex/state machine before displaying
- **Prevention**: Always strip ANSI from SSH command output in web UIs

---

## [process] Long-running SSH commands timeout

- **Last seen**: 2026-06-16
- **Times seen**: 2
- **Symptom**: Model downloads and loads timeout at 15-30s, button spinners stuck
- **Root cause**: SSH command wrapper has a tokio timeout; `lms get` and `lms load` can take minutes
- **Fix**: Use `nohup` to background long commands, write to log file, poll status separately
- **Prevention**: Any CLI command that can run >10s should be backgrounded with progress polling

---

## [ui] Toast messages overflow with CLI progress output

- **Last seen**: 2026-06-16
- **Times seen**: 1
- **Symptom**: Toast shows massive wall of "Loading 0% 1% 2%..." text
- **Root cause**: Load command waited for completion, returning full progress stream as the response
- **Fix**: Background the command (return "Loading started"), truncate toasts to 100 chars
- **Prevention**: Never return raw CLI streaming output as API response; background + poll instead

---

## [data] LM Studio search returns duplicate entries

- **Last seen**: 2026-06-16
- **Times seen**: 1
- **Symptom**: Same model appears twice in search results
- **Root cause**: `lms get <query>` output contains duplicate lines from LM Studio's interactive picker
- **Fix**: Deduplicate parsed results using a HashSet on model name
- **Prevention**: Always deduplicate when parsing external CLI output

---

## [process] nohup over SSH still hangs (fd inheritance)

- **Last seen**: 2026-06-17
- **Times seen**: 3
- **Symptom**: SSH command hangs on `nohup ./binary > log 2>&1 &` — never returns
- **Root cause**: Background process inherits stdout/stderr pipe fds from parent shell; SSH waits for EOF on those fds which never comes
- **Fix**: Use subshell pattern `(cmd > log 2>&1 &) && echo done` — subshell closes fds immediately
- **Prevention**: For remote process start via SSH, always use `(cmd > log 2>&1 &)` subshell, never bare `nohup cmd &`

---

## [process] Duplicate download processes cause "already in progress" lock

- **Last seen**: 2026-06-17
- **Times seen**: 1
- **Symptom**: Download stuck at 0% with "This download is already in progress"
- **Root cause**: Multiple clicks spawned duplicate `lms get` processes; LM Studio's internal download manager locks on model name
- **Fix**: `pkill -f 'lms get <model>'` before starting new download; clear log file
- **Prevention**: Kill existing process for same model before spawning new download

---

## [parsing] ANSI stripper must use regex, not hand-rolled state machine

- **Last seen**: 2026-06-17
- **Times seen**: 1
- **Symptom**: Search results return 0 parsed models despite CLI outputting 24
- **Root cause**: Hand-rolled ANSI stripper broke on `\x1b[?25h` sequences (used `is_alphabetic()` which matches Unicode, and `?` was handled wrong)
- **Fix**: Replace with regex: `\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07|\r`
- **Prevention**: Use regex crate for ANSI stripping; hand-rolled parsers miss edge cases

---

## [timing] Rust tokio timeout must exceed inner command timeout

- **Last seen**: 2026-06-17
- **Times seen**: 1
- **Symptom**: Search returns empty (timeout error) even though CLI produces output
- **Root cause**: `timeout 15` in the shell command + SSH overhead = 16s total, but Rust `tokio::time::timeout` was also 15s — race condition
- **Fix**: Set Rust timeout to 20s (5s buffer over the 15s command timeout)
- **Prevention**: Outer timeout must always be inner_timeout + transport_overhead + buffer

---

## [ui] Download completion detection must match actual log text

- **Last seen**: 2026-06-18
- **Times seen**: 1
- **Symptom**: Active download card stays visible after download finishes
- **Root cause**: Checked for `"downloaded"` but log says `"Download completed."` — `"download completed"` doesn't contain `"downloaded"`
- **Fix**: Added `"download completed"` and `"finalizing"` to completion detection
- **Prevention**: Always check actual log output format before writing detection logic

---

## [js] Duplicate const declaration kills all page JS

- **Last seen**: 2026-06-20
- **Times seen**: 1
- **Symptom**: All buttons/interactions stop working — no visible error in UI
- **Root cause**: Two `const model = ...` declarations in same function scope causes SyntaxError that silently breaks entire script block
- **Fix**: Remove duplicate declaration, reuse existing variable
- **Prevention**: After adding code to existing functions, always check for variable name collisions. Use JS syntax check (`node -c`) on template scripts.

---

## [parsing] lms ps column parsing must use header positions

- **Last seen**: 2026-06-20
- **Times seen**: 2
- **Symptom**: Loaded models show no status badge, or wrong status (random characters)
- **Root cause**: Keyword-based column detection (`find("IDLE")`) fails when model names contain no `/` or status is a non-standard value
- **Fix**: Parse header line positions (IDENTIFIER, STATUS, SIZE columns) and extract by character offset
- **Prevention**: For tabular CLI output, always use header-based column position parsing, never keyword search in data rows

---

## [ui] Custom dropdowns need careful event handling

- **Last seen**: 2026-06-20
- **Times seen**: 1
- **Symptom**: Custom model dropdown wouldn't open or select items
- **Root cause**: Click-outside listener conflicts with toggle button, event propagation issues, and Askama template rendering `{% break %}` not supported
- **Fix**: Reverted to native `<select>` which always works reliably
- **Prevention**: Only use custom dropdowns when native select is truly insufficient. Test click handling thoroughly. Avoid unsupported template syntax.

---

## [async] Model load is async — must poll before using

- **Last seen**: 2026-06-20
- **Times seen**: 1
- **Symptom**: Multi-model test stuck at "Preparing..." or warmup call hangs
- **Root cause**: `lms load` returns immediately ("Loading started") but model takes 10-30s to actually load into VRAM
- **Fix**: Poll `/api/models/loaded` every 2s until model name appears (max 60s timeout)
- **Prevention**: After any `lms load` call, always poll for model presence before sending inference requests

---

## [css] Tailwind `hidden` class conflicts with JS toggle

- **Last seen**: 2026-06-20
- **Times seen**: 1
- **Symptom**: Manual input field never shows when "Manual" is selected in dropdown
- **Root cause**: `classList.toggle('hidden', condition)` didn't work reliably — possibly due to settings restore overwriting DOM before toggle runs
- **Fix**: Use inline `style.display = 'block'/'none'` which has higher specificity and can't be overridden by class conflicts
- **Prevention**: For JS-toggled visibility, prefer inline style over class manipulation when persistence/restore is involved

---

## [logging] Rolling log file triggers cargo-watch restart loop

- **Last seen**: 2026-06-20
- **Times seen**: 1
- **Symptom**: App restarts repeatedly every 2 seconds when using cargo-watch or file watcher
- **Root cause**: tracing-appender writes rolling log files (lms-gui-rs.log.*) to the project directory; file watcher detects new/modified file → recompiles → restarts → writes log → loop
- **Fix**: Add `lms-gui-rs.log*` to .gitignore and watcher ignore (cargo-watch: `-i "lms-gui-rs.log*"`)
- **Prevention**: When adding file-based logging, always add log patterns to .gitignore and watcher exclusions in the same commit

---

## [serde] Wrong field type in external API struct silently empties results

- **Last seen**: 2026-06-27
- **Times seen**: 1
- **Symptom**: HuggingFace model searches returned zero results with no errors in logs after enriching `HfModel`
- **Root cause**: `gated` typed `Option<String>` per research note ("auto"/"manual"/null), but HF API returns a **boolean**. Serde failed deserializing `false` into `Option<String>`; handler swallowed the error via `.unwrap_or_default()` returning empty vec with no log entry (errors returned as strings, not via `tracing`)
- **Fix**: Changed `gated: Option<String>` → `gated: bool`; template check `.is_some()` → direct bool; added `tracing::error!` on parse-failure path in lms_client.rs
- **Prevention**: For serde structs of external APIs, ALWAYS validate field types against an actual API response (`curl` + inspect JSON) — research notes about "possible values" are not ground truth. Error-returning methods that callers swallow with `.unwrap_or_default()` must `tracing::error!` the error BEFORE returning, or failures go completely silent

---

## [lms] Parallel slots divide context window — agents get "n_ctx too small"

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Zed/IDE sends ~80K-token prompt, LM Studio errors with `n_keep (80079) >= n_ctx (40960)` even though `lms ps` shows `CONTEXT: 131072`
- **Root cause**: LMS default `--parallel 4` divides the loaded context across slots for continuous batching. With VRAM guardrails, per-slot context drops well below the loaded value (131072 loaded → ~40960 effective per slot). The OpenAI-compatible `/v1/models` endpoint reports `max_context_length: null`, hiding this from clients
- **Fix**: Load with `--parallel 1` for full per-request context (`lms load <model> --context-length 131072 --parallel 1`). Added a Parallel setting to the GUI (defaults to 4, set to 1 for agent/IDE use)
- **Prevention**: When loading models for IDE/agent use (large system prompts), always set `--parallel 1`. The `/v1/models` endpoint never reports loaded context — query `/api/v0/models` for `loaded_context_length` instead

---

## [regex] Specific patterns must precede generic in alternation order

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: `HfModel::param_count("12x4B")` returns `4.0` instead of `48.0`
- **Root cause**: Regex `(\d+\.?\d*)[bB]` matches "4B" before the MoE pattern `(\d+)x(\d+\.?\d*)[bB]` runs — Rust `regex` crate uses leftmost match, and the generic pattern matches a substring of the MoE form
- **Fix**: Order alternations from most-specific to least-specific, or anchor MoE pattern so it wins for "12x4B"
- **Prevention**: When combining regex alternations, always test compound forms (MoE, scientific notation) first. Add a test case for every compound form. Treat "first match wins" as a footgun, not a feature

---

## [testing] Characterization tests must assert CURRENT (buggy) behavior

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Temptation to write `assert_eq!(param_count("12x4B"), 48.0)` (correct value) when writing the safety-net test
- **Root cause**: Characterization tests pin behavior so a refactor can be verified as behavior-preserving. Asserting the *desired* value breaks the safety net — a later behavior-preserving change would appear to fail, and a real regression to a *different* buggy value could pass unnoticed
- **Fix**: Assert `4.0` (current buggy output) with a `// KNOWN BUG: should be 48.0, see follow-up tech debt` comment linking the issue
- **Prevention**: Before writing any characterization test, run the code under test and assert whatever it currently returns. Track locked-in bugs in a "Follow-up Tech Debt" section of the plan, never "fix" them in the test assertion

---

## [process] Behavior-preserving refactors must not fix bugs discovered along the way

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Discovered the `param_count("12x4B")` regex bug mid-refactor and wanted to fix it "while we're here"
- **Root cause**: Fixing a bug during a behavior-preserving refactor invalidates the safety net (characterization test now asserts old behavior the refactor changed), expands scope, and makes the diff unreviewable for behavior preservation — reviewers can't tell which changes are refactor vs. fix
- **Fix**: Note the bug in "Follow-up Tech Debt" section of the plan, leave it unfixed, ensure the characterization test pins the buggy behavior
- **Prevention**: Define "behavior-preserving" upfront as "characterization tests pass unchanged." Any discovered bug goes to a separate follow-up ticket with its own test+fix PR, never bundled into the refactor

---

## [shell] `&&` chaining short-circuits cleanup when first command fails

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Deleted model reappears in `lms ls` immediately after deletion — the `rm -rf` never ran
- **Root cause**: `lms unload <model> && rm -rf <dir>` — `lms unload` returns non-zero when the model isn't currently loaded, so `&&` short-circuits and skips the `rm -rf`. The model files stay on disk and `lms ls` rescans them on next call
- **Fix**: Use `;` (sequential) instead of `&&` so cleanup runs regardless of unload's exit status. No LMS server restart needed — `lms ls` rescans the filesystem on each invocation
- **Prevention**: For multi-step cleanup commands, prefer `;` over `&&` when steps are independent. `&&` means "only proceed if this succeeded"; cleanup must always run. Test the "model not loaded" edge case for any delete path

---

## [cli] LMS model ID does not match on-disk directory name

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: `find -name 'qwen3-4b'` returns nothing; model files not found for deletion
- **Root cause**: LMS exposes a normalized model ID (`qwen3-4b`) but the HuggingFace download directory uses the original repo casing (`Qwen3-4B-GGUF`). Exact case-sensitive `find -name` misses it
- **Fix**: Use `find -iname '*qwen3-4b*'` (case-insensitive wildcard) to locate the directory
- **Prevention**: Never assume LMS model IDs match filesystem paths — LMS normalizes names. Always use case-insensitive wildcard search when locating model directories by ID

---

## [lms] `lms load -y` forces duplicate instances (`:2` suffix)

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Loading an already-loaded model creates a second instance with `:2` suffix in `lms ps`
- **Root cause**: The `-y` flag forces a new load even when the model is already loaded, bypassing LMS's dedup. Calling load without checking loaded state first creates duplicates
- **Fix**: Remove `-y`; before calling `lms load`, check `/api/models/loaded` (or `lms ps`) and skip if the model is already loaded
- **Prevention**: Never use force/`-y` flags by default. Always check current state before issuing idempotent-intended commands. Treat "load" as a no-op-if-loaded operation, not a force-reload

---

## [process] Orphaned `.part` files accumulate from multiple cleanup gaps

- **Last seen**: 2026-06-28
- **Times seen**: 2
- **Symptom**: 14GB of orphaned partial download files accumulated in the models directory after repeated canceled/failed downloads
- **Root cause**: Multiple independent cleanup paths each missed `.part` files. (1) `cancel_download` killed the `lms get` process and cleaned PID files but never deleted `.part` files. (2) Pre-start cleanup killed stale processes and removed PID/log files but NEVER deleted old `.part` files. Each path assumed another path handled it
- **Fix**: (1) On cancel, also delete the download's `.part` files / target directory. (2) Pre-start cleanup now deletes `.part` files; reaper cleans tracked downloads' `.part` files on startup
- **Prevention**: Any cancel/abort/cleanup handler must clean up ALL side effects of the canceled process, not just the process itself. When multiple cleanup paths exist, each must be independently complete — never assume "the other path handles it." Audit ALL cleanup paths (cancel, pre-start, reaper) for orphaned artifacts, not just the obvious one

---

## [lms] Restarting the LMS server inside a request handler causes API downtime

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Delete operation times out (20s) and the API becomes unresponsive
- **Root cause**: `lms server stop && lms server start` inside a `run_cmd` (20s tokio timeout) takes the API down during the restart. The subsequent health-check loop exceeds the 20s timeout, aborting the request
- **Fix**: Don't restart the server after delete — `lms ls` rescans the filesystem on each call, so a running server picks up deletions immediately. Remove server restart from the delete flow entirely
- **Prevention**: Never restart a service from within a request handler that has a timeout. If a refresh is needed, prefer the API's own rescan mechanism. Treat server restarts as out-of-band operator actions, not request-handler steps

---

## [tooling] Playwright MCP `browser_snapshot` with no args fails in OpenCode

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: `browser_snapshot` call rejected with `input: Field required` even though all params are optional
- **Root cause**: OpenCode bug #20637 — when an MCP tool has all-optional parameters and the LLM calls it with no args, the harness rejects the empty object as "Field required" instead of passing it through
- **Fix**: Always pass at least one parameter (e.g., `depth`) when calling `browser_snapshot`
- **Prevention**: For MCP tools with all-optional params in OpenCode, never call with an empty args object — always include at least one param. Track OpenCode bug #20637 for the underlying fix

---

## [lms] Quantization selection must use `@quant` syntax, not rely on auto-select

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Downloads complete with the wrong quantization (default Q8) regardless of user's badge selection
- **Root cause**: `startDownload()` sent only the model name to `lms get -y`, which auto-selects a default quant. The `@quant` syntax (`lms get model@q8_0`) that pins a specific quant was never used; the UI's quant badges were display-only
- **Fix**: Make quant badges clickable; append `@<quant>` to the download command (e.g., `lms get qwen3-4b@q8_0`)
- **Prevention**: When a UI element represents a CLI flag/value, trace the selection from click to API to CLI command end-to-end. Display-only controls that don't reach the command are a common UX bug. Verify user selections appear in the final shell command

---

## [lms] Small-context models cannot fit large IDE/agent system prompts

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Zed's Write profile fails with context overflow on small models like qwen3-4b (40K context)
- **Root cause**: Zed's Write profile system prompt is ~80K tokens. Models with 40K context windows cannot fit the prompt plus user input. This is distinct from the parallel-slots context division (see lesson "[lms] Parallel slots divide context window") — here the model's *total* context is too small, not divided
- **Fix**: Use a model with a context window of 200K+ tokens for IDE/agent workloads with large system prompts
- **Prevention**: Before assigning a model to an IDE/agent integration, check the integration's system prompt size against the model's context window. Keep a tiered model list: small models for chat, large-context models for agents. Never assume "it loaded" means "it fits"

---

## [data] HF mirrors cause duplicate simultaneous downloads across publishers

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: Same model (Qwen3-Coder-Next-GGUF) downloaded into two publisher folders (lmstudio-community/ and unsloth/) simultaneously
- **Root cause**: HuggingFace search returns multiple publisher mirrors of the same model as separate cards. `dl_id` hashed on the full name including publisher, so mirrors got distinct IDs. `pkill -f` matched the exact full name, so killing one didn't stop the other. No cross-publisher dedup existed
- **Fix**: Dedup HF results by basename (strip publisher). `normalize_dl_key` strips publisher from the key, so `dl_id` collides across publishers — the second download attempt finds the existing one instead of starting a duplicate
- **Prevention**: For any search source with mirror/copy semantics (HF publishers, GH forks), dedup by normalized basename, not full identifier. Identity keys for dedup must be invariant across cosmetic differences (publisher, casing). When `pkill` is used for process management, ensure the match pattern covers all variants, or kill by tracked PID instead

---

## [process] Symptom-reported bug may have an unrelated root cause

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: User reported "Text Generation filter gives no results"
- **Root cause**: The actual bug was URL parameter preservation — the Sort dropdown link dropped `pipeline_tag` from the URL. The filter itself worked; the navigation destroyed the filter state. Investigation via running the app with `RUST_LOG=debug` was needed to confirm the filter logic was fine and the URL was the culprit
- **Fix**: `applyFilter()` JS function uses `URLSearchParams` to preserve all params (sort, pipeline_tag, search) across navigation
- **Prevention**: When a user reports "X doesn't work," don't assume X's logic is broken. Trace the full request flow: URL construction → navigation → server handler → DB query → response. Reproduce with debug logging before editing the reported component. Symptom and cause are frequently in different layers

---

## [process] Late-discovered cleanup bugs signal incomplete initial audit

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: `.part` file accumulation discovered late, after the dedup fix was already in progress
- **Root cause**: During the dedup fix, `cancel_download` and `download_model` cleanup commands were not exhaustively audited for side-effect cleanup. The `.part` file gap was only found by examining disk usage later
- **Fix**: Pre-start cleanup now deletes `.part` files; reaper cleans tracked downloads' `.part` files on startup
- **Prevention**: When fixing a download/process lifecycle bug, audit ALL related cleanup commands (cancel, pre-start, reaper, error paths) for artifact cleanup in the same pass — not just the one path that triggered the fix. Disk-usage inspection (`du -sh models/*`) is a cheap late-stage check; run it after any download-flow change

---

## [process] Multi-file frontend changes need code review, not just testing

- **Last seen**: 2026-06-28
- **Times seen**: 1
- **Symptom**: `card.id` inconsistency — one code path replaced `@` in the model name, another didn't, producing mismatched IDs between the download trigger and the status poller
- **Root cause**: Two frontend files touched the same `card.id` concept with different normalization. Testing each file in isolation passed; the inconsistency only surfaced in review comparing the two files
- **Fix**: Caught by code review; normalized both paths to use the same replacement
- **Prevention**: For multi-file frontend changes sharing an identifier/contract, always run code review comparing the files side by side — testing each in isolation won't catch cross-file inconsistencies. Define shared normalization in one helper function and call it from both sites, so the contract can't drift
