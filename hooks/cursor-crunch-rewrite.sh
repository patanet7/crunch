#!/usr/bin/env bash
# crunch-hook-version: 1
# Crunch Cursor Agent hook — rewrites shell commands to use crunch for token savings.
# Works with both Cursor editor and cursor-cli (they share ~/.cursor/hooks.json).
# Cursor preToolUse hook format: receives JSON on stdin, returns JSON on stdout.
# Requires: crunch >= 0.23.0, jq
#
# This is a thin delegating hook: all rewrite logic lives in `crunch rewrite`,
# which is the single source of truth (src/discover/registry.rs).
# To add or change rewrite rules, edit the Rust registry — not this file.

if ! command -v jq &>/dev/null; then
  echo "[crunch] WARNING: jq is not installed. Hook cannot rewrite commands. Install jq: https://jqlang.github.io/jq/download/" >&2
  exit 0
fi

if ! command -v crunch &>/dev/null; then
  echo "[crunch] WARNING: crunch is not installed or not in PATH. Hook cannot rewrite commands." >&2
  exit 0
fi

# Version guard: crunch rewrite was added in 0.23.0.
CRUNCH_VERSION=$(crunch --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
if [ -n "$CRUNCH_VERSION" ]; then
  MAJOR=$(echo "$CRUNCH_VERSION" | cut -d. -f1)
  MINOR=$(echo "$CRUNCH_VERSION" | cut -d. -f2)
  if [ "$MAJOR" -eq 0 ] && [ "$MINOR" -lt 23 ]; then
    echo "[crunch] WARNING: crunch $CRUNCH_VERSION is too old (need >= 0.23.0). Upgrade: cargo install crunch" >&2
    exit 0
  fi
fi

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

if [ -z "$CMD" ]; then
  echo '{}'
  exit 0
fi

# Delegate all rewrite logic to the Rust binary.
# crunch rewrite exits 1 when there's no rewrite — hook passes through silently.
REWRITTEN=$(crunch rewrite "$CMD" 2>/dev/null) || { echo '{}'; exit 0; }

# No change — nothing to do.
if [ "$CMD" = "$REWRITTEN" ]; then
  echo '{}'
  exit 0
fi

jq -n --arg cmd "$REWRITTEN" '{
  "permission": "allow",
  "updated_input": { "command": $cmd }
}'
