---
name: configure-crunch
description: Use when setting up crunch for a new project, adding .crunch.toml, configuring mise task mappings, customizing tee logging, adjusting per-project ignore directories, or setting up env-wrapper auto-routing for virtualenv commands
---

# Configure Crunch

Set up `.crunch.toml` per-project config for the crunch output compression proxy.

## When to Use

- User says "set up crunch", "configure crunch", "add crunch config"
- Starting work in a new project that uses crunch
- Need to add/change mise task mappings
- Need project-specific ignore dirs or tee settings

## Steps

1. **Detect project type** — check for `Cargo.toml`, `pyproject.toml`, `package.json`, `go.mod`, etc.
2. **Check mise tasks** — run `mise tasks` to see what's defined (don't assume)
3. **Check existing config** — read `.crunch.toml` if it exists, also `~/.config/crunch/config.toml`
4. **Write `.crunch.toml`** with only the sections that differ from defaults

## Config Sections Quick Reference

| Section | Purpose | Where it lives |
|---------|---------|----------------|
| `[mise]` | Map tool names to mise tasks | `.crunch.toml` or global |
| `[tee]` | Log directory, mode, limits | `.crunch.toml` or global |
| `[tee.overrides]` | Disable tee per-tool | `.crunch.toml` or global |
| `[filters]` | ignore_dirs, ignore_files | `.crunch.toml` or global |
| `[hooks]` | exclude_commands | `.crunch.toml` or global |
| `[display]` | colors, max_width | `.crunch.toml` or global |
| `[env]` | Env-wrapper auto-routing | **Global only** (`~/.config/crunch/config.toml`) |

**IMPORTANT:** `[env]` is blocked from project `.crunch.toml` for security — a malicious repo could inject shell commands via the wrapper field. Always configure `[env]` in `~/.config/crunch/config.toml`.

## Env-Wrapper Auto-Routing (`[env]`)

Auto-prepends an env manager (uv, poetry, etc.) to bare commands so they run in the correct virtualenv.

**Must be configured in `~/.config/crunch/config.toml`** (not `.crunch.toml`).

```toml
[env]
wrapper = "uv run"
wrap_commands = ["python", "python3", "pip", "pip3"]
```

- `wrapper` — must be an allowed value: `uv run`, `poetry run`, `pipx run`, `pdm run`, `conda run`, `nix run`, `mise run`
- `wrap_commands` — must be known tools (Python/Node/Ruby ecosystem). Arbitrary binaries like `ssh`, `git` are rejected for security.
- Both must be set for the feature to activate.

### How it works

| What AI model runs | What actually executes | Why |
|--------------------|----------------------|-----|
| `python3 script.py` | `uv run python3 script.py` | Bare command auto-wrapped |
| `pytest -x` | `uv run crunch pytest -x` | Wrapped + crunch compression |
| `uv run pytest -x` | `uv run crunch pytest -x` | Already wrapped, crunch inserted |
| `git status` | `crunch git status` | Not in wrap_commands, normal rewrite |

### Priority chain

1. Already wrapped (`uv run ...`) — ENV_WRAPPER handles crunch insertion
2. Has mise mapping — mise handles env, skip wrapping
3. In wrap_commands — prepend wrapper
4. Normal crunch rewrite

### ENV_WRAPPER recognition (automatic, no config needed)

Crunch automatically detects `uv run`, `poetry run`, `pipx run`, `pdm run` prefixes and inserts `crunch` between the wrapper and the inner tool. This works without any `[env]` config:

```
uv run pytest -x          → uv run crunch pytest -x
poetry run ruff check .   → poetry run crunch ruff check .
```

### When to use `[env]` vs mise

| Scenario | Use |
|----------|-----|
| Tools with mise tasks (`pytest = "test"`) | `[mise]` section in `.crunch.toml` |
| Bare `python3`/`pip` calls needing venv | `[env]` section in global config |
| Model already says `uv run pytest` | Nothing — ENV_WRAPPER handles it automatically |

## Mise Mapping Rules

**Always run `mise tasks` first.** Map the crunch tool name to the mise task name:

```toml
[mise]
# tool = "mise-task-name"
pytest = "test"        # crunch pytest -> mise run test
ruff = "lint"          # crunch ruff -> mise run lint
```

Common mappings by ecosystem:

| Ecosystem | Tool -> Task |
|-----------|-------------|
| Python | `pytest = "test"`, `ruff = "lint"`, `mypy = "typecheck"` |
| Node | `vitest = "test"`, `tsc = "typecheck"`, `prettier = "format"` |
| Go | `go = "test"` (if wrapped in mise task) |
| Ruby | `rspec = "test"`, `rubocop = "lint"` |

**No `[mise]` section = tools run directly** (RTK default behavior).

## Tee Config

Defaults are usually fine. Override only when needed:

```toml
[tee]
directory = "/custom/path"   # default: /tmp/crunch
mode = "failures"            # default: "always" (always | failures | never)
max_files = 50               # default: 20

[tee.overrides]
git = { enabled = false }
ls = { enabled = false }
```

**Do NOT use a `path` template field** — it doesn't exist. Use `directory` for custom base path.

## Filters by Project Type

```toml
# Python
[filters]
ignore_dirs = [".git", "__pycache__", ".venv", ".pytest_cache", ".mypy_cache", ".ruff_cache"]

# Rust
[filters]
ignore_dirs = [".git", "target"]

# Node/TypeScript
[filters]
ignore_dirs = [".git", "node_modules", ".next", "dist", "coverage"]

# Go
[filters]
ignore_dirs = [".git", "vendor"]
```

Only include `[filters]` if you need to override the defaults:
`.git`, `node_modules`, `target`, `__pycache__`, `.venv`, `vendor`, `.pytest_cache`, `.mypy_cache`

## Common Mistakes

- **Assuming tools without checking** — run `mise tasks` to see actual task names
- **Using `path` instead of `directory`** — the `path` template field doesn't exist
- **Including defaults** — only add sections that differ from defaults; less config = less drift
- **Confusing tee overrides with hook excludes** — `[tee.overrides]` controls logging; `[hooks].exclude_commands` controls which commands the hook rewrites
- **Putting `[env]` in `.crunch.toml`** — blocked for security. Must go in `~/.config/crunch/config.toml`
- **Using an unsupported wrapper** — only `uv run`, `poetry run`, `pipx run`, `pdm run`, `conda run`, `nix run`, `mise run` are allowed
- **Wrapping tools that have mise mappings** — if `pytest = "test"` exists in `[mise]`, don't put `pytest` in `wrap_commands` (mise handles the env)
- **Not needing `[env]` at all** — if the model already says `uv run pytest`, crunch detects the wrapper automatically. `[env]` is only for bare `python3`/`pip` calls
