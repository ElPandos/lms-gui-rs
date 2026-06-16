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
