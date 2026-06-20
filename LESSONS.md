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
