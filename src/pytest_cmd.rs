use crate::tracking;
use crate::utils::{resolved_command, strip_ansi, tool_exists, truncate};
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use regex::Regex;

#[derive(Debug, PartialEq)]
enum ParseState {
    Header,
    TestProgress,
    Errors,
    Failures,
    Summary,
}

/// All pytest summary categories parsed from the final summary line.
#[derive(Debug, Default, PartialEq)]
struct PytestResult {
    passed: usize,
    failed: usize,
    skipped: usize,
    errors: usize,
    xfailed: usize,
    xpassed: usize,
    deselected: usize,
    warnings: usize,
    no_tests_ran: bool,
    summary_found: bool,
}

lazy_static! {
    /// Matches `<number> <category>` pairs in pytest summary lines.
    /// E.g., "3 failed", "42 passed", "1 error", "2 warnings"
    static ref SUMMARY_PAIR: Regex = Regex::new(r"(\d+)\s+(\w+)").unwrap();

    /// Detects a pytest summary line by the duration pattern `in X.XXs` at end of line.
    /// Anchored to EOL to avoid false positives on tracebacks like "Timeout in 3.5s during test".
    /// Works with and without `===` delimiters (quiet mode).
    static ref SUMMARY_LINE_DETECT: Regex = Regex::new(r"in\s+\d+\.\d+s\s*=*\s*$").unwrap();
}

/// Parse a pytest summary line into a PytestResult.
///
/// Handles all formats:
/// - `=== 5 passed in 0.50s ===`
/// - `=== 3 failed, 2 passed, 1 xfailed, 1 error in 0.12s ===`
/// - `5 passed in 0.50s` (quiet mode, no delimiters)
/// - `=== no tests ran in 0.00s ===`
/// - `=== 2 failed, 5 deselected in 0.12s ===`
fn parse_summary(line: &str) -> PytestResult {
    let mut result = PytestResult::default();

    if line.is_empty() {
        return result;
    }

    // Check for "no tests ran" explicitly
    if line.contains("no tests ran") {
        result.no_tests_ran = true;
        result.summary_found = true;
        return result;
    }

    // Extract all <number> <word> pairs
    for cap in SUMMARY_PAIR.captures_iter(line) {
        let count: usize = match cap[1].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let category = &cap[2];

        match category {
            "passed" => result.passed = count,
            "failed" => result.failed = count,
            "skipped" => result.skipped = count,
            // pytest uses both "error" and "errors"
            "error" | "errors" => result.errors = count,
            "xfailed" => result.xfailed = count,
            "xpassed" => result.xpassed = count,
            "deselected" => result.deselected = count,
            // pytest uses both "warning" and "warnings"
            "warning" | "warnings" => result.warnings = count,
            _ => {} // ignore unknown categories (future-proof)
        }
    }

    // Mark as found if we extracted any meaningful data
    if result.passed > 0
        || result.failed > 0
        || result.skipped > 0
        || result.errors > 0
        || result.xfailed > 0
        || result.xpassed > 0
        || result.deselected > 0
        || result.warnings > 0
    {
        result.summary_found = true;
    }

    result
}

/// Detect whether a line is a pytest summary line (contains duration pattern).
fn is_summary_line(line: &str) -> bool {
    SUMMARY_LINE_DETECT.is_match(line)
}

/// Process pre-captured output (e.g. from mise) through the pytest parser.
/// Handles filtering, tee, tracking, and exit code — everything `run()` does
/// except executing the command.
pub fn run_with_output(raw: &str, args: &[String], exit_code: i32, verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("crunch: parsing pytest output ({} bytes)", raw.len());
    }

    let filtered = filter_pytest_output(raw, Some(exit_code));

    if let Some(hint) = crate::tee::tee_and_hint_scoped(raw, "pytest", args, exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    timer.track(
        &format!("pytest {}", args.join(" ")),
        &format!("crunch pytest {}", args.join(" ")),
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

    // Try to detect pytest command (could be "pytest", "python -m pytest", etc.)
    let mut cmd = if tool_exists("pytest") {
        resolved_command("pytest")
    } else {
        // Fallback to python -m pytest
        let mut c = resolved_command("python");
        c.arg("-m").arg("pytest");
        c
    };

    // Force short traceback and quiet mode for compact output
    let has_tb_flag = args.iter().any(|a| a.starts_with("--tb"));
    let has_quiet_flag = args.iter().any(|a| a == "-q" || a == "--quiet");

    if !has_tb_flag {
        cmd.arg("--tb=short");
    }
    if !has_quiet_flag {
        cmd.arg("-q");
    }

    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: pytest --tb=short -q {}", args.join(" "));
    }

    let output = cmd
        .output()
        .context("Failed to run pytest. Is it installed? Try: pip install pytest")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    // Compute exit code BEFORE filtering so the parser can use it as ground truth
    let exit_code = output
        .status
        .code()
        .unwrap_or(if output.status.success() { 0 } else { 1 });

    let filtered = filter_pytest_output(&stdout, Some(exit_code));

    if let Some(hint) = crate::tee::tee_and_hint_scoped(&raw, "pytest", args, exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    // Include stderr if present (import errors, etc.)
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim());
    }

    timer.track(
        &format!("pytest {}", args.join(" ")),
        &format!("crunch pytest {}", args.join(" ")),
        &raw,
        &filtered,
    );

    // Preserve exit code for CI/CD
    if !output.status.success() {
        return Ok(exit_code);
    }

    Ok(0)
}

/// Parse pytest output using state machine.
///
/// `exit_code` is used as ground truth to disambiguate when the summary line
/// is missing or unparseable. Pytest exit codes: 0=OK, 1=FAILED, 2=INTERRUPTED,
/// 3=INTERNAL_ERROR, 4=USAGE_ERROR, 5=NO_TESTS_COLLECTED.
/// Filter pre-captured pytest output for the PostToolUse compressor.
/// Thin wrapper over [`filter_pytest_output`] that takes a concrete exit code.
pub fn filter_for_hook(raw: &str, exit_code: i32) -> String {
    filter_pytest_output(raw, Some(exit_code))
}

fn filter_pytest_output(output: &str, exit_code: Option<i32>) -> String {
    // Strip ANSI codes so colored output doesn't break detection
    let clean = strip_ansi(output);

    let mut state = ParseState::Header;
    let mut failures: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut current_block: Vec<String> = Vec::new();
    let mut summary_line = String::new();

    for line in clean.lines() {
        let trimmed = line.trim();

        // State transitions — check section headers first
        if trimmed.starts_with("===") && trimmed.contains("test session starts") {
            state = ParseState::Header;
            continue;
        } else if trimmed.starts_with("===") && trimmed.contains("ERRORS") {
            state = ParseState::Errors;
            if !current_block.is_empty() {
                errors.push(current_block.join("\n"));
                current_block.clear();
            }
            continue;
        } else if trimmed.starts_with("===") && trimmed.contains("FAILURES") {
            // Flush any pending error block before switching to failures
            if !current_block.is_empty() {
                errors.push(current_block.join("\n"));
                current_block.clear();
            }
            state = ParseState::Failures;
            continue;
        } else if trimmed.starts_with("===") && trimmed.contains("short test summary") {
            // Flush current block BEFORE changing state — otherwise flush_block
            // sees Summary state and discards the block via the _ => {} arm.
            flush_block(&mut current_block, &mut failures, &mut errors, &state);
            state = ParseState::Summary;
            continue;
        } else if is_summary_line(trimmed) {
            // Detect summary by duration pattern — works with and without === delimiters
            summary_line = trimmed.to_string();
            continue;
        }

        // Handle !!!!! interruption markers
        if trimmed.starts_with("!!!!") {
            continue;
        }

        // Process based on state
        match state {
            ParseState::Header => {
                // Transition to TestProgress on test-like lines (don't require "collected")
                if trimmed.starts_with("collected")
                    || (trimmed.contains(".py")
                        && (trimmed.contains('%')
                            || trimmed.contains("PASSED")
                            || trimmed.contains("FAILED")))
                    || trimmed.starts_with("[gw")
                {
                    state = ParseState::TestProgress;
                }
            }
            ParseState::TestProgress => {
                // Absorb progress lines — we don't need them in output
            }
            ParseState::Errors => {
                collect_block(trimmed, &mut current_block, &mut errors);
            }
            ParseState::Failures => {
                collect_block(trimmed, &mut current_block, &mut failures);
            }
            ParseState::Summary => {
                // Only collect summary FAILED/ERROR lines if we don't already
                // have detailed blocks from the FAILURES/ERRORS sections.
                // Otherwise we get duplicates.
                if trimmed.starts_with("FAILED") && failures.is_empty() {
                    failures.push(trimmed.to_string());
                } else if trimmed.starts_with("ERROR") && errors.is_empty() {
                    errors.push(trimmed.to_string());
                }
            }
        }
    }

    // Save last block
    flush_block(&mut current_block, &mut failures, &mut errors, &state);

    // Parse summary line into structured result
    let result = if summary_line.is_empty() {
        PytestResult::default()
    } else {
        parse_summary(&summary_line)
    };

    // Build output using exit code as ground truth
    let output = build_pytest_summary(&result, &failures, &errors, exit_code);

    // Graceful fallback: if parser produced empty/whitespace output, show raw head+tail
    if output.trim().is_empty() {
        return fallback_raw_output(&clean);
    }

    output
}

/// Collect lines into the current block, splitting on `___` separators.
fn collect_block(trimmed: &str, current_block: &mut Vec<String>, target: &mut Vec<String>) {
    if trimmed.starts_with("___") {
        if !current_block.is_empty() {
            target.push(current_block.join("\n"));
            current_block.clear();
        }
        current_block.push(trimmed.to_string());
    } else if !trimmed.is_empty() && !trimmed.starts_with("===") {
        current_block.push(trimmed.to_string());
    }
}

/// Flush the current block to the appropriate target based on state.
fn flush_block(
    current_block: &mut Vec<String>,
    failures: &mut Vec<String>,
    errors: &mut Vec<String>,
    state: &ParseState,
) {
    if current_block.is_empty() {
        return;
    }
    match state {
        ParseState::Errors => errors.push(current_block.join("\n")),
        ParseState::Failures => failures.push(current_block.join("\n")),
        _ => {} // discard blocks from other states
    }
    current_block.clear();
}

/// Fallback: return first 10 + last 15 lines of raw output.
fn fallback_raw_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let mut result = String::from("[crunch: parser fallback]\n");

    if lines.len() <= 25 {
        result.push_str(&lines.join("\n"));
    } else {
        for line in &lines[..10] {
            result.push_str(line);
            result.push('\n');
        }
        result.push_str(&format!(
            "\n... ({} lines omitted) ...\n\n",
            lines.len() - 25
        ));
        for line in &lines[lines.len() - 15..] {
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Build the compact pytest summary using structured result + exit code decision matrix.
fn build_pytest_summary(
    result: &PytestResult,
    failures: &[String],
    errors: &[String],
    exit_code: Option<i32>,
) -> String {
    // Decision matrix: summary_found × exit_code
    if result.summary_found {
        if result.no_tests_ran {
            return "Pytest: No tests ran".to_string();
        }
        return build_normal_summary(result, failures, errors);
    }

    // Summary NOT found — use exit code as ground truth
    match exit_code {
        Some(0) => "Pytest: All tests passed".to_string(),
        Some(1) => {
            // Tests failed but summary not captured
            let mut out = "Pytest: Tests failed (summary not captured)".to_string();
            if !failures.is_empty() || !errors.is_empty() {
                out.push('\n');
                out.push_str("═══════════════════════════════════════");
                append_issues(&mut out, failures, errors);
            }
            out
        }
        Some(2) => "Pytest: Interrupted".to_string(),
        Some(3) => "Pytest: Internal error".to_string(),
        Some(4) => "Pytest: Usage error".to_string(),
        Some(5) => "Pytest: No tests collected".to_string(),
        _ => {
            // Unknown exit code or None — show what we have
            if !failures.is_empty() || !errors.is_empty() {
                let mut out = "Pytest: Completed (exit code unknown)".to_string();
                out.push('\n');
                out.push_str("═══════════════════════════════════════");
                append_issues(&mut out, failures, errors);
                out
            } else {
                String::new() // will trigger fallback
            }
        }
    }
}

/// Build normal summary when we have parsed counts.
fn build_normal_summary(result: &PytestResult, failures: &[String], errors: &[String]) -> String {
    // All passed, no issues
    if result.failed == 0 && result.errors == 0 && result.passed > 0 {
        let mut line = format!("Pytest: {} passed", result.passed);
        // Still mention skipped/deselected/warnings if present
        append_minor_categories(&mut line, result);
        return line;
    }

    // Build header with all non-zero categories
    let mut header = String::from("Pytest: ");
    let mut parts: Vec<String> = Vec::new();

    if result.passed > 0 {
        parts.push(format!("{} passed", result.passed));
    }
    if result.failed > 0 {
        parts.push(format!("{} failed", result.failed));
    }
    if result.errors > 0 {
        parts.push(format!("{} errors", result.errors));
    }
    if result.skipped > 0 {
        parts.push(format!("{} skipped", result.skipped));
    }
    if result.xfailed > 0 {
        parts.push(format!("{} xfailed", result.xfailed));
    }
    if result.xpassed > 0 {
        parts.push(format!("{} xpassed", result.xpassed));
    }
    if result.deselected > 0 {
        parts.push(format!("{} deselected", result.deselected));
    }
    if result.warnings > 0 {
        parts.push(format!("{} warnings", result.warnings));
    }

    header.push_str(&parts.join(", "));

    if failures.is_empty() && errors.is_empty() {
        return header;
    }

    let mut out = header;
    out.push('\n');
    out.push_str("═══════════════════════════════════════");
    append_issues(&mut out, failures, errors);
    out.trim().to_string()
}

/// Append minor categories (skipped, deselected, warnings) to a passing summary line.
fn append_minor_categories(line: &mut String, result: &PytestResult) {
    if result.skipped > 0 {
        line.push_str(&format!(", {} skipped", result.skipped));
    }
    if result.xfailed > 0 {
        line.push_str(&format!(", {} xfailed", result.xfailed));
    }
    if result.xpassed > 0 {
        line.push_str(&format!(", {} xpassed", result.xpassed));
    }
    if result.deselected > 0 {
        line.push_str(&format!(", {} deselected", result.deselected));
    }
    if result.warnings > 0 {
        line.push_str(&format!(", {} warnings", result.warnings));
    }
}

/// Append formatted errors and failures to the output string.
fn append_issues(out: &mut String, failures: &[String], errors: &[String]) {
    if !errors.is_empty() {
        out.push_str("\n\nErrors:\n");
        for (i, error) in errors.iter().take(3).enumerate() {
            format_issue(out, i, error, "ERROR");
        }
        if errors.len() > 3 {
            out.push_str(&format!("\n... +{} more errors\n", errors.len() - 3));
        }
    }

    if !failures.is_empty() {
        out.push_str("\n\nFailures:\n");
        for (i, failure) in failures.iter().take(5).enumerate() {
            format_issue(out, i, failure, "FAIL");
        }
        if failures.len() > 5 {
            out.push_str(&format!("\n... +{} more failures\n", failures.len() - 5));
        }
    }
}

/// Format a single error or failure block for display.
fn format_issue(out: &mut String, index: usize, issue: &str, label: &str) {
    let lines: Vec<&str> = issue.lines().collect();

    if let Some(first_line) = lines.first() {
        if first_line.starts_with("___") {
            let name = first_line.trim_matches('_').trim();
            out.push_str(&format!("{}. [{}] {}\n", index + 1, label, name));
        } else if first_line.starts_with("FAILED") || first_line.starts_with("ERROR") {
            let parts: Vec<&str> = first_line.split(" - ").collect();
            if let Some(test_path) = parts.first() {
                let name = test_path
                    .trim_start_matches("FAILED ")
                    .trim_start_matches("ERROR ");
                out.push_str(&format!("{}. [{}] {}\n", index + 1, label, name));
            }
            if parts.len() > 1 {
                out.push_str(&format!("     {}\n", truncate(parts[1], 100)));
            }
            return;
        } else {
            out.push_str(&format!(
                "{}. [{}] {}\n",
                index + 1,
                label,
                truncate(first_line, 100)
            ));
        }
    }

    // Show relevant error lines
    if lines.len() > 1 {
        let mut relevant_count = 0;
        for line in &lines[1..] {
            let lt = line.trim();
            let ll = lt.to_lowercase();
            let is_relevant = lt.starts_with('>')
                || lt.starts_with('E')
                || ll.contains("assert")
                || ll.contains("error")
                || lt.contains(".py:");

            if is_relevant && relevant_count < 3 {
                out.push_str(&format!("     {}\n", truncate(line, 100)));
                relevant_count += 1;
            }
        }
    }

    if index < 4 {
        // Add spacing between items (not after last)
        out.push('\n');
    }
}

/// Legacy parse_summary_line — kept for backward compatibility with old tests.
#[cfg(test)]
fn parse_summary_line(summary: &str) -> (usize, usize, usize) {
    let r = parse_summary(summary);
    (r.passed, r.failed, r.skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_pytest_all_pass() {
        let output = r#"=== test session starts ===
platform darwin -- Python 3.11.0
collected 5 items

tests/test_foo.py .....                                            [100%]

=== 5 passed in 0.50s ==="#;

        let result = filter_pytest_output(output, Some(0));
        assert!(result.contains("Pytest"));
        assert!(result.contains("5 passed"));
    }

    #[test]
    fn test_filter_pytest_with_failures() {
        let output = r#"=== test session starts ===
collected 5 items

tests/test_foo.py ..F..                                            [100%]

=== FAILURES ===
___ test_something ___

    def test_something():
>       assert False
E       assert False

tests/test_foo.py:10: AssertionError

=== short test summary info ===
FAILED tests/test_foo.py::test_something - assert False
=== 4 passed, 1 failed in 0.50s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(result.contains("4 passed, 1 failed"));
        assert!(result.contains("test_something"));
        assert!(result.contains("assert False"));
    }

    #[test]
    fn test_filter_pytest_multiple_failures() {
        let output = r#"=== test session starts ===
collected 3 items

tests/test_foo.py FFF                                              [100%]

=== FAILURES ===
___ test_one ___
E   AssertionError: expected 5

___ test_two ___
E   ValueError: invalid value

=== short test summary info ===
FAILED tests/test_foo.py::test_one - AssertionError: expected 5
FAILED tests/test_foo.py::test_two - ValueError: invalid value
FAILED tests/test_foo.py::test_three - KeyError
=== 3 failed in 0.20s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(result.contains("3 failed"));
        assert!(result.contains("test_one"));
        assert!(result.contains("test_two"));
        assert!(result.contains("expected 5"));
    }

    #[test]
    fn test_filter_pytest_no_tests() {
        let output = r#"=== test session starts ===
collected 0 items

=== no tests ran in 0.00s ==="#;

        let result = filter_pytest_output(output, Some(5));
        assert!(result.contains("No tests ran"));
    }

    #[test]
    fn test_parse_summary_line_legacy() {
        assert_eq!(parse_summary_line("=== 5 passed in 0.50s ==="), (5, 0, 0));
        assert_eq!(
            parse_summary_line("=== 4 passed, 1 failed in 0.50s ==="),
            (4, 1, 0)
        );
        assert_eq!(
            parse_summary_line("=== 3 passed, 1 failed, 2 skipped in 1.0s ==="),
            (3, 1, 2)
        );
    }

    // ── Phase 4: Rendering improvements tests ──

    #[test]
    fn test_render_all_categories_in_header() {
        let output = r#"=== test session starts ===
collected 50 items

=== 42 passed, 3 failed, 2 skipped, 1 xfailed, 1 xpassed, 5 deselected, 2 warnings in 5.00s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(result.contains("42 passed"), "Missing passed: {}", result);
        assert!(result.contains("3 failed"), "Missing failed: {}", result);
        assert!(result.contains("2 skipped"), "Missing skipped: {}", result);
        assert!(result.contains("1 xfailed"), "Missing xfailed: {}", result);
        assert!(result.contains("1 xpassed"), "Missing xpassed: {}", result);
        assert!(
            result.contains("5 deselected"),
            "Missing deselected: {}",
            result
        );
        assert!(
            result.contains("2 warnings"),
            "Missing warnings: {}",
            result
        );
    }

    #[test]
    fn test_render_passing_with_skipped() {
        let output = r#"=== test session starts ===
=== 10 passed, 3 skipped in 1.00s ==="#;

        let result = filter_pytest_output(output, Some(0));
        assert!(result.contains("10 passed"), "Missing passed: {}", result);
        assert!(result.contains("3 skipped"), "Missing skipped: {}", result);
    }

    #[test]
    fn test_render_errors_separate_from_failures() {
        let output = r#"=== test session starts ===
collected 5 items

=== ERRORS ===
_______ ERROR at setup of test_db _______
fixture 'db' not found

=== FAILURES ===
_______ test_math _______
E   assert 1 == 2

=== short test summary info ===
ERROR tests/test_db.py::test_db
FAILED tests/test_math.py::test_math - assert 1 == 2
============= 1 failed, 3 passed, 1 error in 0.50s =============="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("Errors:"),
            "Should have Errors section: {}",
            result
        );
        assert!(
            result.contains("Failures:"),
            "Should have Failures section: {}",
            result
        );
    }

    #[test]
    fn test_render_fallback_on_empty_output() {
        let result = filter_pytest_output("", None);
        // Empty output + no exit code → build_pytest_summary returns "" → fallback_raw_output
        assert!(
            result.contains("parser fallback"),
            "Should trigger parser fallback, got: '{}'",
            result
        );
    }

    #[test]
    fn test_render_passing_with_warnings_and_deselected() {
        let output = "=== 50 passed, 10 deselected, 3 warnings in 2.50s ===";
        let result = filter_pytest_output(output, Some(0));
        assert!(result.contains("50 passed"), "Missing passed: {}", result);
        assert!(
            result.contains("10 deselected"),
            "Missing deselected: {}",
            result
        );
        assert!(
            result.contains("3 warnings"),
            "Missing warnings: {}",
            result
        );
    }

    // ── Review fix: flush_block regression test ──

    #[test]
    fn test_last_failure_block_not_dropped_before_summary() {
        // Regression: the last failure block (no following ___ separator)
        // was silently discarded because flush_block was called AFTER
        // state changed to Summary.
        let output = r#"=== test session starts ===
collected 1 item

=== FAILURES ===
_______________________________ test_only_failure ________________________________

    def test_only_failure():
>       assert False
E       AssertionError

tests/test_foo.py:3: AssertionError

=== short test summary info ===
FAILED tests/test_foo.py::test_only_failure - AssertionError
=== 1 failed in 0.02s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("test_only_failure"),
            "Last failure block must not be dropped, got: {}",
            result
        );
        assert!(
            result.contains("AssertionError") || result.contains("assert False"),
            "Failure detail must be preserved, got: {}",
            result
        );
    }

    #[test]
    fn test_summary_line_no_false_positive_on_traceback() {
        // Regression: "in 3.5s" inside a traceback should NOT be detected as summary
        let traceback_line = "    Timeout occurred in 3.5s during connection";
        assert!(
            !is_summary_line(traceback_line),
            "Traceback line should not match as summary"
        );
        // But real summary still works
        assert!(is_summary_line("=== 5 passed in 0.50s ==="));
        assert!(is_summary_line("1 failed in 0.02s"));
    }

    // ── Phase 3: Robust state machine tests ──

    #[test]
    fn test_errors_section_collected_separately() {
        let output = r#"=== test session starts ===
collected 3 items

=== ERRORS ===
_______ ERROR collecting tests/test_broken.py _______
ImportError: cannot import name 'foo'

=== short test summary info ===
ERROR tests/test_broken.py
============= 1 error in 0.05s =============="#;

        let result = filter_pytest_output(output, Some(2));
        assert!(
            result.contains("error") || result.contains("ERROR"),
            "Should mention errors, got: {}",
            result
        );
    }

    #[test]
    fn test_mixed_errors_and_failures() {
        let output = r#"=== test session starts ===
collected 7 items

=== ERRORS ===
_______ ERROR at setup of test_root _______
fixture 'db' not found

=== FAILURES ===
_______ test_a1 _______
E   AssertionError: expected 5

=== short test summary info ===
FAILED a/test_db.py::test_a1 - AssertionError
ERROR b/test_error.py::test_root
============= 1 failed, 2 passed, 1 error in 0.12s =============="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("1 failed"),
            "Should show failures: {}",
            result
        );
        assert!(
            result.contains("1 error") || result.contains("Errors:"),
            "Should show errors: {}",
            result
        );
    }

    #[test]
    fn test_xdist_worker_output_detected() {
        let output = r#"=== test session starts ===
[gw0] PASSED tests/test_foo.py::test_one
[gw1] PASSED tests/test_foo.py::test_two
[gw0] FAILED tests/test_bar.py::test_three
=== 2 passed, 1 failed in 3.50s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("2 passed, 1 failed"),
            "Should parse xdist output, got: {}",
            result
        );
    }

    #[test]
    fn test_no_collected_line_still_works() {
        // Some pytest modes don't output "collected X items"
        let output = r#"=== test session starts ===
platform linux -- Python 3.12.0
tests/test_foo.py .....                                            [100%]
=== 5 passed in 0.50s ==="#;

        let result = filter_pytest_output(output, Some(0));
        assert!(
            result.contains("5 passed"),
            "Should work without collected line: {}",
            result
        );
    }

    #[test]
    fn test_interruption_markers_handled() {
        let output = r#"=== test session starts ===
collected 10 items

=== ERRORS ===
_______ ERROR collecting tests/test_broken.py _______
ImportError: cannot import name 'missing'
!!!!!!!!!!!!!!!! Interrupted: 1 error during collection !!!!!!!!!!!!!!!!"#;

        let result = filter_pytest_output(output, Some(2));
        assert!(
            result.contains("Interrupted") || result.contains("error"),
            "Should handle interruption: {}",
            result
        );
    }

    #[test]
    fn test_ansi_codes_stripped() {
        // Simulate colored pytest output
        let output = "\x1b[1m=== test session starts ===\x1b[0m\n\
                       collected 5 items\n\
                       \x1b[32m=== 5 passed in 0.50s ===\x1b[0m";

        let result = filter_pytest_output(output, Some(0));
        assert!(
            result.contains("5 passed"),
            "Should strip ANSI and still parse, got: {}",
            result
        );
    }

    #[test]
    fn test_collection_error_with_exit_code_2() {
        let output = r#"=== test session starts ===
=== ERRORS ===
_______ ERROR collecting tests/broken.py _______
E   ModuleNotFoundError: No module named 'nonexistent'
=== short test summary info ===
ERROR tests/broken.py - ModuleNotFoundError: No module named 'nonexistent'
!!!!!!!!!!!!!!!! Interrupted: 1 error during collection !!!!!!!!!!!!!!!!"#;

        let result = filter_pytest_output(output, Some(2));
        // Should show error info, not "No tests collected"
        assert!(
            !result.contains("No tests collected"),
            "Collection error should not say 'No tests collected': {}",
            result
        );
    }

    // ── Phase 2: Exit-code-aware filtering tests ──

    #[test]
    fn test_exit_code_1_no_summary_shows_failed() {
        // Tests failed but summary line wasn't captured → use exit code
        let output = "some random output\nno summary here";
        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("Tests failed"),
            "Expected 'Tests failed' but got: {}",
            result
        );
    }

    #[test]
    fn test_exit_code_5_no_summary_shows_no_tests_collected() {
        let output = "some output\nno summary";
        let result = filter_pytest_output(output, Some(5));
        assert!(result.contains("No tests collected"));
    }

    #[test]
    fn test_exit_code_0_no_summary_shows_all_passed() {
        let output = "some output without summary";
        let result = filter_pytest_output(output, Some(0));
        assert!(result.contains("All tests passed"));
    }

    #[test]
    fn test_exit_code_2_shows_interrupted() {
        let output = "partial output";
        let result = filter_pytest_output(output, Some(2));
        assert!(result.contains("Interrupted"));
    }

    #[test]
    fn test_exit_code_1_with_summary_uses_summary() {
        // When summary IS found, use it even with exit code 1
        let output = r#"=== test session starts ===
collected 5 items

=== FAILURES ===
___ test_foo ___
E   assert False

=== short test summary info ===
FAILED tests/test_foo.py::test_foo - assert False
=== 4 passed, 1 failed in 0.50s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("4 passed, 1 failed"),
            "Should use parsed summary, got: {}",
            result
        );
        assert!(!result.contains("No tests collected"));
    }

    #[test]
    fn test_p0_fix_large_run_no_summary_exit1_not_no_tests_collected() {
        // THE P0 BUG: large test run, summary not captured, exit code 1
        // Old behavior: "No tests collected" (WRONG)
        // New behavior: "Tests failed (summary not captured)"
        let output = "lots of test output...\ntests running...\nbut no summary line";
        let result = filter_pytest_output(output, Some(1));
        assert!(
            !result.contains("No tests collected"),
            "P0 BUG: must not say 'No tests collected' when exit code is 1, got: {}",
            result
        );
        assert!(result.contains("Tests failed"));
    }

    #[test]
    fn test_exit_code_none_fallback() {
        let output = "line 1\nline 2\nline 3";
        let result = filter_pytest_output(output, None);
        // Should trigger fallback since no summary and no exit code
        assert!(result.contains("parser fallback") || result.contains("line 1"));
    }

    // ── Phase 1: PytestResult + parse_summary tests ──

    #[test]
    fn test_parse_summary_standard() {
        let r = parse_summary("=== 5 passed in 0.50s ===");
        assert_eq!(r.passed, 5);
        assert_eq!(r.failed, 0);
        assert!(r.summary_found);
        assert!(!r.no_tests_ran);
    }

    #[test]
    fn test_parse_summary_mixed_all_categories() {
        let r = parse_summary("=== 3 failed, 2 passed, 1 xfailed, 1 error in 0.12s ===");
        assert_eq!(r.passed, 2);
        assert_eq!(r.failed, 3);
        assert_eq!(r.xfailed, 1);
        assert_eq!(r.errors, 1);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_quiet_mode_no_delimiters() {
        let r = parse_summary("5 passed in 0.50s");
        assert_eq!(r.passed, 5);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_no_tests_ran() {
        let r = parse_summary("=== no tests ran in 0.00s ===");
        assert!(r.no_tests_ran);
        assert!(r.summary_found);
        assert_eq!(r.passed, 0);
        assert_eq!(r.failed, 0);
    }

    #[test]
    fn test_parse_summary_warnings() {
        let r = parse_summary("=== 2 passed, 1 warning in 0.03s ===");
        assert_eq!(r.passed, 2);
        assert_eq!(r.warnings, 1);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_deselected() {
        let r = parse_summary("=== 2 failed, 5 deselected in 0.12s ===");
        assert_eq!(r.failed, 2);
        assert_eq!(r.deselected, 5);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_empty_string() {
        let r = parse_summary("");
        assert_eq!(r, PytestResult::default());
        assert!(!r.summary_found);
    }

    #[test]
    fn test_parse_summary_skipped_xpassed() {
        let r = parse_summary("=== 10 passed, 3 skipped, 2 xpassed in 5.20s ===");
        assert_eq!(r.passed, 10);
        assert_eq!(r.skipped, 3);
        assert_eq!(r.xpassed, 2);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_errors_plural() {
        let r = parse_summary("=== 2 errors in 0.05s ===");
        assert_eq!(r.errors, 2);
        assert!(r.summary_found);
    }

    #[test]
    fn test_parse_summary_warnings_plural() {
        let r = parse_summary("=== 5 passed, 2 warnings in 0.30s ===");
        assert_eq!(r.warnings, 2);
        assert!(r.summary_found);
    }

    #[test]
    fn test_is_summary_line_standard() {
        assert!(is_summary_line("=== 5 passed in 0.50s ==="));
    }

    #[test]
    fn test_is_summary_line_quiet() {
        assert!(is_summary_line("5 passed in 0.50s"));
    }

    #[test]
    fn test_is_summary_line_no_tests() {
        assert!(is_summary_line("=== no tests ran in 0.00s ==="));
    }

    #[test]
    fn test_is_summary_line_not_summary() {
        assert!(!is_summary_line("collected 5 items"));
        assert!(!is_summary_line("=== FAILURES ==="));
        assert!(!is_summary_line("tests/test_foo.py::test_bar PASSED"));
    }

    #[test]
    fn test_is_summary_line_complex() {
        assert!(is_summary_line(
            "============= 3 failed, 2 passed, 1 xfailed, 1 error in 0.12s =============="
        ));
    }

    // ── Phase 6: Integration tests with realistic fixtures ──

    #[test]
    fn test_integration_xdist_full_output() {
        let output = r#"=== test session starts ===
platform linux -- Python 3.12.0, pytest-8.0.0, pluggy-1.4.0
plugins: xdist-3.5.0
8 workers [50 items]
scheduling tests via LoadScheduling

[gw0] PASSED tests/test_auth.py::test_login
[gw1] PASSED tests/test_auth.py::test_logout
[gw2] FAILED tests/test_api.py::test_create_user
[gw3] PASSED tests/test_api.py::test_list_users
[gw0] PASSED tests/test_db.py::test_connection
[gw1] PASSED tests/test_db.py::test_migration
[gw2] PASSED tests/test_db.py::test_rollback

=== FAILURES ===
_______________________________ test_create_user _______________________________

    def test_create_user():
>       assert response.status_code == 201
E       AssertionError: assert 400 == 201

tests/test_api.py:42: AssertionError

=== short test summary info ===
FAILED tests/test_api.py::test_create_user - AssertionError: assert 400 == 201
=== 49 passed, 1 failed in 3.50s ==="#;

        let result = filter_pytest_output(output, Some(1));
        assert!(
            result.contains("49 passed, 1 failed"),
            "Summary: {}",
            result
        );
        assert!(
            result.contains("test_create_user"),
            "Failure detail: {}",
            result
        );
        assert!(result.contains("400 == 201"), "Error detail: {}", result);
    }

    #[test]
    fn test_integration_quiet_mode() {
        // Quiet mode: no === delimiters on summary
        let output = r#".....                                                              [100%]
5 passed in 0.50s"#;

        let result = filter_pytest_output(output, Some(0));
        assert!(
            result.contains("5 passed"),
            "Should parse quiet-mode summary, got: {}",
            result
        );
    }

    #[test]
    fn test_integration_no_summary_flag() {
        // --no-summary: pytest produces no summary at all
        let output = r#"=== test session starts ===
collected 10 items

tests/test_foo.py ..........                                       [100%]"#;

        let result = filter_pytest_output(output, Some(0));
        assert!(
            result.contains("All tests passed"),
            "Should use exit code when no summary: {}",
            result
        );
    }

    #[test]
    fn test_integration_tb_long_large_tracebacks() {
        // --tb=long produces massive tracebacks
        let mut output =
            String::from("=== test session starts ===\ncollected 3 items\n\n=== FAILURES ===\n");

        for i in 0..10 {
            output.push_str(&format!(
                "_______________________________ test_{} _______________________________\n",
                i
            ));
            output.push_str("    def test_something():\n");
            output.push_str(">       assert False\n");
            output.push_str("E       AssertionError\n\n");
            // Simulate long traceback
            for j in 0..50 {
                output.push_str(&format!("        frame {} at line {}\n", j, j * 10));
            }
            output.push_str(&format!(
                "tests/test_foo.py:{}: AssertionError\n\n",
                i * 10 + 5
            ));
        }

        output.push_str("=== short test summary info ===\n");
        for i in 0..10 {
            output.push_str(&format!(
                "FAILED tests/test_foo.py::test_{} - AssertionError\n",
                i
            ));
        }
        output.push_str("=== 0 passed, 10 failed in 5.00s ===\n");

        let result = filter_pytest_output(&output, Some(1));
        assert!(
            result.contains("10 failed"),
            "Should show failure count: {}",
            result
        );
        // Should limit displayed failures (shows 5, truncates rest)
        assert!(
            result.contains("more failures"),
            "Should truncate excess failures: {}",
            result
        );
    }

    #[test]
    fn test_integration_collection_error_full() {
        let output = r#"=== test session starts ===
platform linux -- Python 3.12.0

=== ERRORS ===
______________________ ERROR collecting tests/test_broken.py ______________________
ImportError while importing test module '/path/to/tests/test_broken.py'.
Hint: make sure your test modules/packages have valid Python names.
E   ModuleNotFoundError: No module named 'nonexistent_dependency'

=== short test summary info ===
ERROR tests/test_broken.py - ModuleNotFoundError: No module named 'nonexistent_dependency'
!!!!!!!!!!!!!!!!!!!!!! Interrupted: 1 error during collection !!!!!!!!!!!!!!!!!!!!!!"#;

        let result = filter_pytest_output(output, Some(2));
        assert!(
            !result.contains("No tests collected"),
            "Collection error != no tests collected: {}",
            result
        );
        assert!(
            result.contains("Interrupted") || result.contains("error"),
            "Should indicate interruption/error: {}",
            result
        );
    }

    #[test]
    fn test_integration_real_world_summary_format() {
        // Real-world pytest summary with many = signs
        let output = "========================== test session starts ==========================\n\
                       platform darwin -- Python 3.11.7, pytest-8.0.2, pluggy-1.4.0\n\
                       collected 1466 items\n\n\
                       ====================== 43 failed, 1423 passed in 45.23s ======================\n";

        let result = filter_pytest_output(output, Some(1));
        assert!(result.contains("43 failed"), "Failed count: {}", result);
        assert!(result.contains("1423 passed"), "Passed count: {}", result);
    }
}
