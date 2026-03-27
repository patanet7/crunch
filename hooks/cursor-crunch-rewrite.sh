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

# Delegate all rewrite + permission logic to the Rust binary.
#
# Exit code protocol for `crunch rewrite`:
#   0 + stdout  Rewrite found, no deny/ask rule matched → auto-allow
#   1           No crunch equivalent → pass through unchanged
#   2           Deny rule matched → block the command
#   3 + stdout  Ask rule matched → rewrite but prompt user
REWRITTEN=$(crunch rewrite "$CMD" 2>/dev/null)
EXIT_CODE=$?

case $EXIT_CODE in
  0)
    # Rewrite found — auto-allow (handled below)
    ;;
  1)
    # No crunch equivalent — pass through unchanged
    echo '{}'
    exit 0
    ;;
  2)
    # Deny rule matched — emit error, pass through (Cursor has no native deny)
    echo "[crunch] Command denied by permission rule: $CMD" >&2
    echo '{}'
    exit 0
    ;;
  3)
    # Ask rule matched — rewrite but do NOT auto-allow
    # Cursor has no native "ask" mechanism, so we warn and allow
    echo "[crunch] Command requires confirmation (ask rule): $CMD → $REWRITTEN" >&2
    ;;
  *)
    echo '{}'
    exit 0
    ;;
esac

# No change — nothing to do.
if [ "$CMD" = "$REWRITTEN" ]; then
  echo '{}'
  exit 0
fi

jq -n --arg cmd "$REWRITTEN" '{
  "permission": "allow",
  "updated_input": { "command": $cmd }
}'
