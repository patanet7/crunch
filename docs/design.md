# Crunch — Design Spec

> A privacy-respecting, mise-aware output compression proxy for AI coding assistants.
> Fork of [RTK](https://github.com/rtk-ai/rtk) (Apache-2.0) with telemetry removed,
> log-first architecture, and mise task runner integration.

## Problem

LLM coding assistants consume large amounts of context window on verbose tool output
(test results, lint reports, git diffs, container logs). RTK solves this with per-tool
parsers that compress output, but it:

- Phones home with telemetry and Datadog traces
- Tracks token usage in a local SQLite database
- Has no awareness of task runners like mise
- Treats compressed output as the only copy (lossy)

## Solution

**Crunch** is a fork of RTK that:

1. Strips all telemetry and token tracking
2. Adds mise integration — project tools route through mise, other engineers use mise directly
3. Implements log-first architecture — raw output always saved to disk, model gets compressed summary with log path on failure/warning
4. Keeps all 71 RTK parsers and the multi-agent init system intact

## Architecture

```
┌─────────────────────────────────────────────┐
│  Claude Code (PreToolUse hook)              │
│  "pytest -x" → "crunch pytest -x"          │
├─────────────────────────────────────────────┤
│  Crunch CLI                                 │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐ │
│  │ Router   │→ │ Executor │→ │ Parser    │ │
│  │ (clap)   │  │ (mise or │  │ (per-tool │ │
│  │          │  │  direct) │  │  filter)  │ │
│  └──────────┘  └────┬─────┘  └─────┬─────┘ │
│                     │              │        │
│              ┌──────▼──────┐ ┌─────▼──────┐ │
│              │ Tee (log)   │ │ Summary    │ │
│              │ always-on   │ │ to stdout  │ │
│              └─────────────┘ └────────────┘ │
├─────────────────────────────────────────────┤
│  Filesystem                                 │
│  /tmp/crunch/<project>/<tool>-<scope>-<ts>  │
└─────────────────────────────────────────────┘
```

### Routing Rules

- **Project tools** (pytest, ruff, mypy, pyright): Check `[mise]` config mapping → execute via `mise run <task> -- [args]` → parse with tool-specific parser
- **System tools** (git, grep, ls, find, docker, kubectl): Execute directly → parse with tool-specific parser (RTK's existing behavior)
- **Unknown commands**: Pass through unmodified, no parsing, still tee'd

### Data Flow Example

```
Model runs:    pytest -x --tb=short tests/test_build.py
Hook rewrites: crunch pytest -x --tb=short tests/test_build.py
Crunch:        mise run test -- -x --tb=short tests/test_build.py
Mise:          uv run pytest -x --tb=short tests/test_build.py
Output:        raw → tee to log → pytest parser → compressed summary → model
```

## Mise Integration

Config-driven tool→task mapping. Crunch intercepts the tool name, routes execution
through mise, and parses the output with the appropriate parser.

### Config

```toml
[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"
pyright = "pyright"
```

### Execution

When crunch sees `crunch pytest -x -q tests/test_build.py`:

1. Router matches `pytest` → checks `[mise]` config → finds task `"test"`
2. Executor runs: `mise run test -- -x -q tests/test_build.py`
3. Flags after `--` pass through to the underlying tool
4. Raw output teed to `/tmp/crunch/finAngent/pytest-test_build-20260327-143012.log`
5. Pytest parser processes output → compressed summary to stdout
6. On failure/warning: summary includes log path

### Fallback

If no `[mise]` mapping exists for a tool, crunch executes it directly (RTK's default
behavior). A Rust project with no `.crunch.toml` behaves identically to vanilla RTK.

### Flag-to-Task Intelligence

Start simple — flags pass through to mise via `--`. Smarter flag→task mapping
(e.g., detecting `-v` and routing to `test-all` instead of `test`) will be added
incrementally as real usage reveals the need.

## Tee & Logging (Log-First Architecture)

Every command crunch executes gets raw output saved to disk before parsing.
The compressed summary is never the only copy.

### Log Path Template

```
/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log
```

- **project**: basename of git root, or cwd basename if not in a git repo
- **tool**: the command being run (pytest, ruff, git-diff, etc.)
- **scope**: derived from file args or `-k` filters; falls back to `all`
- **timestamp**: `YYYYMMDD-HHMMSS`

Examples:
```
/tmp/crunch/finAngent/pytest-all-20260327-143012.log
/tmp/crunch/finAngent/pytest-test_build-20260327-143015.log
/tmp/crunch/finAngent/ruff-check-20260327-143020.log
/tmp/crunch/finAngent/git-diff-20260327-143025.log
```

### Summary Output Behavior

| Outcome | Summary | Log path shown? |
|---------|---------|-----------------|
| Clean pass | `Pytest: 12 passed` | No |
| Warnings | `Ruff: 3 warnings (2 fixable)` | Yes |
| Failures | `Pytest: 10 passed, 2 failed` | Yes |
| Crash/error | `Mypy: exited 2` | Yes |

### Log Lifecycle

- Created on every crunch execution (where tee is enabled for that tool)
- Cleaned on `/tmp` reboot cycle (OS handles it)
- Optional: `crunch logs clean` to manually wipe a project's logs
- No retention database — just files

### Tee Config

Per-tool granularity, config-driven:

```toml
[tee]
enabled = true
path = "/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log"

[tee.overrides]
git = { enabled = false }    # don't log git output (already in git)
ls = { enabled = false }     # don't bother logging ls
pytest = { enabled = true }  # always log (default)
```

Tee defaults to **on** for all tools. Opt out of things you don't need, not opt in.

## What Gets Stripped from RTK

### Files to Delete

| File | Reason |
|------|--------|
| `telemetry.rs` | Phones home to RTK servers |
| `tracking.rs` | SQLite token counting database |
| `local_llm.rs` | Local LLM inference |
| `cc_economics.rs`, `ccusage.rs` | Token cost calculators |
| `gain.rs` | "How many tokens did I save" reporting |
| `session_cmd.rs` | Session management |
| `discover/` | Project introspection module |
| `learn/` | Learning/reporting module |
| `integrity.rs`, `verify_cmd.rs` | RTK self-verification |
| `hook_audit_cmd.rs` | Multi-tool hook auditing |

### Files to Modify

| File | Changes |
|------|---------|
| `main.rs` | Remove enum variants for deleted modules, remove telemetry init |
| `config.rs` | Remove `TelemetryConfig`, `TrackingConfig`; add `MiseConfig`, `TeeConfig` with overrides |
| `tee.rs` | Project-scoped logging, always-on default, path template, scope detection |
| `Cargo.toml` | Remove unused deps, rename package to `crunch` |

### Files to Keep Untouched

- All 71 parser modules (git.rs, pytest_cmd.rs, ruff_cmd.rs, grep_cmd.rs, ls.rs, container.rs, etc.)
- `filter.rs` — language-aware code filtering engine
- `utils.rs`, `display_helpers.rs` — shared utilities
- `parser/` — error parsing infrastructure
- `runner.rs` — command execution
- `init.rs` — keep all agent init paths (Claude Code, Cursor, Gemini, etc.)
- `hook_cmd.rs`, `rewrite_cmd.rs` — keep hook system as optional

### Files to Add

| File | Purpose |
|------|---------|
| `mise_cmd.rs` | Mise routing: config lookup, tool→task dispatch, flag passthrough |

## Claude Code Integration

### PreToolUse Hook

Hook file `~/.claude/hooks/crunch-rewrite.sh` intercepts Bash tool calls and
rewrites known commands to crunch equivalents:

```
"git status"  → "crunch git status"
"pytest -x"   → "crunch pytest -x"
"ls -la"      → "crunch ls -la"
unknown       → unchanged
```

### Settings.json Entry

```json
"PreToolUse": [
  {
    "matcher": "Bash",
    "hooks": [
      {
        "type": "command",
        "command": "bash ~/.claude/hooks/crunch-rewrite.sh"
      }
    ]
  }
]
```

### Setup

```bash
crunch init -g   # generates hook file, patches settings.json
```

### Default Exclusions

```toml
[hooks]
exclude_commands = ["read", "cat"]
```

File reading should not be compressed by default — aggressive read filtering
strips function bodies, which is harmful for a coding assistant.

### Coexistence

- PreToolUse (crunch) fires before execution
- PostToolUse (semgrep-scan.sh) fires after execution
- No conflict — different lifecycle events

## Config

### Global: `~/.config/crunch/config.toml`

```toml
[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"
pyright = "pyright"

[tee]
enabled = true
path = "/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log"

[tee.overrides]
git = { enabled = false }
ls = { enabled = false }

[hooks]
exclude_commands = ["read", "cat"]

[filters]
ignore_dirs = [".git", "node_modules", "__pycache__", ".venv", "target", ".pytest_cache", ".mypy_cache"]
ignore_files = ["*.lock", "*.min.js", "*.min.css"]

[display]
colors = true
max_width = 120
```

### Per-project: `.crunch.toml`

Optional file in project root. Merges key-by-key over global config — project
settings override or extend global ones without replacing entire sections.
Allows different projects to have different mise mappings and tee settings.

```toml
# finAngent/.crunch.toml
[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"
pyright = "pyright"
```

No `.crunch.toml` means no mise routing — crunch behaves like vanilla RTK.

## Repo Strategy

- **Private fork** under personal GitHub account
- Renamed to `crunch`
- Apache-2.0 license preserved (RTK attribution maintained)
- Can be made public later if desired

## Success Criteria

1. `crunch pytest` routes through mise and returns compressed output
2. Raw output always available at predictable log path
3. Model can `Read` log path for full output when summary isn't enough
4. Zero telemetry — no network calls, no tracking database
5. All existing RTK parsers work unchanged (git, grep, ls, docker, etc.)
6. Other engineers on the project use `mise run test` directly — crunch is invisible to them
7. `crunch init -g` sets up Claude Code hook in one command
