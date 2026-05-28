#!/usr/bin/env bash
# crunch-hook-version: 4
# Crunch Claude Code hook — rewrites commands to use crunch for token savings.
# Requires: crunch, jq
#
# This is a thin delegating hook: all rewrite logic lives in `crunch rewrite`,
# which is the single source of truth (src/discover/registry.rs).
# To add or change rewrite rules, edit the Rust registry — not this file.
#
# THINKING-SAFETY: when extended/interleaved thinking is active, rewriting the
# tool_use input via updatedInput corrupts the signed thinking block when the
# turn is replayed (Claude Code >= 2.1.152), producing a fatal
#   400 ... `thinking` blocks ... cannot be modified
# that bricks the session. So the hook MUST pass through unchanged (no rewrite)
# on any turn that may carry a thinking block. See crunch_thinking_active below.
#
# Exit code protocol for `crunch rewrite`:
#   0 + stdout  Rewrite found, no deny/ask rule matched → auto-allow
#   1           No Crunch equivalent → pass through unchanged
#   2           Deny rule matched → pass through (Claude Code native deny handles it)
#   3 + stdout  Ask rule matched → rewrite but let Claude Code prompt the user

if ! command -v jq &>/dev/null; then
  echo "[crunch] WARNING: jq is not installed. Hook cannot rewrite commands. Install jq: https://jqlang.github.io/jq/download/" >&2
  exit 0
fi

if ! command -v crunch &>/dev/null; then
  echo "[crunch] WARNING: crunch is not installed or not in PATH. Hook cannot rewrite commands." >&2
  exit 0
fi

# Verify crunch rewrite subcommand is available.
if ! crunch rewrite --help &>/dev/null; then
  echo "[crunch] WARNING: 'crunch rewrite' not available. Upgrade crunch." >&2
  exit 0
fi

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

if [ -z "$CMD" ]; then
  exit 0
fi

# ── Thinking-safety guard ─────────────────────────────────────────────
# Skip the rewrite entirely (pass through unchanged) whenever the current turn
# may carry a signed thinking block. Rewriting tool_use input on such a turn
# triggers the unrecoverable "thinking blocks cannot be modified" 400.
EFFORT_LEVEL=$(echo "$INPUT" | jq -r '.effort.level // empty')
TRANSCRIPT_PATH=$(echo "$INPUT" | jq -r '.transcript_path // empty')

crunch_thinking_active() {
  # Primary signal (O(1)): medium/high (and any higher) effort enable
  # (interleaved) thinking on current Opus models, so any tool_use on these turns
  # is unsafe to rewrite. Treat ANY non-empty level that isn't explicitly "off"
  # as thinking-on, so future/unknown level names fail safe (skip the rewrite).
  case "$EFFORT_LEVEL" in
    "" | none | off | low | minimal) : ;; # thinking off → fall through to transcript check
    *) return 0 ;;                          # medium/high/unknown → assume thinking on
  esac
  # Secondary signal: the transcript's recent history already contains a thinking
  # block (covers configs where effort is absent). Bounded tail read only — the
  # transcript can be hundreds of MB, so never scan the whole file.
  if [ -n "$TRANSCRIPT_PATH" ] && [ -f "$TRANSCRIPT_PATH" ]; then
    if tail -c 131072 "$TRANSCRIPT_PATH" 2>/dev/null |
      grep -q '"type":"thinking"\|"type":"redacted_thinking"'; then
      return 0
    fi
  fi
  return 1
}

if crunch_thinking_active; then
  # Thinking turn — do not rewrite. The command runs unmodified.
  exit 0
fi

# Delegate all rewrite + permission logic to the Rust binary.
REWRITTEN=$(crunch rewrite "$CMD" 2>/dev/null)
EXIT_CODE=$?

case $EXIT_CODE in
  0)
    # Rewrite found, no permission rules matched — safe to auto-allow.
    # If the output is identical, the command was already using Crunch.
    [ "$CMD" = "$REWRITTEN" ] && exit 0
    ;;
  1)
    # No Crunch equivalent — pass through unchanged.
    exit 0
    ;;
  2)
    # Deny rule matched — let Claude Code's native deny rule handle it.
    exit 0
    ;;
  3)
    # Ask rule matched — rewrite the command but do NOT auto-allow so that
    # Claude Code prompts the user for confirmation.
    ;;
  *)
    exit 0
    ;;
esac

ORIGINAL_INPUT=$(echo "$INPUT" | jq -c '.tool_input')
UPDATED_INPUT=$(echo "$ORIGINAL_INPUT" | jq --arg cmd "$REWRITTEN" '.command = $cmd')

if [ "$EXIT_CODE" -eq 3 ]; then
  # Ask: rewrite the command, omit permissionDecision so Claude Code prompts.
  jq -n \
    --argjson updated "$UPDATED_INPUT" \
    '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": $updated
      }
    }'
else
  # Allow: rewrite the command and auto-allow.
  jq -n \
    --argjson updated "$UPDATED_INPUT" \
    '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "Crunch auto-rewrite",
        "updatedInput": $updated
      }
    }'
fi
