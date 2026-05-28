# Crunch — Copilot Integration (VS Code Copilot Chat + Copilot CLI)

**Usage**: Token-optimized CLI proxy (60-90% savings on dev operations)

## What's automatic

The `.github/copilot-instructions.md` file is loaded at session start by both Copilot CLI and VS Code Copilot Chat.
It instructs Copilot to prefix commands with `crunch` automatically.

The `.github/hooks/crunch-rewrite.json` hook adds a `PreToolUse` safety net via `crunch hook` —
a cross-platform Rust binary that intercepts raw bash tool calls and rewrites them.
No shell scripts, no `jq` dependency, works on Windows natively.

## Meta commands (always use directly)

```bash
crunch discover          # Scan session history for missed crunch opportunities
```

## Installation verification

```bash
crunch --version   # Should print: crunch X.Y.Z
which crunch       # Verify correct binary path
```

## How the hook works

`crunch hook` reads `PreToolUse` JSON from stdin, detects the agent format, and responds appropriately:

**VS Code Copilot Chat** (supports `updatedInput` — transparent rewrite, no denial):
1. Agent runs `git status` → `crunch hook` intercepts via `PreToolUse`
2. `crunch hook` detects VS Code format (`tool_name`/`tool_input` keys)
3. Returns `hookSpecificOutput.updatedInput.command = "crunch git status"`
4. Agent runs the rewritten command silently — no denial, no retry

**GitHub Copilot CLI** (deny-with-suggestion — CLI ignores `updatedInput` today):
1. Agent runs `git status` → `crunch hook` intercepts via `PreToolUse`
2. `crunch hook` detects Copilot CLI format (`toolName`/`toolArgs` keys)
3. Returns `permissionDecision: deny` with reason: `"Token savings: use 'crunch git status' instead"`
4. Copilot reads the reason and re-runs `crunch git status`

When Copilot CLI adds `updatedInput` support, only `crunch hook` needs updating — no config changes.

## Integration comparison

| Tool                  | Mechanism                               | Hook output              | File                               |
|-----------------------|-----------------------------------------|--------------------------|------------------------------------|
| Claude Code           | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `hooks/crunch-rewrite.sh`          |
| VS Code Copilot Chat  | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `.github/hooks/crunch-rewrite.json`|
| GitHub Copilot CLI    | `PreToolUse` deny-with-suggestion       | Denial + retry           | `.github/hooks/crunch-rewrite.json`|
| OpenCode              | Plugin `tool.execute.before`            | Transparent rewrite      | `hooks/opencode-crunch.ts`         |
| (any)                 | Custom instructions                     | Prompt-level guidance    | `.github/copilot-instructions.md`  |
