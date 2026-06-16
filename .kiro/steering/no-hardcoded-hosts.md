---
title:        No Hardcoded Hosts or Credentials
inclusion:    always
version:      1.0
last-updated: 2026-06-18
status:       active
---

# No Hardcoded Hosts or Credentials

## Rule

Never hardcode IP addresses, hostnames, usernames, or passwords in source code. Use environment variables instead.

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `ENV_IP_JUMP_231_HOST` | Remote host IP address |
| `ENV_USER_JUMP_231_HOST` | Remote host SSH username |
| `ENV_PASS_JUMP_231_HOST` | Remote host SSH password |

## Usage Pattern

```python
import os
host = os.environ["ENV_IP_JUMP_231_HOST"]
user = os.environ["ENV_USER_JUMP_231_HOST"]
```

## Violations

- ❌ `REMOTE_HOST = "hts@137.58.231.231"`
- ❌ `ssh hts@137.58.231.231`
- ❌ Any literal IP or username in committed code
- ✅ `os.environ["ENV_IP_JUMP_231_HOST"]`
- ✅ References to env var names in comments/docs are fine
