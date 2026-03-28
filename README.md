# Crunch

Privacy-respecting, mise-aware output compression proxy for AI coding assistants.

Crunch intercepts tool commands (git, cargo, pytest, ruff, etc.) via hooks, compresses verbose output into concise summaries for your LLM, and saves the raw output to disk so nothing is ever lost. Typical token savings: 60-90%.

Fork of [RTK](https://github.com/rtk-ai/rtk) (Apache-2.0) with telemetry stripped, mise integration added, and log-first architecture.

## Quick Start

```bash
# 1. Install
cargo install --path .

# 2. Set up Claude Code hook
crunch init -g

# 3. Use Claude Code normally — crunch works automatically
#    "git status" becomes "crunch git status" behind the scenes
```

## How It Works

```
+-------------------------------------------------+
|  AI Assistant (PreToolUse hook)                  |
|  "pytest -x" --> "crunch pytest -x"             |
+-------------------------------------------------+
|  Crunch CLI                                     |
|  +----------+  +----------+  +-----------+      |
|  | Router   |->| Executor |->| Parser    |      |
|  | (clap)   |  | (mise or |  | (per-tool |      |
|  |          |  |  direct) |  |  filter)  |      |
|  +----------+  +----+-----+  +-----+-----+      |
|                     |              |             |
|              +------v------+ +-----v------+     |
|              | Tee (log)   | | Summary    |     |
|              | always-on   | | to stdout  |     |
|              +-------------+ +------------+     |
+-------------------------------------------------+
|  Filesystem                                     |
|  /tmp/crunch/<project>/<tool>-<scope>-<ts>.log  |
+-------------------------------------------------+
```

1. The AI assistant runs a command (e.g., `pytest -x`)
2. A PreToolUse hook rewrites it to `crunch pytest -x`
3. Crunch checks for a mise mapping -- if found, routes through `mise run <task> -- <args>`
4. Raw output is saved to `/tmp/crunch/{project}/` (log-first)
5. A per-tool parser compresses the output into a concise summary
6. The summary is returned to the LLM; on failure, the log path is included so the model can read the full output

## Installation

### Build from source

```bash
# Install Rust toolchain (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/yourusername/crunch.git
cd crunch
cargo build --release

# Install to ~/.cargo/bin/
cargo install --path .
```

The release binary is optimized (LTO, stripped) and typically under 5MB.

### Verify installation

```bash
crunch --version
```

## Setup

### Claude Code

```bash
crunch init -g
```

This does two things:
- Writes a hook script to `~/.claude/hooks/crunch-rewrite.sh`
- Patches `~/.claude/settings.json` to register the PreToolUse hook

The hook intercepts Bash tool calls and rewrites known commands:

```
git status   -->  crunch git status
pytest -x    -->  crunch pytest -x
ls -la       -->  crunch ls -la
unknown      -->  unchanged
```

### Cursor

```bash
crunch init -g --agent cursor
```

### Windsurf

```bash
crunch init -g --agent windsurf
```

### Cline

```bash
crunch init -g --agent cline
```

### OpenCode

```bash
crunch init -g --opencode
```

### Other agents

Crunch ships hooks for Gemini, Copilot, and Codex as well. See `crunch init --help` for all options.

## Mise Integration

This is the key feature that distinguishes Crunch from vanilla RTK. If your team uses [mise](https://mise.jdx.dev/) as a task runner, Crunch can route tool commands through mise tasks so the AI assistant respects your project's task definitions.

### Config

Add a `[mise]` section to your config mapping tool names to mise task names:

```toml
# .crunch.toml (per-project) or ~/.config/crunch/config.toml (global)
[mise]
pytest = "test"       # crunch pytest --> mise run test
ruff = "lint"         # crunch ruff --> mise run lint
mypy = "typecheck"    # crunch mypy --> mise run typecheck
```

### Example workflow

When the AI assistant runs `pytest -x --tb=short tests/test_build.py`:

```
1. Hook rewrites:  crunch pytest -x --tb=short tests/test_build.py
2. Crunch routes:  mise run test -- -x --tb=short tests/test_build.py
3. Mise executes:  uv run pytest -x --tb=short tests/test_build.py
4. Output flow:    raw --> tee to log --> pytest parser --> compressed summary --> model
```

Flags after `--` pass through to the underlying tool. Your teammates who run `mise run test` directly are unaffected -- Crunch is invisible to them.

### Fallback behavior

If no `[mise]` mapping exists for a tool, Crunch executes it directly (the same behavior as vanilla RTK). A project with no `.crunch.toml` behaves identically to RTK.

## Configuration

Crunch uses a two-level config system with key-by-key merging:

1. **Global**: `~/.config/crunch/config.toml` -- defaults for all projects
2. **Per-project**: `.crunch.toml` in the project root -- overrides specific keys

Per-project settings merge over global ones without replacing entire sections.

### Global config example

```toml
# ~/.config/crunch/config.toml

[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"

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

### Per-project config example

```toml
# .crunch.toml (in project root)

[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"

[tee.overrides]
cargo = { enabled = true }
```

### Tee logging config

Every command's raw output is saved to disk before parsing. The compressed summary is never the only copy.

**Log path template:**
```
/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log
```

- **project** -- basename of git root (or cwd if not in a repo)
- **tool** -- command being run (pytest, ruff, git-diff, etc.)
- **scope** -- derived from file args or `-k` filters; falls back to `all`
- **timestamp** -- `YYYYMMDD-HHMMSS`

**When the log path appears in output:**

| Outcome | Summary example | Log path shown? |
|---------|----------------|-----------------|
| Clean pass | `Pytest: 12 passed` | No |
| Warnings | `Ruff: 3 warnings (2 fixable)` | Yes |
| Failures | `Pytest: 10 passed, 2 failed` | Yes |
| Crash/error | `Mypy: exited 2` | Yes |

Tee defaults to **on** for all tools. Disable per-tool via `[tee.overrides]`:

```toml
[tee.overrides]
git = { enabled = false }    # already in git history
ls = { enabled = false }     # not worth logging
```

Logs are cleaned on `/tmp` reboot cycle (OS handles it), or manually with `crunch logs clean`.

## Supported Tools

| Tool | Subcommands / Modes | Typical Savings |
|------|---------------------|-----------------|
| git | log, diff, status, show, branch, push, pull, commit, add, fetch, stash | 60-80% |
| cargo | test, build, clippy, check | 90%+ |
| pytest | all modes | 90%+ |
| ruff | check, format | 70-85% |
| mypy | all modes | 70-85% |
| go | test, build, vet | 80-90% |
| grep | code search (groups by file, truncates) | 50-70% |
| ls | directory listing (filters noise dirs) | 50-70% |
| find | file finding (gitignore-aware) | 50-70% |
| read | file reading (filter-level stripping) | varies |
| docker | container operations (compact tables) | 60-80% |
| kubectl | cluster operations (compact tables) | 60-80% |
| npm / pnpm | install, build, test | 70-85% |
| vitest | test runner | 80-90% |
| playwright | test runner | 80-90% |
| tsc | type checking | 70-85% |
| next | Next.js build/dev | 70-85% |
| prettier | formatting | 60-80% |
| rspec | Ruby test runner | 80-90% |
| rubocop | Ruby linter | 70-85% |
| rake | Ruby task runner | 70-85% |
| prisma | ORM operations | 60-80% |
| pip | package management | 60-80% |
| dotnet | build, test, format | 70-85% |
| golangci-lint | Go linter | 70-85% |
| gh | GitHub CLI | 50-70% |
| aws | AWS CLI | 60-80% |
| psql | PostgreSQL | 50-70% |
| curl / wget | HTTP requests | 50-70% |
| log | log deduplication | 80-90% |
| diff | file diff | 60-80% |
| json | JSON formatting | 50-70% |

71 parser modules total. Unknown commands pass through unmodified (still tee'd to disk).

## Token Savings

Check cumulative token savings with:

```bash
crunch gain
```

This reports how many tokens Crunch has saved across your sessions, using local SQLite tracking (no data leaves your machine).

## Performance

Crunch adds minimal overhead to fast commands and actually speeds up heavy ones by reducing the amount of text the LLM needs to process:

| Command | Overhead | Token Savings |
|---------|----------|---------------|
| `git status` | +10ms | 60-80% |
| `git log` | +3ms | 80% |
| `cargo test` | net faster (compressed output) | 90%+ |
| `ls` / `grep` | +4-5ms | 50-70% |

Design constraints: no async runtime (no tokio), `lazy_static!` for all regex compilation, <10ms startup target.

## FAQ

### How is this different from RTK?

Crunch is a fork of [RTK](https://github.com/rtk-ai/rtk) with three changes:

1. **Telemetry removed** -- RTK phones home with usage data. Crunch strips all network telemetry (the `telemetry.rs` module and its `ureq`/`hostname` dependencies). Local-only analytics (token counting, gain reporting) are kept.
2. **Mise integration added** -- The `[mise]` config section routes project tools through mise tasks, so the AI assistant respects your task runner setup.
3. **Log-first architecture** -- Raw output is always saved to `/tmp/crunch/` before parsing. On failure, the summary includes the log path so the model can read the full output.

All 71 RTK parser modules are kept untouched.

### Is any data sent anywhere?

No. Crunch makes zero network calls. All analytics (token counting, gain stats) are stored in a local SQLite database. The telemetry module from RTK has been completely removed.

### Can I use this without mise?

Yes. Without a `[mise]` section in your config, Crunch behaves identically to RTK -- tools are executed directly and output is compressed by the per-tool parsers. Mise integration is purely opt-in.

### What happens if a parser fails?

Crunch falls back to raw command output. Parsers are designed to degrade gracefully -- a bug in the pytest parser will never prevent you from seeing pytest output.

### Does Crunch affect exit codes?

No. Crunch always preserves the underlying tool's exit code, so CI/CD pipelines and scripts that depend on exit codes work correctly.

### Can I exclude commands from being rewritten?

Yes. Add them to `[hooks].exclude_commands` in your config:

```toml
[hooks]
exclude_commands = ["read", "cat", "curl"]
```

`read` and `cat` are excluded by default because aggressive read filtering can strip function bodies, which is harmful for a coding assistant.

## License

Apache-2.0. Forked from [RTK](https://github.com/rtk-ai/rtk) by [rtk-ai](https://github.com/rtk-ai).
