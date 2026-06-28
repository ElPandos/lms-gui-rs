#!/usr/bin/env bash
#
# archive-bench-to-vault.sh — Copy a benchmark report to the Obsidian vault
#
# Usage:
#   scripts/archive-bench-to-vault.sh <report.md> [vault-benchmarks-dir]
#
# Defaults:
#   report.md           — required, path to the report written by the profiler
#   vault-benchmarks-dir — defaults to ~/OneDrive/Obsidian/Ericsson/50-ai/benchmarks
#
# The script:
#   1. Extracts the date and target from the report filename (profile-{target}-{date}.md)
#   2. Creates a vault note with Obsidian frontmatter
#   3. Prepends the frontmatter to the report body
#   4. Writes to the vault benchmarks folder
#   5. Prints the vault note path
#
# Example:
#   scripts/archive-bench-to-vault.sh docs/reports/profile-lms-gui-rs-2026-06-28.md
#
set -euo pipefail

REPORT="${1:?Usage: $0 <report.md> [vault-benchmarks-dir]}"
VAULT_DIR="${2:-$HOME/OneDrive/Obsidian/Ericsson/50-ai/benchmarks}"

if [[ ! -f "$REPORT" ]]; then
    echo "ERROR: Report file not found: $REPORT" >&2
    exit 1
fi

if [[ ! -d "$VAULT_DIR" ]]; then
    echo "ERROR: Vault benchmarks directory not found: $VAULT_DIR" >&2
    echo "Create it first: mkdir -p \"$VAULT_DIR\"" >&2
    exit 1
fi

# Extract date and target from filename: profile-{target}-{date}.md
FILENAME=$(basename "$REPORT")
if [[ "$FILENAME" =~ ^profile-(.+)-([0-9]{4}-[0-9]{2}-[0-9]{2})\.md$ ]]; then
    TARGET="${BASH_REMATCH[1]}"
    DATE="${BASH_REMATCH[2]}"
else
    # Fallback: use today's date and the full filename stem
    DATE=$(date +%Y-%m-%d)
    TARGET=$(basename "$REPORT" .md)
fi

# Determine project name from git remote or directory name
PROJECT=$(basename "$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")")

# Vault note name: YYYY-MM-DD--{project}--{target}.md
VAULT_NOTE="${VAULT_DIR}/${DATE}--${PROJECT}--${TARGET}.md"

# Read the report body
REPORT_BODY=$(cat "$REPORT")

# Build Obsidian frontmatter
# Note: the report body already has markdown content; we prepend YAML frontmatter
# so Obsidian Properties can index the benchmark metadata.
FRONTMATTER="---
type: benchmark
domain: rust
project: ${PROJECT}
created: ${DATE}
target: ${TARGET}
profiler: cargo-test-micro-bench
source: docs/reports/${FILENAME}
tags:
  - rust/performance
  - rust/benchmarking
summary: \"Benchmark run for ${PROJECT} (${TARGET}) on ${DATE}\"
---

# Benchmark: ${PROJECT} — ${DATE}

> [!info] Full report
> Project path: \`docs/reports/${FILENAME}\`
"

# Write the vault note
echo "${FRONTMATTER}" > "$VAULT_NOTE"
echo "" >> "$VAULT_NOTE"
echo "${REPORT_BODY}" >> "$VAULT_NOTE"

echo "Archived benchmark report to vault:"
echo "  ${VAULT_NOTE}"
echo ""
echo "Update the index: ${VAULT_DIR}/_index.md"
