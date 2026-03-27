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
