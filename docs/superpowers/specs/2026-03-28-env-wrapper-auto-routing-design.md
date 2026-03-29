# Env-Wrapper Auto-Routing

## Problem

When AI models issue bare commands like `python3 script.py` or `pip install foo`, these run outside the project's virtual environment. The user must manually prefix with `uv run`, `poetry run`, etc. This is error-prone and the model often forgets.

Separately, crunch-supported tools (pytest, ruff) that live in the venv need the env activated when crunch spawns them as subprocesses.

## Solution

A new `[env]` config section that auto-prepends an env wrapper to configured bare commands during hook rewrite.

## Config

```toml
# .crunch.toml or ~/.config/crunch/config.toml
[env]
wrapper = "uv run"
wrap_commands = ["python", "python3", "pip", "pip3"]
```

- `wrapper` — prefix command string (e.g., `"uv run"`, `"poetry run"`). Optional, no default.
- `wrap_commands` — list of base commands to auto-wrap. Optional, no default.
- Both must be set for the feature to activate.
- Follows existing config hierarchy: global defaults, project `.crunch.toml` overrides per-section.

## Priority Chain

When a command is received, checks happen in this order:

1. **Already wrapped** (`uv run ...`, `poetry run ...`) — skip, let ENV_WRAPPER logic handle crunch insertion
2. **Has mise mapping** — skip, mise handles the env activation
3. **Base command in wrap_commands** — prepend wrapper
4. **Normal rewrite** — existing behavior

## Rewrite Behavior

| Input | Config | Output | Reason |
|-------|--------|--------|--------|
| `python3 script.py` | wrapper + wrap_commands | `uv run python3 script.py` | Unsupported tool, needs env |
| `pytest -x` | wrapper + wrap_commands (includes pytest), no mise | `uv run crunch pytest -x` | Supported tool, needs env + compression |
| `pytest -x` | wrapper + wrap_commands, mise mapping exists | `crunch pytest -x` | Mise handles env |
| `uv run pytest -x` | any | `uv run crunch pytest -x` | Already wrapped, ENV_WRAPPER handles |
| `git status` | wrapper configured, not in wrap_commands | `crunch git status` | Not a wrapped command |
| `python3 script.py` | no wrapper configured | no rewrite | Feature inactive |
| `python3 setup.py && pytest` | wrapper + wrap_commands | `uv run python3 setup.py && uv run crunch pytest` | Both segments wrapped |
| `python3 script.py 2>&1 \| tail` | wrapper + wrap_commands | `uv run python3 script.py 2>&1 \| tail` | Redirects/pipes preserved |

## Architecture

### Config (`config.rs`)

New struct:
```rust
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct EnvConfig {
    #[serde(default)]
    pub wrapper: Option<String>,
    #[serde(default)]
    pub wrap_commands: Vec<String>,
}
```

Added to `Config` as `pub env: EnvConfig`.

Merge behavior: project `.crunch.toml` `[env]` section replaces global entirely (same as `[display]`, `[hooks]`, etc.).

### Rewrite Layer (`registry.rs`)

Rename existing `rewrite_segment` → `rewrite_segment_inner` (zero logic changes).

New `rewrite_segment` function:
```
fn rewrite_segment(seg, excluded):
    base_cmd = first word of seg (after stripping redirects)
    result = rewrite_segment_inner(seg, excluded)

    env = cached_config().env
    if env.wrapper is None → return result
    if base_cmd NOT in env.wrap_commands → return result
    if seg already starts with wrapper → return result
    if lookup_mise_task(base_cmd) is Some → return result

    wrapper = env.wrapper
    match result:
        Some(rewritten) → Some("{wrapper} {rewritten}")
        None → Some("{wrapper} {cmd_part}{redirect_suffix}")
```

### Files Changed

| File | Change |
|------|--------|
| `config.rs` | Add `EnvConfig`, field on `Config`, merge support in `merge_configs_from_str` |
| `registry.rs` | Rename `rewrite_segment` → `rewrite_segment_inner`, add wrapper layer |
| `rules.rs` | No changes |
| `mise_cmd.rs` | No changes (existing `lookup_mise_task` called from registry) |

### Test Cases

1. `python3 script.py` with wrapper → `uv run python3 script.py`
2. `pytest -x` with wrapper, no mise → `uv run crunch pytest -x`
3. `pytest -x` with wrapper + mise mapping → `crunch pytest -x` (mise wins)
4. `uv run pytest -x` already wrapped → `uv run crunch pytest -x` (no double wrap)
5. `git status` not in wrap_commands → `crunch git status`
6. No wrapper configured → all behavior unchanged (regression)
7. Compound: `python3 setup.py && pytest -x` → both segments wrapped
8. Redirects/pipes: `python3 script.py 2>&1 | tail -5` → first segment wrapped
9. `python3 -c "print('hi')"` → ignored (in IGNORED_PREFIXES, rewrite_segment_inner returns None, but wrap_commands check happens... need to handle this)
10. Empty wrap_commands → feature inactive

### Edge Case: Ignored Commands

`python3 -c "..."` is in `IGNORED_PREFIXES`. The inner rewrite returns `None`. The wrapper layer would wrap it to `uv run python3 -c "..."` which is wasteful but harmless. Acceptable for v1 — could add an ignore check later if needed.

## Non-Goals

- Auto-detection of env manager from project markers (pyproject.toml, poetry.lock)
- Wrapping commands that already have mise mappings
- Handling `uv run --flags tool` (flags between wrapper and tool)
