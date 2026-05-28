use crate::config;
use crate::tracking;
use crate::utils::{resolved_command, truncate};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct RuffLocation {
    #[allow(dead_code)]
    row: usize,
    #[allow(dead_code)]
    column: usize,
}

#[derive(Debug, Deserialize)]
struct RuffFix {
    #[allow(dead_code)]
    applicability: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RuffDiagnostic {
    code: String,
    #[allow(dead_code)]
    message: String,
    #[allow(dead_code)]
    location: RuffLocation,
    #[allow(dead_code)]
    end_location: Option<RuffLocation>,
    filename: String,
    fix: Option<RuffFix>,
}

/// Process pre-captured output (e.g. from mise) through the ruff parser.
/// Handles filtering, tracking, and exit code — everything `run()` does
/// except executing the command.
pub fn run_with_output(raw: &str, args: &[String], exit_code: i32, verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("crunch: parsing ruff output ({} bytes)", raw.len());
    }

    // Detect subcommand from args
    let is_check = args.is_empty()
        || args[0] == "check"
        || (!args[0].starts_with('-') && args[0] != "format" && args[0] != "version");
    let is_format = args.iter().any(|a| a == "format");

    let filtered = if is_check && !raw.trim().is_empty() {
        filter_ruff_check_json(raw)
    } else if is_format {
        filter_ruff_format(raw)
    } else {
        raw.trim().to_string()
    };

    if let Some(hint) = crate::tee::tee_and_hint_scoped(raw, "ruff", args, exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    timer.track(
        &format!("ruff {}", args.join(" ")),
        &format!("crunch ruff {}", args.join(" ")),
        raw,
        &filtered,
    );

    if exit_code != 0 {
        return Ok(exit_code);
    }

    Ok(0)
}

pub fn run(args: &[String], verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();

    // Detect subcommand: check, format, or version
    let is_check = args.is_empty()
        || args[0] == "check"
        || (!args[0].starts_with('-') && args[0] != "format" && args[0] != "version");

    let is_format = args.iter().any(|a| a == "format");

    let mut cmd = resolved_command("ruff");

    if is_check {
        // Force JSON output for check command
        if !args.contains(&"--output-format".to_string()) {
            cmd.arg("check").arg("--output-format=json");
        } else {
            cmd.arg("check");
        }

        // Add user arguments (skip "check" if it was the first arg)
        let start_idx = if !args.is_empty() && args[0] == "check" {
            1
        } else {
            0
        };
        for arg in &args[start_idx..] {
            cmd.arg(arg);
        }

        // Default to current directory if no path specified
        if args
            .iter()
            .skip(start_idx)
            .all(|a| a.starts_with('-') || a.contains('='))
        {
            cmd.arg(".");
        }
    } else {
        // Format or other commands - pass through
        for arg in args {
            cmd.arg(arg);
        }
    }

    if verbose > 0 {
        eprintln!("Running: ruff {}", args.join(" "));
    }

    let output = cmd
        .output()
        .context("Failed to run ruff. Is it installed? Try: pip install ruff")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    let filtered = if is_check && !stdout.trim().is_empty() {
        filter_ruff_check_json(&stdout)
    } else if is_format {
        filter_ruff_format(&raw)
    } else {
        // Fallback for other commands (version, etc.)
        raw.trim().to_string()
    };

    println!("{}", filtered);

    timer.track(
        &format!("ruff {}", args.join(" ")),
        &format!("crunch ruff {}", args.join(" ")),
        &raw,
        &filtered,
    );

    // Preserve exit code for CI/CD
    if !output.status.success() {
        return Ok(output.status.code().unwrap_or(1));
    }

    Ok(0)
}

/// Filter ruff check JSON output - group by rule and file
pub fn filter_ruff_check_json(output: &str) -> String {
    let diagnostics: Result<Vec<RuffDiagnostic>, _> = serde_json::from_str(output);

    let diagnostics = match diagnostics {
        Ok(d) => d,
        Err(e) => {
            // Fallback if JSON parsing fails
            return format!(
                "Ruff check (JSON parse failed: {})\n{}",
                e,
                truncate(output, config::limits().passthrough_max_chars)
            );
        }
    };

    if diagnostics.is_empty() {
        return "Ruff: No issues found".to_string();
    }

    let total_issues = diagnostics.len();
    let fixable_count = diagnostics.iter().filter(|d| d.fix.is_some()).count();

    // Count unique files
    let unique_files: std::collections::HashSet<_> =
        diagnostics.iter().map(|d| &d.filename).collect();
    let total_files = unique_files.len();

    // Group by rule code
    let mut by_rule: HashMap<String, usize> = HashMap::new();
    for diag in &diagnostics {
        *by_rule.entry(diag.code.clone()).or_insert(0) += 1;
    }

    // Group by file
    let mut by_file: HashMap<&str, usize> = HashMap::new();
    for diag in &diagnostics {
        *by_file.entry(&diag.filename).or_insert(0) += 1;
    }

    let mut file_counts: Vec<_> = by_file.iter().collect();
    file_counts.sort_by(|a, b| b.1.cmp(a.1));

    // Build output
    let mut result = String::new();
    result.push_str(&format!(
        "Ruff: {} issues in {} files",
        total_issues, total_files
    ));

    if fixable_count > 0 {
        result.push_str(&format!(" ({} fixable)", fixable_count));
    }
    result.push('\n');
    result.push_str("═══════════════════════════════════════\n");

    // Show top rules
    let mut rule_counts: Vec<_> = by_rule.iter().collect();
    rule_counts.sort_by(|a, b| b.1.cmp(a.1));

    if !rule_counts.is_empty() {
        result.push_str("Top rules:\n");
        for (rule, count) in rule_counts.iter().take(10) {
            result.push_str(&format!("  {} ({}x)\n", rule, count));
        }
        result.push('\n');
    }

    // Show diagnostics grouped by file — include line numbers and messages
    // so the AI can locate and fix each issue.
    result.push_str("Issues:\n");
    for (file, _count) in file_counts.iter().take(10) {
        let short_path = compact_path(file);
        let mut file_diags: Vec<&RuffDiagnostic> = diagnostics
            .iter()
            .filter(|d| &d.filename == *file)
            .collect();
        file_diags.sort_by_key(|d| d.location.row);

        for (i, diag) in file_diags.iter().enumerate().take(15) {
            result.push_str(&format!(
                "  {}:{}: {} {}\n",
                if i == 0 {
                    short_path.to_string()
                } else {
                    " ".repeat(short_path.len())
                },
                diag.location.row,
                diag.code,
                diag.message,
            ));
        }
        if file_diags.len() > 15 {
            result.push_str(&format!(
                "  {}  ... +{} more\n",
                " ".repeat(short_path.len()),
                file_diags.len() - 15
            ));
        }
    }

    if file_counts.len() > 10 {
        result.push_str(&format!("\n... +{} more files\n", file_counts.len() - 10));
    }

    if fixable_count > 0 {
        result.push_str(&format!(
            "\n[hint] Run `ruff check --fix` to auto-fix {} issues\n",
            fixable_count
        ));
    }

    result.trim().to_string()
}

/// Filter ruff format output - show files that need formatting
pub fn filter_ruff_format(output: &str) -> String {
    let mut files_to_format: Vec<String> = Vec::new();
    let mut files_checked = 0;

    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        // Count "would reformat" lines (check mode) - case insensitive
        if lower.contains("would reformat:") {
            // Extract filename from "Would reformat: path/to/file.py"
            if let Some(filename) = trimmed.split(':').nth(1) {
                files_to_format.push(filename.trim().to_string());
            }
        }

        // Count total checked files - look for patterns like "3 files left unchanged"
        if lower.contains("left unchanged") {
            // Find "X file(s) left unchanged" pattern specifically
            // Split by comma to handle "2 files would be reformatted, 3 files left unchanged"
            let parts: Vec<&str> = trimmed.split(',').collect();
            for part in parts {
                let part_lower = part.to_lowercase();
                if part_lower.contains("left unchanged") {
                    let words: Vec<&str> = part.split_whitespace().collect();
                    // Look for number before "file" or "files"
                    for (i, word) in words.iter().enumerate() {
                        if (word == &"file" || word == &"files") && i > 0 {
                            if let Ok(count) = words[i - 1].parse::<usize>() {
                                files_checked = count;
                                break;
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    let output_lower = output.to_lowercase();

    // Check if all files are formatted
    if files_to_format.is_empty() && output_lower.contains("left unchanged") {
        return "Ruff format: All files formatted correctly".to_string();
    }

    let mut result = String::new();

    if output_lower.contains("would reformat") {
        // Check mode: show files that need formatting
        if files_to_format.is_empty() {
            result.push_str("Ruff format: All files formatted correctly\n");
        } else {
            result.push_str(&format!(
                "Ruff format: {} files need formatting\n",
                files_to_format.len()
            ));
            result.push_str("═══════════════════════════════════════\n");

            for (i, file) in files_to_format.iter().take(10).enumerate() {
                result.push_str(&format!("{}. {}\n", i + 1, compact_path(file)));
            }

            if files_to_format.len() > 10 {
                result.push_str(&format!(
                    "\n... +{} more files\n",
                    files_to_format.len() - 10
                ));
            }

            if files_checked > 0 {
                result.push_str(&format!("\n{} files already formatted\n", files_checked));
            }

            result.push_str("\n[hint] Run `ruff format` to format these files\n");
        }
    } else {
        // Write mode or other output - show summary
        result.push_str(output.trim());
    }

    result.trim().to_string()
}

/// Compact file path (remove common prefixes)
fn compact_path(path: &str) -> String {
    let path = path.replace('\\', "/");

    if let Some(pos) = path.rfind("/src/") {
        format!("src/{}", &path[pos + 5..])
    } else if let Some(pos) = path.rfind("/lib/") {
        format!("lib/{}", &path[pos + 5..])
    } else if let Some(pos) = path.rfind("/tests/") {
        format!("tests/{}", &path[pos + 7..])
    } else if let Some(pos) = path.rfind('/') {
        path[pos + 1..].to_string()
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_ruff_check_no_issues() {
        let output = "[]";
        let result = filter_ruff_check_json(output);
        assert!(result.contains("Ruff"));
        assert!(result.contains("No issues found"));
    }

    #[test]
    fn test_filter_ruff_check_with_issues() {
        let output = r#"[
  {
    "code": "F401",
    "message": "`os` imported but unused",
    "location": {"row": 1, "column": 8},
    "end_location": {"row": 1, "column": 10},
    "filename": "src/main.py",
    "fix": {"applicability": "safe"}
  },
  {
    "code": "F401",
    "message": "`sys` imported but unused",
    "location": {"row": 2, "column": 8},
    "end_location": {"row": 2, "column": 11},
    "filename": "src/main.py",
    "fix": null
  },
  {
    "code": "E501",
    "message": "Line too long (100 > 88 characters)",
    "location": {"row": 10, "column": 89},
    "end_location": {"row": 10, "column": 100},
    "filename": "src/utils.py",
    "fix": null
  }
]"#;
        let result = filter_ruff_check_json(output);
        assert!(result.contains("3 issues"));
        assert!(result.contains("2 files"));
        assert!(result.contains("1 fixable"));
        assert!(result.contains("F401"));
        assert!(result.contains("E501"));
        assert!(result.contains("main.py"));
        assert!(result.contains("utils.py"));
        // Must include line numbers and messages so AI can fix issues
        assert!(
            result.contains(":1:"),
            "should include line numbers: {}",
            result
        );
        assert!(
            result.contains("imported but unused"),
            "should include error messages: {}",
            result
        );
        assert!(
            result.contains("Line too long"),
            "should include error messages: {}",
            result
        );
    }

    #[test]
    fn test_filter_ruff_check_shows_locations_and_messages() {
        let output = r#"[
  {
    "code": "F821",
    "message": "Undefined name `missing_func`",
    "location": {"row": 12, "column": 5},
    "end_location": {"row": 12, "column": 17},
    "filename": "apps/explorer/main.py",
    "fix": null
  },
  {
    "code": "F821",
    "message": "Undefined name `other_func`",
    "location": {"row": 25, "column": 10},
    "end_location": {"row": 25, "column": 20},
    "filename": "apps/explorer/main.py",
    "fix": null
  }
]"#;
        let result = filter_ruff_check_json(output);
        // AI needs these to fix the issues:
        assert!(
            result.contains("main.py:12"),
            "need line number: {}",
            result
        );
        assert!(
            result.contains(":25:"),
            "need line number for 2nd issue: {}",
            result
        );
        assert!(
            result.contains("Undefined name `missing_func`"),
            "need message: {}",
            result
        );
        assert!(
            result.contains("Undefined name `other_func`"),
            "need message: {}",
            result
        );
    }

    #[test]
    fn test_filter_ruff_format_all_formatted() {
        let output = "5 files left unchanged";
        let result = filter_ruff_format(output);
        assert!(result.contains("Ruff format"));
        assert!(result.contains("All files formatted correctly"));
    }

    #[test]
    fn test_filter_ruff_format_needs_formatting() {
        let output = r#"Would reformat: src/main.py
Would reformat: tests/test_utils.py
2 files would be reformatted, 3 files left unchanged"#;
        let result = filter_ruff_format(output);
        assert!(result.contains("2 files need formatting"));
        assert!(result.contains("main.py"));
        assert!(result.contains("test_utils.py"));
        assert!(result.contains("3 files already formatted"));
    }

    #[test]
    fn test_run_with_output_tees_on_failure() {
        // tee_raw_scoped writes the RAW output to /tmp/crunch/{project}/ruff-*.log.
        //
        // We assert by UNIQUE-CONTENT match, not by a before/after filename diff:
        // the log name is `ruff-{scope}-{timestamp}.log` at 1-second granularity
        // and /tmp/crunch/{project}/ persists across runs, so a same-second
        // collision (another run or a real ruff invocation) reuses the filename
        // and a set-diff would flakily see "no new file". A unique marker in the
        // input lets us find our tee file regardless of filename collisions.
        let project = crate::tee::detect_project_name();
        let log_dir = std::path::PathBuf::from("/tmp/crunch").join(&project);
        let _ = std::fs::create_dir_all(&log_dir);

        let marker = format!("ZZ_TEE_MARKER_{}", std::process::id());
        // Large enough (>500 chars) to trigger tee; carries the unique marker.
        let big_output = format!(
            "[\n  {{\"code\":\"F401\",\"message\":\"unused import\",\"location\":{{\"row\":1,\"column\":0}},\"end_location\":{{\"row\":1,\"column\":0}},\"filename\":\"src/{marker}.py\",\"fix\":null}}\n]\n{}",
            " ".repeat(600)
        );
        let args: Vec<String> = vec!["check".to_string()];

        // exit_code=0 to avoid process::exit killing the test runner.
        let _ = super::run_with_output(&big_output, &args, 0, 0);

        // Some ruff-*.log in the dir must now contain our marker.
        let found = std::fs::read_dir(&log_dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with("ruff-"))
                    .any(|e| {
                        std::fs::read_to_string(e.path())
                            .map(|c| c.contains(&marker))
                            .unwrap_or(false)
                    })
            })
            .unwrap_or(false);
        assert!(
            found,
            "Expected a ruff tee file containing {marker} in {:?} — run_with_output should tee raw output",
            log_dir
        );
    }

    #[test]
    fn test_compact_path() {
        assert_eq!(
            compact_path("/Users/foo/project/src/main.py"),
            "src/main.py"
        );
        assert_eq!(compact_path("/home/user/app/lib/utils.py"), "lib/utils.py");
        assert_eq!(
            compact_path("C:\\Users\\foo\\project\\tests\\test.py"),
            "tests/test.py"
        );
        assert_eq!(compact_path("relative/file.py"), "file.py");
    }
}
