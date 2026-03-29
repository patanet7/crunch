---
name: configure-crunch
description: Use when setting up crunch for a new project, adding .crunch.toml, configuring mise task mappings, customizing tee logging, or adjusting per-project ignore directories
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

| Section | Purpose | When to include |
|---------|---------|-----------------|
| `[mise]` | Map tool names to mise tasks | Only if project uses mise |
| `[tee]` | Log directory, mode, limits | Only to override defaults |
| `[tee.overrides]` | Disable tee per-tool | `git`, `ls` commonly disabled |
| `[filters]` | ignore_dirs, ignore_files | Project-specific noise dirs |
| `[hooks]` | exclude_commands | Defaults: `["read", "cat"]` |
| `[display]` | colors, max_width | Rarely needed per-project |

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
