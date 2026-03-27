# Crunch — Developer Guide

Privacy-respecting, mise-aware output compression proxy for AI coding assistants.
Fork of [RTK](https://github.com/rtk-ai/rtk) (Apache-2.0).

## Setup

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cargo build --release

# Install locally
cargo install --path .

# Setup Claude Code hook
crunch init -g
```

## Running things

```bash
cargo build                     # dev build
cargo build --release           # release build (LTO, stripped)
cargo test                      # all tests
cargo test <module>::tests::    # module-specific tests
cargo test -- --nocapture       # with stdout
cargo fmt --all                 # format
cargo clippy --all-targets      # lint
```

### Quality gate (run before every commit)

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```

## Project structure

```
src/
  main.rs              # CLI entry, clap enum router, command dispatch
  mise_cmd.rs          # NEW: mise routing (tool->task config lookup, flag passthrough)
  config.rs            # MODIFIED: [mise], [tee] config; no telemetry/tracking
  tee.rs               # MODIFIED: always-on, project-scoped logging to /tmp/crunch/
  filter.rs            # language-aware code filtering (none/minimal/aggressive)
  runner.rs            # command execution
  utils.rs             # shared utilities (truncate, strip_ansi, package detection)
  display_helpers.rs   # terminal formatting
  parser/              # error parsing infrastructure
  init.rs              # hook setup (Claude Code, Cursor, Gemini, etc.)
  hook_cmd.rs          # hook management
  rewrite_cmd.rs       # command rewrite logic

  # Per-tool parsers (inherited from RTK, keep untouched)
  git.rs               # git operations (diff, log, status, show, etc.)
  grep_cmd.rs          # code search (groups by file, truncates)
  ls.rs                # directory listing (filters noise dirs)
  read.rs              # file reading (filter-level stripping)
  find_cmd.rs          # file finding (gitignore-aware)
  pytest_cmd.rs        # pytest (state machine parser, failures only)
  ruff_cmd.rs          # ruff (JSON check, text format)
  mypy_cmd.rs          # mypy (group by file/error code)
  container.rs         # docker/kubectl (compact tables)
  log_cmd.rs           # log deduplication
  cargo_cmd.rs         # cargo build/test/clippy
  go_cmd.rs            # go test/build/vet
  # ... 71 total parser modules

docs/
  design.md            # design spec
```

## Key conventions

### What changed from RTK

- **Stripped**: `telemetry.rs`, `tracking.rs`, `local_llm.rs`, `cc_economics.rs`, `ccusage.rs`, `gain.rs`, `session_cmd.rs`, `discover/`, `learn/`, `integrity.rs`, `verify_cmd.rs`, `hook_audit_cmd.rs`
- **Added**: `mise_cmd.rs` (mise task routing)
- **Modified**: `config.rs` (no telemetry/tracking, added [mise] and [tee]), `tee.rs` (always-on, project-scoped), `main.rs` (removed stripped modules), `Cargo.toml` (renamed, removed unused deps)
- **Untouched**: All 71 parser modules, `filter.rs`, `utils.rs`, `parser/`, `init.rs`, hook system

### Mise integration

Project tools route through mise; system tools execute directly.

```toml
# ~/.config/crunch/config.toml or .crunch.toml
[mise]
pytest = "test"       # crunch pytest → mise run test
ruff = "lint"         # crunch ruff → mise run lint
mypy = "typecheck"    # crunch mypy → mise run typecheck
```

`crunch pytest -x tests/test_build.py` becomes `mise run test -- -x tests/test_build.py`.
If no mapping exists, crunch executes the tool directly (RTK default behavior).

### Log-first tee

Every command's raw output is saved before parsing. Compressed summary is never the only copy.

```
/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log
```

- Clean pass: `Pytest: 12 passed` (no path)
- Failure/warning: includes log path so model can `Read` the full output

Per-tool tee config:
```toml
[tee]
enabled = true
path = "/tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log"

[tee.overrides]
git = { enabled = false }
ls = { enabled = false }
```

### Config hierarchy

1. `~/.config/crunch/config.toml` — global defaults
2. `.crunch.toml` in project root — per-project overrides (key-by-key merge)

## Rust conventions

- **Error handling**: `anyhow::Result` everywhere, always use `.context("description")` with `?`
- **No unwrap()** in production code — tests only
- **Graceful degradation**: if a parser fails, fall back to raw command output
- **Performance**: <10ms startup, no async, `lazy_static!` for regex, borrow over clone
- **Exit codes**: always preserve the underlying tool's exit code
- **Pipe compatibility**: stdout/stderr separation, unix behavior expected

## Common pitfalls

- Don't add async dependencies (kills startup time — no tokio)
- Don't recompile regex at runtime — use `lazy_static!`
- Don't panic on filter failure — always fall back to raw output
- Don't break exit code propagation — CI/CD depends on it
- Keep `read` filter default at `none` — aggressive strips function bodies

## Testing

Unit tests embedded in each module (`#[cfg(test)] mod tests`).
Pattern: raw string fixture → filter function → assert output contains/excludes.

```bash
cargo test                          # all
cargo test pytest_cmd::tests::      # specific module
cargo test -- --nocapture           # with stdout
```

For filter changes, also test manually: `crunch <cmd>` and inspect output.

## Build

Release profile: `opt-level = 3`, LTO, `codegen-units = 1`, `strip = true`, `panic = "abort"`.
Target binary size: <5MB.

```bash
cargo build --release
ls -lh target/release/crunch
```
