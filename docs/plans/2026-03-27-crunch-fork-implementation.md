# Crunch Fork Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fork RTK into Crunch — strip telemetry, add mise integration, rework tee to log-first architecture, rebrand all user-facing strings.

**Architecture:** Bulk-copy RTK source into `src/`, strip telemetry (1 file + 1 call), add mise routing layer that intercepts configured tools and dispatches through `mise run <task>`, rework tee from failure-only to always-on project-scoped logging in `/tmp/crunch/`, rebrand all references from `rtk` to `crunch`.

**Tech Stack:** Rust, clap, anyhow, serde/toml, regex, lazy_static, rusqlite (tracking kept)

**Spec:** `docs/design.md`

---

## File Map

### Wave 0 — Bulk Copy & Rename

| Action | Path | Notes |
|--------|------|-------|
| Copy | `upstream-rtk/src/**/*.rs` → `src/` | All 72 .rs files + `discover/`, `learn/`, `parser/`, `filters/` dirs |
| Copy | `upstream-rtk/hooks/` → `hooks/` | All hook scripts |
| Copy | `upstream-rtk/build.rs` → `build.rs` | TOML filter build script |
| Copy | `upstream-rtk/Cargo.toml` → `Cargo.toml` | Then modify |
| Delete | `src/telemetry.rs` | Phone-home ping |
| Modify | `src/main.rs` | Remove `mod telemetry`, remove `telemetry::maybe_ping()` |
| Modify | `Cargo.toml` | Rename to `crunch`, drop `ureq` + `hostname` deps, update metadata |

### Wave 1 — Mise Integration

| Action | Path | Notes |
|--------|------|-------|
| Modify | `src/config.rs` | Add `MiseConfig` struct, add `mise` field to `Config` |
| Create | `src/mise_cmd.rs` | Mise routing: config lookup, `mise run <task> -- [args]` dispatch |
| Modify | `src/main.rs` | Add `mod mise_cmd`, wire mise check into command dispatch |

### Wave 2 — Tee Rework

| Action | Path | Notes |
|--------|------|-------|
| Modify | `src/config.rs` | Add `TeeOverrides` map to `TeeConfig` |
| Modify | `src/tee.rs` | Always-on default, project-scoped path template, per-tool overrides, scope detection |

### Wave 3 — Init & Hook Rebranding

| Action | Path | Notes |
|--------|------|-------|
| Modify | `hooks/rtk-rewrite.sh` → `hooks/crunch-rewrite.sh` | Rebrand all strings |
| Modify | `hooks/cursor-rtk-rewrite.sh` → `hooks/cursor-crunch-rewrite.sh` | Rebrand |
| Modify | `hooks/opencode-rtk.ts` → `hooks/opencode-crunch.ts` | Rebrand |
| Modify | `hooks/rtk-awareness.md` → `hooks/crunch-awareness.md` | Rebrand |
| Modify | `hooks/rtk-awareness-codex.md` → `hooks/crunch-awareness-codex.md` | Rebrand |
| Modify | All other `hooks/*` files | Rebrand rtk→crunch |
| Modify | `src/init.rs` | Update `include_str!` paths, rebrand all strings |
| Modify | `src/rewrite_cmd.rs` | Rebrand output strings |
| Modify | `src/hook_cmd.rs` | Rebrand |

### Wave 4 — Full Rebrand & Polish

| Action | Path | Notes |
|--------|------|-------|
| Modify | `src/main.rs` | CLI name, about, long_about, all user-facing strings |
| Modify | `src/display_helpers.rs` | Any `rtk` strings in formatting |
| Modify | `src/gain.rs` | Rebrand gain command output |
| Modify | `src/discover/**` | Rebrand discovery output strings |
| Modify | `src/toml_filter.rs` | `.rtk/filters.toml` → `.crunch/filters.toml`, env vars `RTK_*` → `CRUNCH_*` |
| Modify | `src/trust.rs` | Rebrand trust paths |
| Modify | `src/hook_check.rs` | Rebrand warning messages |
| Modify | `src/integrity.rs` | Rebrand |
| Modify | `src/permissions.rs` | Rebrand |
| Modify | `CLAUDE.md` | Update to reflect implemented state |

---

## Wave 0: Bulk Copy & Rename

### Task 0.1: Copy upstream source tree

**Files:**
- Copy: `upstream-rtk/src/` → `src/`
- Copy: `upstream-rtk/hooks/` → `hooks/`
- Copy: `upstream-rtk/build.rs` → `build.rs`
- Copy: `upstream-rtk/Cargo.toml` → `Cargo.toml`

- [ ] **Step 1: Copy all source files**

```bash
cp -R upstream-rtk/src/ src/
cp -R upstream-rtk/hooks/ hooks/
cp upstream-rtk/build.rs build.rs
cp upstream-rtk/Cargo.toml Cargo.toml
```

- [ ] **Step 2: Verify the copy**

Run: `ls src/*.rs | wc -l && ls src/discover/ && ls src/learn/ && ls src/filters/ | head -5 && ls hooks/`
Expected: ~60+ .rs files, discover/ with mod.rs/provider.rs/registry.rs/report.rs/rules.rs, learn/ with detector.rs/mod.rs/report.rs, filters/ with .toml files, hooks/ with shell scripts.

- [ ] **Step 3: Verify build compiles as-is (still named rtk)**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles successfully (binary named `rtk`).

- [ ] **Step 4: Verify tests pass as-is**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 5: Commit the bulk copy**

```bash
git add src/ hooks/ build.rs Cargo.toml
git commit -m "feat: bulk copy RTK v0.34.0 source into crunch repo"
```

### Task 0.2: Strip telemetry

**Files:**
- Delete: `src/telemetry.rs`
- Modify: `src/main.rs:1` (remove `mod telemetry`)
- Modify: `src/main.rs:1277-1278` (remove `telemetry::maybe_ping()`)
- Modify: `Cargo.toml` (remove `ureq` and `hostname` deps)

- [ ] **Step 1: Delete telemetry module**

```bash
rm src/telemetry.rs
```

- [ ] **Step 2: Remove `mod telemetry` from main.rs**

In `src/main.rs`, find and remove the line:
```rust
mod telemetry;
```

- [ ] **Step 3: Remove telemetry call from main()**

In `src/main.rs`, find and remove these lines from the `main()` function:
```rust
    // Fire-and-forget telemetry ping (1/day, non-blocking)
    telemetry::maybe_ping();
```

- [ ] **Step 4: Remove `ureq` and `hostname` from Cargo.toml**

In `Cargo.toml`, remove these two lines from `[dependencies]`:
```toml
ureq = "2"
hostname = "0.4"
```

- [ ] **Step 5: Verify build still compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles successfully with no dead code warnings for telemetry.

- [ ] **Step 6: Verify tests still pass**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: strip telemetry — remove phone-home ping, ureq, hostname deps"
```

### Task 0.3: Rename package to crunch

**Files:**
- Modify: `Cargo.toml` (package name, binary name, metadata)
- Modify: `src/main.rs` (CLI name in clap derive)

- [ ] **Step 1: Update Cargo.toml package metadata**

In `Cargo.toml`, change the `[package]` section:

```toml
[package]
name = "crunch"
version = "0.1.0"
edition = "2021"
description = "Privacy-respecting, mise-aware output compression proxy for AI coding assistants"
license = "Apache-2.0"
repository = "https://github.com/thomaspatane/crunch"
readme = "README.md"
keywords = ["cli", "llm", "token", "filter", "mise"]
categories = ["command-line-utilities", "development-tools"]
```

Remove the `authors`, `homepage` fields (those were RTK-specific). Remove the `[package.metadata.deb]` and `[package.metadata.generate-rpm]` sections (not needed yet).

- [ ] **Step 2: Update CLI name in main.rs**

In `src/main.rs`, change the clap `#[command]` attribute:

```rust
#[derive(Parser)]
#[command(
    name = "crunch",
    version,
    about = "Crunch - Output compression proxy for AI coding assistants",
    long_about = "A privacy-respecting, mise-aware CLI proxy that filters and summarizes tool outputs before they reach your LLM context."
)]
struct Cli {
```

- [ ] **Step 3: Build and verify binary name**

Run: `cargo build 2>&1 | tail -3 && ls target/debug/crunch`
Expected: Compiles, binary exists at `target/debug/crunch`.

- [ ] **Step 4: Verify the binary runs**

Run: `target/debug/crunch --version`
Expected: `crunch 0.1.0`

- [ ] **Step 5: Run tests**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/main.rs Cargo.lock
git commit -m "feat: rename package rtk → crunch, update CLI metadata"
```

### Task 0.4: Quality gate

- [ ] **Step 1: Run full quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: No format issues, no clippy warnings (or only pre-existing ones from upstream), all tests pass.

- [ ] **Step 2: Check binary size**

Run: `cargo build --release 2>&1 | tail -3 && ls -lh target/release/crunch`
Expected: Binary exists, size <5MB.

- [ ] **Step 3: Commit any fmt/clippy fixes if needed**

```bash
git add -A
git commit -m "chore: fmt + clippy cleanup after bulk copy"
```

---

## Wave 1: Mise Integration

### Task 1.1: Add MiseConfig to config.rs

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write test for MiseConfig deserialization**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/config.rs`:

```rust
    #[test]
    fn test_mise_config_deserialize() {
        let toml = r#"
[mise]
pytest = "test"
ruff = "lint"
mypy = "typecheck"
"#;
        let config: Config = toml::from_str(toml).expect("valid toml");
        assert_eq!(config.mise.get("pytest"), Some(&"test".to_string()));
        assert_eq!(config.mise.get("ruff"), Some(&"lint".to_string()));
        assert_eq!(config.mise.get("mypy"), Some(&"typecheck".to_string()));
    }

    #[test]
    fn test_mise_config_default_empty() {
        let config = Config::default();
        assert!(config.mise.is_empty());
    }

    #[test]
    fn test_config_without_mise_section_is_valid() {
        let toml = r#"
[tracking]
enabled = true
history_days = 90
"#;
        let config: Config = toml::from_str(toml).expect("valid toml");
        assert!(config.mise.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config::tests::test_mise_config 2>&1 | tail -10`
Expected: FAIL — `Config` has no `mise` field.

- [ ] **Step 3: Add MiseConfig type alias and field to Config struct**

In `src/config.rs`, add the type alias near the top (after imports):

```rust
use std::collections::HashMap;

/// Tool-to-task mapping for mise integration.
/// Keys are tool names (e.g., "pytest"), values are mise task names (e.g., "test").
pub type MiseConfig = HashMap<String, String>;
```

Then add the field to the `Config` struct:

```rust
pub struct Config {
    #[serde(default)]
    pub tracking: TrackingConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub tee: crate::tee::TeeConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub mise: MiseConfig,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config::tests::test_mise_config 2>&1 | tail -10`
Expected: All 3 new tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add [mise] config section for tool-to-task mapping"
```

### Task 1.2: Add per-project .crunch.toml config loading

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write test for project config merge**

Add to tests in `src/config.rs`:

```rust
    #[test]
    fn test_merge_project_config() {
        let mut global = Config::default();
        global.mise.insert("pytest".into(), "test".into());
        global.mise.insert("ruff".into(), "lint".into());

        let project_toml = r#"
[mise]
pytest = "test-fast"
mypy = "typecheck"
"#;
        let project: Config = toml::from_str(project_toml).expect("valid toml");
        let merged = merge_configs(global, project);

        // Project overrides global for pytest
        assert_eq!(merged.mise.get("pytest"), Some(&"test-fast".to_string()));
        // Project adds mypy
        assert_eq!(merged.mise.get("mypy"), Some(&"typecheck".to_string()));
        // Global ruff preserved
        assert_eq!(merged.mise.get("ruff"), Some(&"lint".to_string()));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test config::tests::test_merge_project_config 2>&1 | tail -10`
Expected: FAIL — `merge_configs` not defined.

- [ ] **Step 3: Implement merge_configs and project config loading**

Add to `src/config.rs`:

```rust
/// Merge a project-level config on top of a global config.
/// Project values override global values key-by-key within each section.
/// The mise HashMap merges naturally: project keys override or extend global keys.
pub fn merge_configs(global: Config, project: Config) -> Config {
    let mut merged = global;

    // Merge mise: project overrides/extends global
    for (k, v) in project.mise {
        merged.mise.insert(k, v);
    }

    // Merge tee overrides if project specifies them
    if project.tee.enabled != TeeConfig::default().enabled
        || project.tee.mode != TeeConfig::default().mode
    {
        merged.tee = project.tee;
    }

    // Merge hooks exclude_commands
    if !project.hooks.exclude_commands.is_empty() {
        merged.hooks.exclude_commands = project.hooks.exclude_commands;
    }

    merged
}

/// Load config with project-level .crunch.toml merge.
/// Priority: .crunch.toml (cwd) > ~/.config/crunch/config.toml > defaults.
pub fn load_merged() -> Result<Config> {
    let global = Config::load()?;

    let project_path = std::env::current_dir()
        .unwrap_or_default()
        .join(".crunch.toml");

    if project_path.exists() {
        let content = std::fs::read_to_string(&project_path)
            .with_context(|| format!("Failed to read {}", project_path.display()))?;
        let project: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", project_path.display()))?;
        Ok(merge_configs(global, project))
    } else {
        Ok(global)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test config::tests::test_merge 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add .crunch.toml per-project config with key-by-key merge"
```

### Task 1.3: Create mise_cmd.rs — mise routing module

**Files:**
- Create: `src/mise_cmd.rs`
- Modify: `src/main.rs` (add `mod mise_cmd`)

- [ ] **Step 1: Write tests for mise routing**

Create `src/mise_cmd.rs` with tests:

```rust
use anyhow::{Context, Result};
use std::process::{Command, Stdio};

use crate::config;

/// Check if a tool has a mise task mapping in config.
/// Returns the mise task name if found, None otherwise.
pub fn lookup_mise_task(tool: &str) -> Option<String> {
    let config = config::load_merged().ok()?;
    config.mise.get(tool).cloned()
}

/// Build the mise command line: `mise run <task> -- [args]`
pub fn build_mise_command(task: &str, args: &[String]) -> Vec<String> {
    let mut cmd = vec!["mise".to_string(), "run".to_string(), task.to_string()];
    if !args.is_empty() {
        cmd.push("--".to_string());
        cmd.extend(args.iter().cloned());
    }
    cmd
}

/// Execute a tool through mise, returning the raw output.
/// Returns None if mise is not available or the task doesn't exist.
pub fn execute_via_mise(task: &str, args: &[String], verbose: u8) -> Result<std::process::Output> {
    let mise_args = build_mise_command(task, args);

    if verbose > 0 {
        eprintln!("crunch: routing through mise: {}", mise_args.join(" "));
    }

    Command::new("mise")
        .args(&mise_args[1..]) // skip "mise" itself
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to execute mise")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_mise_command_with_args() {
        let cmd = build_mise_command("test", &["-x".into(), "tests/test_build.py".into()]);
        assert_eq!(
            cmd,
            vec!["mise", "run", "test", "--", "-x", "tests/test_build.py"]
        );
    }

    #[test]
    fn test_build_mise_command_no_args() {
        let cmd = build_mise_command("lint", &[]);
        assert_eq!(cmd, vec!["mise", "run", "lint"]);
    }

    #[test]
    fn test_lookup_mise_task_no_config() {
        // With no .crunch.toml and no global config, should return None
        let result = lookup_mise_task("nonexistent_tool_xyz");
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Add `mod mise_cmd` to main.rs**

In `src/main.rs`, add after the other `mod` declarations:

```rust
mod mise_cmd;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test mise_cmd::tests 2>&1 | tail -10`
Expected: All 3 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add src/mise_cmd.rs src/main.rs
git commit -m "feat: add mise_cmd module with task lookup and command building"
```

### Task 1.4: Wire mise routing into main.rs command dispatch

**Files:**
- Modify: `src/main.rs`

This is the key integration point. When a command like `crunch pytest -x` arrives, before dispatching to the `pytest_cmd` module, we check if there's a mise mapping. If so, we execute through mise but still parse the output with the tool-specific parser.

- [ ] **Step 1: Add mise routing helper to main.rs**

Add this function before `fn main()` in `src/main.rs`:

```rust
/// Check if a tool should route through mise. If so, execute via mise
/// and return the raw output. The caller is responsible for parsing.
fn try_mise_route(tool: &str, args: &[String], verbose: u8) -> Option<std::process::Output> {
    let task = mise_cmd::lookup_mise_task(tool)?;
    mise_cmd::execute_via_mise(&task, args, verbose).ok()
}
```

- [ ] **Step 2: Apply mise routing to the Pytest command as the first integration**

In `src/main.rs`, find the `Commands::Pytest` match arm and modify it to check mise first:

```rust
        Commands::Pytest { args } => {
            if let Some(output) = try_mise_route("pytest", &args, cli.verbose) {
                // Route through mise, but still parse with pytest parser
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{}\n{}", stdout, stderr);
                let exit_code = output.status.code().unwrap_or(1);
                // Use pytest_cmd's filter on the mise output
                pytest_cmd::run_with_output(&combined, &args, exit_code, cli.verbose)?;
            } else {
                pytest_cmd::run(&args, cli.verbose)?;
            }
        }
```

Note: This requires `pytest_cmd` to expose a `run_with_output` function. We'll add that in the next step.

- [ ] **Step 3: Add `run_with_output` to pytest_cmd.rs**

Read the existing `pytest_cmd::run` function to understand its structure, then add a `run_with_output` variant that accepts pre-captured output instead of executing the command itself. The key insight is that the parser logic (the `filter_*` function) is separate from the execution logic — we just need to call the parser with the mise output.

Look at the existing `run()` function in `src/pytest_cmd.rs` and add a parallel function:

```rust
/// Run pytest parser on pre-captured output (used when routing through mise).
pub fn run_with_output(raw: &str, args: &[String], exit_code: i32, verbose: u8) -> Result<()> {
    let timer = crate::tracking::TimedExecution::start();
    let raw_command = format!("pytest {}", args.join(" "));

    let filtered = filter_pytest_output(raw, verbose);

    if let Some(hint) = crate::tee::tee_and_hint(raw, "pytest", exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    timer.track(&raw_command, "crunch pytest (mise)", raw, &filtered);

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}
```

The exact implementation depends on the shape of the existing `filter_pytest_output` function — read `src/pytest_cmd.rs` to confirm the filter function name and signature before implementing.

- [ ] **Step 4: Build and test**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles.

Run: `cargo test pytest_cmd::tests 2>&1 | tail -10`
Expected: Existing pytest tests still pass.

- [ ] **Step 5: Apply the same pattern to ruff, mypy, and other common mise-routed tools**

Repeat the pattern from Steps 2-3 for:
- `Commands::Ruff` → check `mise_cmd::lookup_mise_task("ruff")`
- `Commands::Mypy` → check `mise_cmd::lookup_mise_task("mypy")`

Each follows the identical pattern: check mise first, if mapped execute through mise then parse with the tool's existing filter, otherwise fall through to normal execution.

- [ ] **Step 6: Run full test suite**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs src/pytest_cmd.rs src/ruff_cmd.rs src/mypy_cmd.rs
git commit -m "feat: wire mise routing into pytest, ruff, mypy command dispatch"
```

---

## Wave 2: Tee Rework (Log-First Architecture)

### Task 2.1: Add per-tool overrides to TeeConfig

**Files:**
- Modify: `src/tee.rs`
- Modify: `src/config.rs` (if TeeConfig is referenced there)

- [ ] **Step 1: Write tests for per-tool overrides**

Add to `#[cfg(test)] mod tests` in `src/tee.rs`:

```rust
    #[test]
    fn test_tee_overrides_deserialize() {
        let toml_str = r#"
enabled = true
mode = "always"
max_files = 20
max_file_size = 1048576

[overrides]
git = { enabled = false }
ls = { enabled = false }
pytest = { enabled = true }
"#;
        let config: TeeConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.is_tool_enabled("git"));
        assert!(!config.is_tool_enabled("ls"));
        assert!(config.is_tool_enabled("pytest"));
        // Tools not in overrides inherit global setting
        assert!(config.is_tool_enabled("cargo"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test tee::tests::test_tee_overrides 2>&1 | tail -10`
Expected: FAIL — `overrides` field and `is_tool_enabled` don't exist.

- [ ] **Step 3: Add overrides to TeeConfig**

In `src/tee.rs`, add the override types and modify `TeeConfig`:

```rust
use std::collections::HashMap;

/// Per-tool tee override.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeeToolOverride {
    pub enabled: bool,
}

/// Configuration for the tee feature.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeeConfig {
    pub enabled: bool,
    pub mode: TeeMode,
    pub max_files: usize,
    pub max_file_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub overrides: HashMap<String, TeeToolOverride>,
}

impl TeeConfig {
    /// Check if tee is enabled for a specific tool.
    /// Per-tool overrides take precedence over the global `enabled` setting.
    pub fn is_tool_enabled(&self, tool: &str) -> bool {
        if let Some(over) = self.overrides.get(tool) {
            over.enabled
        } else {
            self.enabled
        }
    }
}
```

Update `Default for TeeConfig` to include `overrides: HashMap::new()`.

- [ ] **Step 4: Run tests**

Run: `cargo test tee::tests 2>&1 | tail -15`
Expected: All tee tests pass, including the new override test.

- [ ] **Step 5: Commit**

```bash
git add src/tee.rs
git commit -m "feat: add per-tool tee overrides config"
```

### Task 2.2: Change tee to always-on, project-scoped paths

**Files:**
- Modify: `src/tee.rs`

- [ ] **Step 1: Write tests for project-scoped path and always-on mode**

Add to tests in `src/tee.rs`:

```rust
    #[test]
    fn test_project_log_path_format() {
        let path = build_log_path("myproject", "pytest", "test_build");
        let path_str = path.to_string_lossy();
        assert!(path_str.starts_with("/tmp/crunch/myproject/"));
        assert!(path_str.contains("pytest-test_build-"));
        assert!(path_str.ends_with(".log"));
    }

    #[test]
    fn test_detect_project_name() {
        // In a git repo, should return the repo basename
        let name = detect_project_name();
        // We're in the crunch repo, so:
        assert!(!name.is_empty());
    }

    #[test]
    fn test_detect_scope_from_args() {
        assert_eq!(detect_scope(&["tests/test_build.py".to_string()]), "test_build");
        assert_eq!(detect_scope(&["-x".to_string(), "-q".to_string()]), "all");
        assert_eq!(detect_scope(&[]), "all");
    }

    #[test]
    fn test_tee_default_is_always() {
        let config = TeeConfig::default();
        assert_eq!(config.mode, TeeMode::Always);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test tee::tests::test_project 2>&1 | tail -10`
Expected: FAIL — functions don't exist, default is still `Failures`.

- [ ] **Step 3: Implement project-scoped tee functions**

In `src/tee.rs`, add:

```rust
/// Detect project name from git root or cwd basename.
pub fn detect_project_name() -> String {
    // Try git rev-parse
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(name) = std::path::Path::new(&path).file_name() {
                return name.to_string_lossy().to_string();
            }
        }
    }
    // Fallback to cwd basename
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Derive scope from command args (e.g., file paths, -k filters).
/// Falls back to "all" if no meaningful scope can be detected.
pub fn detect_scope(args: &[String]) -> String {
    for arg in args {
        // Skip flags
        if arg.starts_with('-') {
            continue;
        }
        // Use filename stem if it looks like a path
        let path = std::path::Path::new(arg);
        if let Some(stem) = path.file_stem() {
            return sanitize_slug(&stem.to_string_lossy());
        }
    }
    "all".to_string()
}

/// Build the log path: /tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log
pub fn build_log_path(project: &str, tool: &str, scope: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("{}-{}-{}.log", sanitize_slug(tool), sanitize_slug(scope), timestamp);
    PathBuf::from("/tmp/crunch").join(sanitize_slug(project)).join(filename)
}
```

- [ ] **Step 4: Change TeeMode default from Failures to Always**

```rust
impl Default for TeeMode {
    fn default() -> Self {
        Self::Always
    }
}
```

- [ ] **Step 5: Update `tee_raw` to use project-scoped paths**

Replace `get_tee_dir` usage with the new project-scoped path builder. The new `tee_raw` function signature adds `tool` and `args` parameters:

```rust
/// Write raw output to project-scoped tee file.
/// Returns file path on success, None if skipped/failed.
pub fn tee_raw_scoped(raw: &str, tool: &str, args: &[String], exit_code: i32) -> Option<PathBuf> {
    // Check CRUNCH_TEE=0 env override (disable)
    if std::env::var("CRUNCH_TEE").ok().as_deref() == Some("0") {
        return None;
    }

    let config = Config::load().ok().map(|c| c.tee).unwrap_or_default();

    // Check per-tool override
    if !config.is_tool_enabled(tool) {
        return None;
    }

    // In always mode, skip tee for success only if output is tiny
    match config.mode {
        TeeMode::Never => return None,
        TeeMode::Failures => {
            if exit_code == 0 {
                return None;
            }
        }
        TeeMode::Always => {}
    }

    if raw.len() < MIN_TEE_SIZE {
        return None;
    }

    let project = detect_project_name();
    let scope = detect_scope(args);
    let log_path = build_log_path(&project, tool, &scope);

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    let content = if raw.len() > config.max_file_size {
        format!(
            "{}\n\n--- truncated at {} bytes ---",
            &raw[..config.max_file_size],
            config.max_file_size
        )
    } else {
        raw.to_string()
    };

    std::fs::write(&log_path, content).ok()?;

    // Rotate old files in the project directory
    if let Some(parent) = log_path.parent() {
        cleanup_old_files(parent, config.max_files);
    }

    Some(log_path)
}

/// Convenience: tee + format hint for project-scoped logs.
pub fn tee_and_hint_scoped(raw: &str, tool: &str, args: &[String], exit_code: i32) -> Option<String> {
    let path = tee_raw_scoped(raw, tool, args, exit_code)?;
    Some(format_hint(&path))
}
```

Keep the old `tee_raw` and `tee_and_hint` functions as-is for backward compatibility — existing modules call them. We'll migrate callers incrementally.

- [ ] **Step 6: Run all tests**

Run: `cargo test tee::tests 2>&1 | tail -15`
Expected: All tests pass.

- [ ] **Step 7: Run full quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: Clean.

- [ ] **Step 8: Commit**

```bash
git add src/tee.rs
git commit -m "feat: always-on project-scoped tee with /tmp/crunch/{project}/ paths"
```

### Task 2.3: Update summary output to conditionally show log path

**Files:**
- Modify: `src/tee.rs`

- [ ] **Step 1: Write test for conditional hint display**

Add to tests in `src/tee.rs`:

```rust
    #[test]
    fn test_hint_shown_on_failure() {
        // Failure with large output should produce a hint
        let raw = "x".repeat(1000);
        // We can't easily test the full tee_and_hint_scoped without filesystem,
        // but we can test the format_hint function
        let path = PathBuf::from("/tmp/crunch/myproject/pytest-test_build-20260327-143012.log");
        let hint = format_hint(&path);
        assert!(hint.contains("/tmp/crunch/myproject/pytest-test_build-20260327-143012.log"));
    }

    #[test]
    fn test_hint_format_no_tilde_for_tmp() {
        // /tmp paths should NOT use ~ shorthand
        let path = PathBuf::from("/tmp/crunch/myproject/pytest-all-20260327-143012.log");
        let hint = format_hint(&path);
        assert!(hint.starts_with("[full output: /tmp/crunch/"));
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test tee::tests::test_hint 2>&1 | tail -10`
Expected: PASS (format_hint already exists, these just confirm behavior with new paths).

- [ ] **Step 3: Commit**

```bash
git add src/tee.rs
git commit -m "test: verify hint format for project-scoped log paths"
```

---

## Wave 3: Init & Hook Rebranding

### Task 3.1: Rebrand hook scripts

**Files:**
- Rename + Modify: all files in `hooks/`

- [ ] **Step 1: Rename hook files**

```bash
cd hooks
mv rtk-rewrite.sh crunch-rewrite.sh
mv cursor-rtk-rewrite.sh cursor-crunch-rewrite.sh
mv opencode-rtk.ts opencode-crunch.ts
mv rtk-awareness.md crunch-awareness.md
mv rtk-awareness-codex.md crunch-awareness-codex.md
mv windsurf-rtk-rules.md windsurf-crunch-rules.md
mv cline-rtk-rules.md cline-crunch-rules.md
mv copilot-rtk-awareness.md copilot-crunch-awareness.md
mv test-rtk-rewrite.sh test-crunch-rewrite.sh 2>/dev/null
mv test-copilot-rtk-rewrite.sh test-copilot-crunch-rewrite.sh 2>/dev/null
cd ..
```

- [ ] **Step 2: Find-and-replace rtk→crunch in all hook files**

In each hook file, replace:
- `rtk` → `crunch` (binary name in commands)
- `RTK` → `Crunch` (display name)
- `rtk-ai/rtk` → appropriate crunch reference
- `~/.config/rtk/` → `~/.config/crunch/`
- `.rtk/` → `.crunch/`
- `RTK_` env var prefixes → `CRUNCH_` (e.g., `RTK_TEE` → `CRUNCH_TEE`)

Apply to: `crunch-rewrite.sh`, `cursor-crunch-rewrite.sh`, `opencode-crunch.ts`, all `.md` files.

**Be careful to NOT replace inside words** (e.g., don't turn "networks" into "necrunches"). Use word-boundary-aware replacements.

- [ ] **Step 3: Verify hook scripts are syntactically valid**

Run: `bash -n hooks/crunch-rewrite.sh && echo "OK"`
Expected: `OK`

- [ ] **Step 4: Commit**

```bash
git add hooks/
git commit -m "feat: rebrand all hook scripts from rtk to crunch"
```

### Task 3.2: Update init.rs include_str paths and branding

**Files:**
- Modify: `src/init.rs`

- [ ] **Step 1: Update all include_str! paths**

In `src/init.rs`, update the embedded file references:

```rust
const REWRITE_HOOK: &str = include_str!("../hooks/crunch-rewrite.sh");
const CURSOR_REWRITE_HOOK: &str = include_str!("../hooks/cursor-crunch-rewrite.sh");
const OPENCODE_PLUGIN: &str = include_str!("../hooks/opencode-crunch.ts");
const RTK_SLIM: &str = include_str!("../hooks/crunch-awareness.md");
const RTK_SLIM_CODEX: &str = include_str!("../hooks/crunch-awareness-codex.md");
```

- [ ] **Step 2: Replace all "rtk" strings in init.rs**

Search and replace throughout `src/init.rs`:
- `"rtk"` → `"crunch"` in command strings and paths
- `"RTK"` → `"Crunch"` in display strings
- `~/.config/rtk/` → `~/.config/crunch/`
- `.rtk/` → `.crunch/`
- `RTK.md` → `CRUNCH.md` (if referenced as an init artifact)
- Update config path references: `rtk/config.toml` → `crunch/config.toml`

- [ ] **Step 3: Update the FILTERS_TEMPLATE and FILTERS_GLOBAL_TEMPLATE strings**

Replace all references to RTK with Crunch in the template strings:
- `"# Project-local RTK filters"` → `"# Project-local Crunch filters"`
- `.rtk/filters.toml` → `.crunch/filters.toml`
- `~/.config/rtk/filters.toml` → `~/.config/crunch/filters.toml`
- `https://github.com/rtk-ai/rtk#custom-filters` → remove or update URL

- [ ] **Step 4: Build to verify include_str paths resolve**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles (include_str! paths match renamed files).

- [ ] **Step 5: Run tests**

Run: `cargo test 2>&1 | tail -15`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/init.rs
git commit -m "feat: rebrand init.rs — update hook paths and all rtk→crunch strings"
```

### Task 3.3: Update rewrite_cmd.rs and discover/registry.rs

**Files:**
- Modify: `src/rewrite_cmd.rs`
- Modify: `src/discover/registry.rs`
- Modify: `src/discover/rules.rs`

- [ ] **Step 1: Update rewrite_cmd.rs**

The `rewrite_command` function in `discover/registry.rs` returns strings like `"rtk git status"`. These need to become `"crunch git status"`.

Read `src/discover/rules.rs` to find where the RTK command strings are defined. Update all `rtk_equivalent` entries from `"rtk <cmd>"` to `"crunch <cmd>"`.

- [ ] **Step 2: Update registry.rs display strings**

Search `src/discover/registry.rs` for any `"rtk"` or `"RTK"` strings and replace with `"crunch"` / `"Crunch"`.

- [ ] **Step 3: Run rewrite tests**

Run: `cargo test rewrite_cmd::tests 2>&1 | tail -10`
Expected: Tests pass (the assertions in tests will need updating too — `"rtk git status"` → `"crunch git status"`).

- [ ] **Step 4: Run discover tests**

Run: `cargo test discover 2>&1 | tail -10`
Expected: Tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/rewrite_cmd.rs src/discover/
git commit -m "feat: rebrand rewrite and discover modules — rtk→crunch command strings"
```

---

## Wave 4: Full Rebrand & Polish

### Task 4.1: Rebrand config paths and env vars

**Files:**
- Modify: `src/config.rs`
- Modify: `src/tee.rs`
- Modify: `src/toml_filter.rs`
- Modify: `src/trust.rs`

- [ ] **Step 1: Update config.rs paths**

In `src/config.rs`, update `get_config_path()`:

```rust
fn get_config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    Ok(config_dir.join("crunch").join("config.toml"))
}
```

- [ ] **Step 2: Update tee.rs env vars and paths**

In `src/tee.rs`:
- `RTK_TEE` → `CRUNCH_TEE`
- `RTK_TEE_DIR` → `CRUNCH_TEE_DIR`
- `rtk/tee` in default path → `crunch/tee` (if the old `tee_raw` function still exists for compat)

- [ ] **Step 3: Update toml_filter.rs paths**

In `src/toml_filter.rs`:
- `.rtk/filters.toml` → `.crunch/filters.toml`
- `~/.config/rtk/filters.toml` → `~/.config/crunch/filters.toml`
- `RTK_NO_TOML` → `CRUNCH_NO_TOML`
- `RTK_TOML_DEBUG` → `CRUNCH_TOML_DEBUG`

- [ ] **Step 4: Update trust.rs**

In `src/trust.rs`, update any trust database paths from `rtk` → `crunch`.

- [ ] **Step 5: Build and test**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/tee.rs src/toml_filter.rs src/trust.rs
git commit -m "feat: rebrand config paths ~/.config/crunch/, env vars CRUNCH_*, .crunch/ dirs"
```

### Task 4.2: Rebrand all remaining user-facing strings

**Files:**
- Modify: `src/main.rs`, `src/display_helpers.rs`, `src/gain.rs`, `src/hook_cmd.rs`, `src/hook_check.rs`, `src/integrity.rs`, `src/permissions.rs`, `src/session_cmd.rs`, `src/discover/*.rs`, `src/learn/*.rs`

- [ ] **Step 1: Global search for remaining "rtk" strings**

Run: `grep -rn '"rtk\|"RTK\|\[rtk\]' src/ | grep -v '#\[cfg(test)\]' | head -40`
This finds all remaining hardcoded RTK strings in source code.

- [ ] **Step 2: Replace all user-facing strings**

Go through each file from the grep results and replace:
- `"rtk"` → `"crunch"` in command references, path strings, display output
- `"RTK"` → `"Crunch"` in display/branding strings
- `"[rtk:"` → `"[crunch:"` in error message prefixes (e.g., `eprintln!("[rtk: {}]", e)`)
- `"rtk "` → `"crunch "` in command prefixes

**Be surgical** — don't replace inside test fixtures that deliberately contain RTK output format strings (those test RTK-format parsing and should stay as-is).

- [ ] **Step 3: Update test assertions that reference "rtk"**

Tests that assert on output strings containing "rtk" need to be updated to assert "crunch" instead. But tests that parse *input* in RTK format (e.g., parsing upstream RTK output) should keep "rtk" in the input fixtures.

- [ ] **Step 4: Full quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: Clean, all tests pass.

- [ ] **Step 5: Verify no "rtk" leaks in user-facing output**

Run: `target/debug/crunch --help 2>&1 | grep -i rtk`
Expected: No matches.

Run: `target/debug/crunch git status 2>&1 | grep -i rtk`
Expected: No matches (only "crunch" in any output).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: complete rebrand — all user-facing strings now say crunch"
```

### Task 4.3: Update CLAUDE.md and final polish

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update CLAUDE.md to reflect implemented state**

Rewrite the CLAUDE.md header to match the standard format:

```markdown
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
```

Remove the "Current State" section (code now exists). Update the project structure to match reality. Keep all the conventions, pitfalls, and testing sections.

- [ ] **Step 2: Final release build and size check**

Run: `cargo build --release 2>&1 | tail -3 && ls -lh target/release/crunch`
Expected: Binary <5MB.

- [ ] **Step 3: Run the binary end-to-end**

Run: `target/release/crunch --version && target/release/crunch git status`
Expected: Shows `crunch 0.1.0`, then a compact git status.

- [ ] **Step 4: Final quality gate**

Run: `cargo fmt --all && cargo clippy --all-targets 2>&1 | tail -20 && cargo test --all 2>&1 | tail -15`
Expected: All clean.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md to reflect implemented crunch fork"
```
