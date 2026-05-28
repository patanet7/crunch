#!/usr/bin/env bash
# End-to-end test for the v4 thinking-safety hook guard.
#
# Proves, against the REAL Claude Code CLI + API, that installing the crunch v4
# rewrite hook does NOT trigger the
#     400 ... `thinking` blocks ... cannot be modified
# failure when interleaved/extended thinking is enabled.
#
# It runs a headless `claude -p` session with:
#   - extended thinking ON (--effort high)
#   - the crunch v4 hook wired into an ISOLATED --settings file
#     (your real ~/.claude config is never touched)
#   - a prompt that makes the model issue several rewritable Bash tool calls
# then asserts the session completed with no thinking-block 400.
#
# YOU run this (it spawns a headless agent with --dangerously-skip-permissions
# and consumes API tokens). The script itself bypasses nothing on your behalf.
#
# Usage:
#   bash scripts/e2e-thinking-hook.sh              # positive test: v4 hook -> no 400
#   bash scripts/e2e-thinking-hook.sh --negative   # also run a v3-style hook
#                                                   # (guard removed) to try to
#                                                   # reproduce the 400 (flaky)
#
# Env overrides: CLAUDE_BIN, MODEL (default claude-opus-4-8), EFFORT (default high)
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CLAUDE="${CLAUDE_BIN:-$HOME/.local/bin/claude}"   # real CLI, not the wezcld alias
MODEL="${MODEL:-claude-opus-4-8}"
EFFORT="${EFFORT:-high}"
HOOK_V4="$REPO/hooks/crunch-rewrite.sh"

[ -x "$CLAUDE" ] || { echo "FATAL: real claude CLI not found at $CLAUDE (set CLAUDE_BIN)"; exit 2; }
command -v jq  >/dev/null || { echo "FATAL: jq required"; exit 2; }
command -v git >/dev/null || { echo "FATAL: git required"; exit 2; }
grep -q 'crunch-hook-version: 4' "$HOOK_V4" || {
  echo "FATAL: $HOOK_V4 is not v4 — rebuild/checkout the thinking-guard hook first"; exit 2; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
MARKER="$WORK/hook_fired.log"

# Build an isolated settings.json whose PreToolUse hook wraps the given hook
# script with a firing-marker (so we can tell "hook never ran" from "ran + guard
# worked").
make_settings() { # $1 = hook script to wrap
  local wrapper="$WORK/hook_wrapper.sh"
  cat > "$wrapper" <<EOF
#!/usr/bin/env bash
echo "fired \$(date +%s)" >> "$MARKER"
exec "$1"
EOF
  chmod +x "$wrapper"
  cat > "$WORK/settings.json" <<EOF
{
  "permissions": { "allow": ["Bash(*)"], "defaultMode": "bypassPermissions" },
  "hooks": { "PreToolUse": [ { "matcher": "Bash",
    "hooks": [ { "type": "command", "command": "$wrapper" } ] } ] }
}
EOF
}

run_session() { # $1 = label  $2 = hook script
  make_settings "$2"
  : > "$MARKER"
  local proj="$WORK/proj"
  rm -rf "$proj"; mkdir -p "$proj"
  ( cd "$proj" && git init -q && git commit --allow-empty -qm "init" \
      && git commit --allow-empty -qm "second" )
  # A prompt that reliably yields several *bare, rewritable* Bash tool calls.
  local prompt="Do not explain anything. Using the Bash tool, run each of these as a separate command, then stop: git status --short ; git log --oneline -5 ; git branch --show-current ; git diff --stat HEAD~1"
  local out code
  out="$( cd "$proj" && "$CLAUDE" -p "$prompt" \
            --model "$MODEL" --effort "$EFFORT" \
            --settings "$WORK/settings.json" \
            --dangerously-skip-permissions 2>&1 )"
  code=$?
  printf '%s\n' "$out" | sed 's/^/    /'
  echo "    ----"
  if printf '%s' "$out" | grep -qi 'cannot be modified'; then
    echo ">> RESULT[$1]: thinking-block 400 PRESENT (exit $code)"
    return 1
  fi
  if [ -s "$MARKER" ]; then
    echo ">> RESULT[$1]: hook fired $(wc -l < "$MARKER" | tr -d ' ') time(s), NO thinking 400 (exit $code)"
  else
    echo ">> RESULT[$1]: WARNING — hook never fired; test INCONCLUSIVE (exit $code)"
    return 2
  fi
  return 0
}

echo "=============================================================="
echo " e2e: crunch v4 thinking-safety hook"
echo " model=$MODEL effort=$EFFORT"
echo "=============================================================="
echo
echo "### POSITIVE — v4 hook (guard active) — expect NO 400"
run_session "v4" "$HOOK_V4"; pos=$?
echo

if [ "${1:-}" = "--negative" ]; then
  # Derive a v3-style hook by stripping the guard block; best-effort attempt to
  # reproduce the 400. NOTE: reproduction is non-deterministic — it needs the
  # model to emit a thinking block plus a rewritten tool_use as the latest
  # message — so a "no 400" here does NOT mean the bug is gone.
  V3="$WORK/hook_v3.sh"
  awk '/Thinking-safety guard/{skip=1} /Delegate all rewrite/{skip=0} !skip' "$HOOK_V4" > "$V3"
  chmod +x "$V3"
  echo "### NEGATIVE — v3-style hook (guard removed) — may reproduce the 400 (flaky)"
  run_session "v3" "$V3" || true
  echo
fi

echo "=============================================================="
if [ "$pos" -eq 0 ]; then
  echo " PASS: v4 hook ran in a real thinking session with NO thinking-block 400."
elif [ "$pos" -eq 2 ]; then
  echo " INCONCLUSIVE: hook did not fire — check headless hook support / settings."
else
  echo " FAIL: v4 hook session hit a thinking-block 400."
fi
echo "=============================================================="
exit "$pos"
