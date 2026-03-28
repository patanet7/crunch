use anyhow::{Context, Result};
use std::process::{Command, Stdio};

use crate::config;

/// Check if a tool has a mise task mapping in config.
/// Returns the mise task name if found, None otherwise.
pub fn lookup_mise_task(tool: &str) -> Option<String> {
    let config = config::cached_config();
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
    execute_via_mise_with_binary("mise", task, args, verbose)
}

/// Execute a tool through a specific mise binary path (for testability).
pub fn execute_via_mise_with_binary(
    binary: &str,
    task: &str,
    args: &[String],
    verbose: u8,
) -> Result<std::process::Output> {
    let mise_args = build_mise_command(task, args);

    if verbose > 0 {
        eprintln!("crunch: routing through mise: {}", mise_args.join(" "));
    }

    Command::new(binary)
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

    #[test]
    fn test_execute_via_mise_returns_error_when_binary_missing() {
        // Use a nonexistent binary path to simulate mise not being installed
        let result =
            execute_via_mise_with_binary("/nonexistent/path/mise_fake_binary", "some_task", &[], 0);
        assert!(result.is_err());
        // The error message should mention mise or Failed
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("mise") || err_msg.contains("Failed"),
            "Expected error message to contain 'mise' or 'Failed', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_build_mise_command_preserves_flag_order() {
        let cmd = build_mise_command(
            "test",
            &[
                "-x".into(),
                "--tb=short".into(),
                "tests/test_build.py".into(),
            ],
        );
        assert_eq!(
            cmd,
            vec![
                "mise",
                "run",
                "test",
                "--",
                "-x",
                "--tb=short",
                "tests/test_build.py"
            ]
        );
    }

    #[test]
    fn test_build_mise_command_empty_task() {
        let cmd = build_mise_command("", &[]);
        assert_eq!(cmd, vec!["mise", "run", ""]);
    }

    #[test]
    fn test_lookup_mise_task_unmapped_tool() {
        // Any tool not in config should return None
        assert!(lookup_mise_task("completely_nonexistent_tool_12345").is_none());
    }

    #[test]
    fn test_execute_via_mise_error_contains_context() {
        // Use a known-bad binary path to trigger error
        let result = execute_via_mise_with_binary("/nonexistent/binary/path", "test", &[], 0);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Error should have useful context
        assert!(
            err.contains("Failed to execute mise") || err.contains("No such file"),
            "Error should contain context: {}",
            err
        );
    }
}
