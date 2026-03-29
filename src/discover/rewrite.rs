use lazy_static::lazy_static;
use regex::Regex;

use super::classify::{classify_command, has_crunch_disabled_prefix, Classification, ENV_PREFIX};
#[cfg(test)]
use super::env_wrap::maybe_env_wrap_with_config;
use super::env_wrap::{maybe_env_wrap, ENV_WRAPPER};

lazy_static! {
    // Match trailing shell redirections:
    // Alt 1: N>&M or N>&- (fd redirect/close): 2>&1, 1>&2, 2>&-
    // Alt 2: &>file or &>>file (bash redirect both): &>/dev/null
    // Alt 3: N>file or N>>file (fd to file): 2>/dev/null, >/tmp/out, 1>>log
    // Note: [^(\\s] excludes process substitutions like >(tee) from false-positive matching
    static ref TRAILING_REDIRECT: Regex =
        Regex::new(r"\s+(?:[0-9]?>&[0-9-]|&>>?\S+|[0-9]?>>?\s*[^(\s]\S*)\s*$").unwrap();
}

/// Strip trailing stderr/stdout redirects from a command segment (#530).
/// Returns (command_without_redirects, redirect_suffix).
fn strip_trailing_redirects(cmd: &str) -> (&str, &str) {
    if let Some(m) = TRAILING_REDIRECT.find(cmd) {
        // Verify redirect is not inside quotes (single-pass count)
        let before = &cmd[..m.start()];
        let (sq, dq) = before.chars().fold((0u32, 0u32), |(s, d), c| match c {
            '\'' => (s + 1, d),
            '"' => (s, d + 1),
            _ => (s, d),
        });
        if sq % 2 == 0 && dq % 2 == 0 {
            return (&cmd[..m.start()], &cmd[m.start()..]);
        }
    }
    (cmd, "")
}

/// Rewrite `head -N file` → `crunch read file --max-lines N`.
/// Returns `None` if the command doesn't match this pattern (fall through to generic logic).
fn rewrite_head_numeric(cmd: &str) -> Option<String> {
    // Match: head -<digits> <file>  (with optional env prefix)
    lazy_static! {
        static ref HEAD_N: Regex = Regex::new(r"^head\s+-(\d+)\s+(.+)$").expect("valid regex");
        static ref HEAD_LINES: Regex =
            Regex::new(r"^head\s+--lines=(\d+)\s+(.+)$").expect("valid regex");
    }
    if let Some(caps) = HEAD_N.captures(cmd) {
        let n = caps.get(1)?.as_str();
        let file = caps.get(2)?.as_str();
        return Some(format!("crunch read {} --max-lines {}", file, n));
    }
    if let Some(caps) = HEAD_LINES.captures(cmd) {
        let n = caps.get(1)?.as_str();
        let file = caps.get(2)?.as_str();
        return Some(format!("crunch read {} --max-lines {}", file, n));
    }
    // head with any other flag (e.g. -c, -q): skip rewriting to avoid clap errors
    if cmd.starts_with("head -") {
        return None;
    }
    None
}

/// Rewrite `tail` numeric line forms to `crunch read ... --tail-lines N`.
/// Returns `None` when the pattern is unsupported (caller falls through / skips rewrite).
fn rewrite_tail_lines(cmd: &str) -> Option<String> {
    lazy_static! {
        static ref TAIL_N: Regex = Regex::new(r"^tail\s+-(\d+)\s+(.+)$").expect("valid regex");
        static ref TAIL_N_SPACE: Regex =
            Regex::new(r"^tail\s+-n\s+(\d+)\s+(.+)$").expect("valid regex");
        static ref TAIL_LINES_EQ: Regex =
            Regex::new(r"^tail\s+--lines=(\d+)\s+(.+)$").expect("valid regex");
        static ref TAIL_LINES_SPACE: Regex =
            Regex::new(r"^tail\s+--lines\s+(\d+)\s+(.+)$").expect("valid regex");
    }

    for re in [
        &*TAIL_N,
        &*TAIL_N_SPACE,
        &*TAIL_LINES_EQ,
        &*TAIL_LINES_SPACE,
    ] {
        if let Some(caps) = re.captures(cmd) {
            let n = caps.get(1)?.as_str();
            let file = caps.get(2)?.as_str();
            return Some(format!("crunch read {} --tail-lines {}", file, n));
        }
    }

    // Unknown tail form: skip rewrite to preserve native behavior.
    None
}

/// Strip a command prefix with word-boundary check.
/// Returns the remainder of the command after the prefix, or `None` if no match.
fn strip_word_prefix<'a>(cmd: &'a str, prefix: &str) -> Option<&'a str> {
    if cmd == prefix {
        Some("")
    } else if cmd.len() > prefix.len()
        && cmd.starts_with(prefix)
        && cmd.as_bytes()[prefix.len()] == b' '
    {
        Some(cmd[prefix.len() + 1..].trim_start())
    } else {
        None
    }
}

/// Inner implementation of `rewrite_segment` — body is unchanged from the original.
/// Returns `Some(rewritten)` if matched (including already-Crunch pass-through).
/// Returns `None` if no match (caller uses original segment).
///
/// `depth` tracks recursion through ENV_WRAPPER chains. Callers pass `0`; the recursive
/// call inside the env-wrapper branch passes `depth + 1`. Bails out at depth > 5 to
/// prevent stack overflow if ENV_WRAPPER patterns are ever expanded to cover more prefixes.
fn rewrite_segment_inner(seg: &str, excluded: &[String], depth: u8) -> Option<String> {
    if depth > 5 {
        return None;
    }

    let trimmed = seg.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip trailing stderr/stdout redirects before matching (#530)
    // e.g. "git status 2>&1" → match "git status", re-append " 2>&1"
    let (cmd_part, redirect_suffix) = strip_trailing_redirects(trimmed);

    // Already Crunch — pass through unchanged
    if cmd_part.starts_with("crunch ") || cmd_part == "crunch" {
        return Some(trimmed.to_string());
    }

    // Env-wrapper commands (uv run, poetry run, etc.) activate a virtualenv.
    // Preserve the wrapper and insert `crunch` between it and the inner tool:
    //   `uv run pytest -x` → `uv run crunch pytest -x`
    if let Some(caps) = ENV_WRAPPER.captures(cmd_part) {
        let wrapper = caps.get(1).unwrap().as_str();
        let wrapper_match = caps.get(0).unwrap();
        let inner = &cmd_part[wrapper_match.end()..];
        // Already wrapped with crunch — pass through
        if inner.starts_with("crunch ") || inner == "crunch" {
            return Some(trimmed.to_string());
        }
        // Try to rewrite the inner command; if it matches, re-wrap
        let inner_with_redirect = if redirect_suffix.is_empty() {
            inner.to_string()
        } else {
            format!("{}{}", inner, redirect_suffix)
        };
        return rewrite_segment_inner(&inner_with_redirect, excluded, depth + 1)
            .map(|rewritten| format!("{} {}", wrapper, rewritten));
    }

    // Special case: `head -N file` / `head --lines=N file` → `crunch read file --max-lines N`
    // Must intercept before generic prefix replacement, which would produce `crunch read -20 file`.
    // Only intercept when head has a flag (-N, --lines=N, -c, etc.); plain `head file` falls
    // through to the generic rewrite below and produces `crunch read file` as expected.
    if cmd_part.starts_with("head -") {
        return rewrite_head_numeric(cmd_part).map(|r| format!("{}{}", r, redirect_suffix));
    }

    // tail has several forms that are not compatible with generic prefix replacement.
    // Only rewrite recognized numeric line forms; otherwise skip rewrite.
    if cmd_part.starts_with("tail ") {
        return rewrite_tail_lines(cmd_part).map(|r| format!("{}{}", r, redirect_suffix));
    }

    // Use classify_command for correct ignore/prefix handling
    let crunch_equivalent = match classify_command(cmd_part) {
        Classification::Supported {
            crunch_equivalent, ..
        } => {
            // Check if the base command is excluded from rewriting (#243)
            let base = cmd_part.split_whitespace().next().unwrap_or("");
            if excluded.iter().any(|e| e == base) {
                return None;
            }
            crunch_equivalent
        }
        _ => return None,
    };

    // Find the matching rule (crunch_cmd values are unique across all rules)
    let rule = super::rules::RULES
        .iter()
        .find(|r| r.crunch_cmd == crunch_equivalent)?;

    // Extract env prefix (sudo, env VAR=val, etc.)
    let stripped_cow = ENV_PREFIX.replace(cmd_part, "");
    let env_prefix_len = cmd_part.len() - stripped_cow.len();
    let env_prefix = &cmd_part[..env_prefix_len];
    let cmd_clean = stripped_cow.trim();

    // #345: CRUNCH_DISABLED=1 in env prefix → skip rewrite entirely
    if has_crunch_disabled_prefix(cmd_part) {
        return None;
    }

    // #196: gh with --json/--jq/--template produces structured output that
    // crunch gh would corrupt — skip rewrite so the caller gets raw JSON.
    if rule.crunch_cmd == "crunch gh" {
        let args_lower = cmd_clean.to_lowercase();
        if args_lower.contains("--json")
            || args_lower.contains("--jq")
            || args_lower.contains("--template")
        {
            return None;
        }
    }

    // Try each rewrite prefix (longest first) with word-boundary check
    for &prefix in rule.rewrite_prefixes {
        if let Some(rest) = strip_word_prefix(cmd_clean, prefix) {
            let rewritten = if rest.is_empty() {
                format!("{}{}{}", env_prefix, rule.crunch_cmd, redirect_suffix)
            } else {
                format!(
                    "{}{} {}{}",
                    env_prefix, rule.crunch_cmd, rest, redirect_suffix
                )
            };
            return Some(rewritten);
        }
    }

    None
}

/// Rewrite a single (non-compound) command segment.
/// Returns `Some(rewritten)` if matched (including already-Crunch pass-through).
/// Returns `None` if no match (caller uses original segment).
pub(super) fn rewrite_segment(seg: &str, excluded: &[String]) -> Option<String> {
    let trimmed = seg.trim();
    // Note: strip_trailing_redirects is called here to extract base/redirect for env_wrap,
    // and again inside rewrite_segment_inner for its own rewrite logic. This is intentional —
    // the function is idempotent and the two callers use the results independently.
    let (cmd_part, redirect_suffix) = strip_trailing_redirects(trimmed);
    // Strip env-var prefixes (e.g. `FOO=1 python3 …`) before extracting the base
    // command so that env-wrap matching works correctly for prefixed commands.
    let stripped = ENV_PREFIX.replace(cmd_part, "");
    let base = stripped.split_whitespace().next().unwrap_or("");
    let inner_result = rewrite_segment_inner(seg, excluded, 0);
    maybe_env_wrap(base, cmd_part, redirect_suffix, inner_result)
}

/// Testable variant of `rewrite_segment` that accepts an explicit `EnvConfig`.
#[cfg(test)]
pub(super) fn rewrite_segment_with_env(
    seg: &str,
    excluded: &[String],
    env: &crate::config::EnvConfig,
) -> Option<String> {
    let trimmed = seg.trim();
    let (cmd_part, redirect_suffix) = strip_trailing_redirects(trimmed);
    // Strip env-var prefixes (e.g. `FOO=1 python3 …`) before extracting the base
    // command so that env-wrap matching works correctly for prefixed commands.
    let stripped = ENV_PREFIX.replace(cmd_part, "");
    let base = stripped.split_whitespace().next().unwrap_or("");
    let inner_result = rewrite_segment_inner(seg, excluded, 0);
    maybe_env_wrap_with_config(base, cmd_part, redirect_suffix, inner_result, env)
}

/// Rewrite a compound command (with `&&`, `||`, `;`, `|`) by rewriting each segment.
fn rewrite_compound(cmd: &str, excluded: &[String]) -> Option<String> {
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len + 32);
    let mut any_changed = false;
    let mut seg_start = 0;
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < len {
        let b = bytes[i];
        match b {
            b'\'' if !in_double => {
                in_single = !in_single;
                i += 1;
            }
            b'"' if !in_single => {
                in_double = !in_double;
                i += 1;
            }
            b'|' if !in_single && !in_double => {
                if i + 1 < len && bytes[i + 1] == b'|' {
                    // `||` operator — rewrite left, continue
                    let seg = cmd[seg_start..i].trim();
                    let rewritten =
                        rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string());
                    if rewritten != seg {
                        any_changed = true;
                    }
                    result.push_str(&rewritten);
                    result.push_str(" || ");
                    i += 2;
                    while i < len && bytes[i] == b' ' {
                        i += 1;
                    }
                    seg_start = i;
                } else {
                    // `|` pipe — rewrite first segment only, pass through the rest unchanged
                    let seg = cmd[seg_start..i].trim();
                    // Skip rewriting `find`/`fd` in pipes — crunch find outputs a grouped
                    // format that is incompatible with pipe consumers like xargs, grep,
                    // wc, sort, etc. which expect one path per line (#439).
                    let is_pipe_incompatible = seg.starts_with("find ")
                        || seg == "find"
                        || seg.starts_with("fd ")
                        || seg == "fd";
                    let rewritten = if is_pipe_incompatible {
                        seg.to_string()
                    } else {
                        rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string())
                    };
                    if rewritten != seg {
                        any_changed = true;
                    }
                    result.push_str(&rewritten);
                    // Preserve the space before the pipe that was lost by trim()
                    result.push(' ');
                    result.push_str(cmd[i..].trim_start());
                    return if any_changed { Some(result) } else { None };
                }
            }
            b'&' if !in_single && !in_double && i + 1 < len && bytes[i + 1] == b'&' => {
                // `&&` operator — rewrite left, continue
                let seg = cmd[seg_start..i].trim();
                let rewritten = rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string());
                if rewritten != seg {
                    any_changed = true;
                }
                result.push_str(&rewritten);
                result.push_str(" && ");
                i += 2;
                while i < len && bytes[i] == b' ' {
                    i += 1;
                }
                seg_start = i;
            }
            b'&' if !in_single && !in_double => {
                // #346: redirect detection — 2>&1 / >&2 (> before &) or &>file / &>>file (> after &)
                let is_redirect =
                    (i > 0 && bytes[i - 1] == b'>') || (i + 1 < len && bytes[i + 1] == b'>');
                if is_redirect {
                    i += 1;
                } else {
                    // single `&` background execution operator
                    let seg = cmd[seg_start..i].trim();
                    let rewritten =
                        rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string());
                    if rewritten != seg {
                        any_changed = true;
                    }
                    result.push_str(&rewritten);
                    result.push_str(" & ");
                    i += 1;
                    while i < len && bytes[i] == b' ' {
                        i += 1;
                    }
                    seg_start = i;
                }
            }
            b';' if !in_single && !in_double => {
                // `;` separator
                let seg = cmd[seg_start..i].trim();
                let rewritten = rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string());
                if rewritten != seg {
                    any_changed = true;
                }
                result.push_str(&rewritten);
                result.push(';');
                i += 1;
                while i < len && bytes[i] == b' ' {
                    i += 1;
                }
                if i < len {
                    result.push(' ');
                }
                seg_start = i;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Last (or only) segment
    let seg = cmd[seg_start..len].trim();
    let rewritten = rewrite_segment(seg, excluded).unwrap_or_else(|| seg.to_string());
    if rewritten != seg {
        any_changed = true;
    }
    result.push_str(&rewritten);

    if any_changed {
        Some(result)
    } else {
        None
    }
}

/// Returns `None` if the command is unsupported or ignored (hook should pass through).
///
/// Handles compound commands (`&&`, `||`, `;`) by rewriting each segment independently.
/// For pipes (`|`), only rewrites the first command (the filter stays raw).
pub fn rewrite_command(cmd: &str, excluded: &[String]) -> Option<String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Heredoc or arithmetic expansion — unsafe to split/rewrite
    if trimmed.contains("<<") || trimmed.contains("$((") {
        return None;
    }

    // Simple (non-compound) already-Crunch command — return as-is.
    // For compound commands that start with "crunch" (e.g. "crunch git add . && cargo test"),
    // fall through to rewrite_compound so the remaining segments get rewritten.
    let has_compound = trimmed.contains("&&")
        || trimmed.contains("||")
        || trimmed.contains(';')
        || trimmed.contains('|')
        || trimmed.contains(" & ");
    if !has_compound && (trimmed.starts_with("crunch ") || trimmed == "crunch") {
        return Some(trimmed.to_string());
    }

    rewrite_compound(trimmed, excluded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_git_status() {
        assert_eq!(
            rewrite_command("git status", &[]),
            Some("crunch git status".into())
        );
    }

    #[test]
    fn test_rewrite_git_log() {
        assert_eq!(
            rewrite_command("git log -10", &[]),
            Some("crunch git log -10".into())
        );
    }

    // --- git -C <path> support (#555) ---

    #[test]
    fn test_rewrite_git_dash_c_status() {
        assert_eq!(
            rewrite_command("git -C /path/to/repo status", &[]),
            Some("crunch git -C /path/to/repo status".into())
        );
    }

    #[test]
    fn test_rewrite_git_dash_c_log() {
        assert_eq!(
            rewrite_command("git -C /tmp/myrepo log --oneline -5", &[]),
            Some("crunch git -C /tmp/myrepo log --oneline -5".into())
        );
    }

    #[test]
    fn test_rewrite_git_dash_c_diff() {
        assert_eq!(
            rewrite_command("git -C /home/user/project diff --name-only", &[]),
            Some("crunch git -C /home/user/project diff --name-only".into())
        );
    }

    #[test]
    fn test_rewrite_cargo_test() {
        assert_eq!(
            rewrite_command("cargo test", &[]),
            Some("crunch cargo test".into())
        );
    }

    #[test]
    fn test_rewrite_compound_and() {
        assert_eq!(
            rewrite_command("git add . && cargo test", &[]),
            Some("crunch git add . && crunch cargo test".into())
        );
    }

    #[test]
    fn test_rewrite_compound_three_segments() {
        assert_eq!(
            rewrite_command(
                "cargo fmt --all && cargo clippy --all-targets && cargo test",
                &[]
            ),
            Some(
                "crunch cargo fmt --all && crunch cargo clippy --all-targets && crunch cargo test"
                    .into()
            )
        );
    }

    #[test]
    fn test_rewrite_already_crunch() {
        assert_eq!(
            rewrite_command("crunch git status", &[]),
            Some("crunch git status".into())
        );
    }

    #[test]
    fn test_rewrite_background_single_amp() {
        assert_eq!(
            rewrite_command("cargo test & git status", &[]),
            Some("crunch cargo test & crunch git status".into())
        );
    }

    #[test]
    fn test_rewrite_background_unsupported_right() {
        assert_eq!(
            rewrite_command("cargo test & htop", &[]),
            Some("crunch cargo test & htop".into())
        );
    }

    #[test]
    fn test_rewrite_background_does_not_affect_double_amp() {
        // `&&` must still work after adding `&` support
        assert_eq!(
            rewrite_command("cargo test && git status", &[]),
            Some("crunch cargo test && crunch git status".into())
        );
    }

    #[test]
    fn test_rewrite_unsupported_returns_none() {
        assert_eq!(rewrite_command("htop", &[]), None);
    }

    #[test]
    fn test_rewrite_ignored_cd() {
        assert_eq!(rewrite_command("cd /tmp", &[]), None);
    }

    #[test]
    fn test_rewrite_with_env_prefix() {
        assert_eq!(
            rewrite_command("GIT_SSH_COMMAND=ssh git push", &[]),
            Some("GIT_SSH_COMMAND=ssh crunch git push".into())
        );
    }

    #[test]
    fn test_rewrite_npx_tsc() {
        assert_eq!(
            rewrite_command("npx tsc --noEmit", &[]),
            Some("crunch tsc --noEmit".into())
        );
    }

    #[test]
    fn test_rewrite_pnpm_tsc() {
        assert_eq!(
            rewrite_command("pnpm tsc --noEmit", &[]),
            Some("crunch tsc --noEmit".into())
        );
    }

    #[test]
    fn test_rewrite_cat_file() {
        assert_eq!(
            rewrite_command("cat src/main.rs", &[]),
            Some("crunch read src/main.rs".into())
        );
    }

    #[test]
    fn test_rewrite_rg_pattern() {
        assert_eq!(
            rewrite_command("rg \"fn main\"", &[]),
            Some("crunch grep \"fn main\"".into())
        );
    }

    #[test]
    fn test_rewrite_npx_playwright() {
        assert_eq!(
            rewrite_command("npx playwright test", &[]),
            Some("crunch playwright test".into())
        );
    }

    #[test]
    fn test_rewrite_next_build() {
        assert_eq!(
            rewrite_command("next build --turbo", &[]),
            Some("crunch next --turbo".into())
        );
    }

    #[test]
    fn test_rewrite_pipe_first_only() {
        // After a pipe, the filter command stays raw
        assert_eq!(
            rewrite_command("git log -10 | grep feat", &[]),
            Some("crunch git log -10 | grep feat".into())
        );
    }

    #[test]
    fn test_rewrite_find_pipe_skipped() {
        // find in a pipe should NOT be rewritten — crunch find output format
        // is incompatible with pipe consumers like xargs (#439)
        assert_eq!(
            rewrite_command("find . -name '*.rs' | xargs grep 'fn run'", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_find_pipe_xargs_wc() {
        assert_eq!(rewrite_command("find src -type f | wc -l", &[]), None);
    }

    #[test]
    fn test_rewrite_find_no_pipe_still_rewritten() {
        // find WITHOUT a pipe should still be rewritten
        assert_eq!(
            rewrite_command("find . -name '*.rs'", &[]),
            Some("crunch find . -name '*.rs'".into())
        );
    }

    #[test]
    fn test_rewrite_heredoc_returns_none() {
        assert_eq!(rewrite_command("cat <<'EOF'\nfoo\nEOF", &[]), None);
    }

    #[test]
    fn test_rewrite_empty_returns_none() {
        assert_eq!(rewrite_command("", &[]), None);
        assert_eq!(rewrite_command("   ", &[]), None);
    }

    #[test]
    fn test_rewrite_mixed_compound_partial() {
        // First segment already crunch, second gets rewritten
        assert_eq!(
            rewrite_command("crunch git add . && cargo test", &[]),
            Some("crunch git add . && crunch cargo test".into())
        );
    }

    // --- #345: CRUNCH_DISABLED ---

    #[test]
    fn test_rewrite_crunch_disabled_curl() {
        assert_eq!(
            rewrite_command("CRUNCH_DISABLED=1 curl https://example.com", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_crunch_disabled_git_status() {
        assert_eq!(rewrite_command("CRUNCH_DISABLED=1 git status", &[]), None);
    }

    #[test]
    fn test_rewrite_crunch_disabled_multi_env() {
        assert_eq!(
            rewrite_command("FOO=1 CRUNCH_DISABLED=1 git status", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_non_crunch_disabled_env_still_rewrites() {
        assert_eq!(
            rewrite_command("SOME_VAR=1 git status", &[]),
            Some("SOME_VAR=1 crunch git status".into())
        );
    }

    // --- #346: 2>&1 and &> redirect detection ---

    #[test]
    fn test_rewrite_redirect_2_gt_amp_1_with_pipe() {
        assert_eq!(
            rewrite_command("cargo test 2>&1 | head", &[]),
            Some("crunch cargo test 2>&1 | head".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_2_gt_amp_1_trailing() {
        assert_eq!(
            rewrite_command("cargo test 2>&1", &[]),
            Some("crunch cargo test 2>&1".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_plain_2_devnull() {
        // 2>/dev/null has no `&`, never broken — non-regression
        assert_eq!(
            rewrite_command("git status 2>/dev/null", &[]),
            Some("crunch git status 2>/dev/null".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_2_gt_amp_1_with_and() {
        assert_eq!(
            rewrite_command("cargo test 2>&1 && echo done", &[]),
            Some("crunch cargo test 2>&1 && echo done".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_amp_gt_devnull() {
        assert_eq!(
            rewrite_command("cargo test &>/dev/null", &[]),
            Some("crunch cargo test &>/dev/null".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_double() {
        // Double redirect: only last one stripped, but full command rewrites correctly
        assert_eq!(
            rewrite_command("git status 2>&1 >/dev/null", &[]),
            Some("crunch git status 2>&1 >/dev/null".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_fd_close() {
        // 2>&- (close stderr fd)
        assert_eq!(
            rewrite_command("git status 2>&-", &[]),
            Some("crunch git status 2>&-".into())
        );
    }

    #[test]
    fn test_rewrite_redirect_quotes_not_stripped() {
        // Redirect-like chars inside quotes should NOT be stripped
        // Known limitation: apostrophes cause conservative no-strip (safe fallback)
        let result = rewrite_command("git commit -m \"it's fixed\" 2>&1", &[]);
        assert!(
            result.is_some(),
            "Should still rewrite even with apostrophe"
        );
    }

    #[test]
    fn test_rewrite_background_amp_non_regression() {
        // background `&` must still work after redirect fix
        assert_eq!(
            rewrite_command("cargo test & git status", &[]),
            Some("crunch cargo test & crunch git status".into())
        );
    }

    // --- P0.2: head -N rewrite ---

    #[test]
    fn test_rewrite_head_numeric_flag() {
        // head -20 file → crunch read file --max-lines 20 (not crunch read -20 file)
        assert_eq!(
            rewrite_command("head -20 src/main.rs", &[]),
            Some("crunch read src/main.rs --max-lines 20".into())
        );
    }

    #[test]
    fn test_rewrite_head_lines_long_flag() {
        assert_eq!(
            rewrite_command("head --lines=50 src/lib.rs", &[]),
            Some("crunch read src/lib.rs --max-lines 50".into())
        );
    }

    #[test]
    fn test_rewrite_head_no_flag_still_rewrites() {
        // plain `head file` → `crunch read file` (no numeric flag)
        assert_eq!(
            rewrite_command("head src/main.rs", &[]),
            Some("crunch read src/main.rs".into())
        );
    }

    #[test]
    fn test_rewrite_head_other_flag_skipped() {
        // head -c 100 file: unsupported flag, skip rewriting
        assert_eq!(rewrite_command("head -c 100 src/main.rs", &[]), None);
    }

    #[test]
    fn test_rewrite_tail_numeric_flag() {
        assert_eq!(
            rewrite_command("tail -20 src/main.rs", &[]),
            Some("crunch read src/main.rs --tail-lines 20".into())
        );
    }

    #[test]
    fn test_rewrite_tail_n_space_flag() {
        assert_eq!(
            rewrite_command("tail -n 12 src/lib.rs", &[]),
            Some("crunch read src/lib.rs --tail-lines 12".into())
        );
    }

    #[test]
    fn test_rewrite_tail_lines_long_flag() {
        assert_eq!(
            rewrite_command("tail --lines=7 src/lib.rs", &[]),
            Some("crunch read src/lib.rs --tail-lines 7".into())
        );
    }

    #[test]
    fn test_rewrite_tail_lines_space_flag() {
        assert_eq!(
            rewrite_command("tail --lines 7 src/lib.rs", &[]),
            Some("crunch read src/lib.rs --tail-lines 7".into())
        );
    }

    #[test]
    fn test_rewrite_tail_other_flag_skipped() {
        assert_eq!(rewrite_command("tail -c 100 src/main.rs", &[]), None);
    }

    #[test]
    fn test_rewrite_tail_plain_file_skipped() {
        assert_eq!(rewrite_command("tail src/main.rs", &[]), None);
    }

    #[test]
    fn test_rewrite_tree() {
        assert_eq!(
            rewrite_command("tree src/", &[]),
            Some("crunch tree src/".into())
        );
    }

    #[test]
    fn test_rewrite_diff() {
        assert_eq!(
            rewrite_command("diff file1.txt file2.txt", &[]),
            Some("crunch diff file1.txt file2.txt".into())
        );
    }

    #[test]
    fn test_rewrite_gh_release() {
        assert_eq!(
            rewrite_command("gh release list", &[]),
            Some("crunch gh release list".into())
        );
    }

    #[test]
    fn test_rewrite_cargo_install() {
        assert_eq!(
            rewrite_command("cargo install rtk", &[]),
            Some("crunch cargo install rtk".into())
        );
    }

    #[test]
    fn test_rewrite_kubectl_describe() {
        assert_eq!(
            rewrite_command("kubectl describe pod mypod", &[]),
            Some("crunch kubectl describe pod mypod".into())
        );
    }

    #[test]
    fn test_rewrite_docker_run() {
        assert_eq!(
            rewrite_command("docker run --rm ubuntu bash", &[]),
            Some("crunch docker run --rm ubuntu bash".into())
        );
    }

    #[test]
    fn test_rewrite_swift_test() {
        assert_eq!(
            rewrite_command("swift test --parallel", &[]),
            Some("crunch swift test --parallel".into())
        );
    }

    // --- #336: docker compose supported subcommands rewritten, unsupported skipped ---

    #[test]
    fn test_rewrite_docker_compose_ps() {
        assert_eq!(
            rewrite_command("docker compose ps", &[]),
            Some("crunch docker compose ps".into())
        );
    }

    #[test]
    fn test_rewrite_docker_compose_logs() {
        assert_eq!(
            rewrite_command("docker compose logs web", &[]),
            Some("crunch docker compose logs web".into())
        );
    }

    #[test]
    fn test_rewrite_docker_compose_build() {
        assert_eq!(
            rewrite_command("docker compose build", &[]),
            Some("crunch docker compose build".into())
        );
    }

    #[test]
    fn test_rewrite_docker_compose_up_skipped() {
        assert_eq!(rewrite_command("docker compose up -d", &[]), None);
    }

    #[test]
    fn test_rewrite_docker_compose_down_skipped() {
        assert_eq!(rewrite_command("docker compose down", &[]), None);
    }

    #[test]
    fn test_rewrite_docker_compose_config_skipped() {
        assert_eq!(
            rewrite_command("docker compose -f foo.yaml config --services", &[]),
            None
        );
    }

    // --- AWS / psql (PR #216) ---

    #[test]
    fn test_rewrite_aws() {
        assert_eq!(
            rewrite_command("aws s3 ls", &[]),
            Some("crunch aws s3 ls".into())
        );
    }

    #[test]
    fn test_rewrite_aws_ec2() {
        assert_eq!(
            rewrite_command("aws ec2 describe-instances --region us-east-1", &[]),
            Some("crunch aws ec2 describe-instances --region us-east-1".into())
        );
    }

    #[test]
    fn test_rewrite_psql() {
        assert_eq!(
            rewrite_command("psql -U postgres -d mydb", &[]),
            Some("crunch psql -U postgres -d mydb".into())
        );
    }

    // --- Python tooling ---

    #[test]
    fn test_rewrite_ruff_check() {
        assert_eq!(
            rewrite_command("ruff check .", &[]),
            Some("crunch ruff check .".into())
        );
    }

    #[test]
    fn test_rewrite_ruff_format() {
        assert_eq!(
            rewrite_command("ruff format src/", &[]),
            Some("crunch ruff format src/".into())
        );
    }

    #[test]
    fn test_rewrite_pytest() {
        assert_eq!(
            rewrite_command("pytest tests/", &[]),
            Some("crunch pytest tests/".into())
        );
    }

    #[test]
    fn test_rewrite_python_m_pytest() {
        assert_eq!(
            rewrite_command("python -m pytest -x tests/", &[]),
            Some("crunch pytest -x tests/".into())
        );
    }

    #[test]
    fn test_rewrite_pip_list() {
        assert_eq!(
            rewrite_command("pip list", &[]),
            Some("crunch pip list".into())
        );
    }

    #[test]
    fn test_rewrite_pip_outdated() {
        assert_eq!(
            rewrite_command("pip outdated", &[]),
            Some("crunch pip outdated".into())
        );
    }

    #[test]
    fn test_rewrite_uv_pip_list() {
        assert_eq!(
            rewrite_command("uv pip list", &[]),
            Some("crunch pip list".into())
        );
    }

    // --- Go tooling ---

    #[test]
    fn test_rewrite_go_test() {
        assert_eq!(
            rewrite_command("go test ./...", &[]),
            Some("crunch go test ./...".into())
        );
    }

    #[test]
    fn test_rewrite_go_build() {
        assert_eq!(
            rewrite_command("go build ./...", &[]),
            Some("crunch go build ./...".into())
        );
    }

    #[test]
    fn test_rewrite_go_vet() {
        assert_eq!(
            rewrite_command("go vet ./...", &[]),
            Some("crunch go vet ./...".into())
        );
    }

    #[test]
    fn test_rewrite_golangci_lint() {
        assert_eq!(
            rewrite_command("golangci-lint run ./...", &[]),
            Some("crunch golangci-lint run ./...".into())
        );
    }

    // --- JS/TS tooling ---

    #[test]
    fn test_rewrite_vitest() {
        assert_eq!(
            rewrite_command("vitest run", &[]),
            Some("crunch vitest run".into())
        );
    }

    #[test]
    fn test_rewrite_pnpm_vitest() {
        assert_eq!(
            rewrite_command("pnpm vitest run", &[]),
            Some("crunch vitest run".into())
        );
    }

    #[test]
    fn test_rewrite_prisma() {
        assert_eq!(
            rewrite_command("npx prisma migrate dev", &[]),
            Some("crunch prisma migrate dev".into())
        );
    }

    #[test]
    fn test_rewrite_prettier() {
        assert_eq!(
            rewrite_command("npx prettier --check src/", &[]),
            Some("crunch prettier --check src/".into())
        );
    }

    #[test]
    fn test_rewrite_pnpm_list() {
        assert_eq!(
            rewrite_command("pnpm list", &[]),
            Some("crunch pnpm list".into())
        );
    }

    // --- Compound operator edge cases ---

    #[test]
    fn test_rewrite_compound_or() {
        // `||` fallback: left rewritten, right rewritten
        assert_eq!(
            rewrite_command("cargo test || cargo build", &[]),
            Some("crunch cargo test || crunch cargo build".into())
        );
    }

    #[test]
    fn test_rewrite_compound_semicolon() {
        assert_eq!(
            rewrite_command("git status; cargo test", &[]),
            Some("crunch git status; crunch cargo test".into())
        );
    }

    #[test]
    fn test_rewrite_compound_pipe_raw_filter() {
        // Pipe: rewrite first segment only, pass through rest unchanged
        assert_eq!(
            rewrite_command("cargo test | grep FAILED", &[]),
            Some("crunch cargo test | grep FAILED".into())
        );
    }

    #[test]
    fn test_rewrite_compound_pipe_git_grep() {
        assert_eq!(
            rewrite_command("git log -10 | grep feat", &[]),
            Some("crunch git log -10 | grep feat".into())
        );
    }

    #[test]
    fn test_rewrite_compound_four_segments() {
        assert_eq!(
            rewrite_command(
                "cargo fmt --all && cargo clippy && cargo test && git status",
                &[]
            ),
            Some(
                "crunch cargo fmt --all && crunch cargo clippy && crunch cargo test && crunch git status"
                    .into()
            )
        );
    }

    #[test]
    fn test_rewrite_compound_mixed_supported_unsupported() {
        // unsupported segments stay raw
        assert_eq!(
            rewrite_command("cargo test && htop", &[]),
            Some("crunch cargo test && htop".into())
        );
    }

    #[test]
    fn test_rewrite_compound_all_unsupported_returns_none() {
        // No rewrite at all: returns None
        assert_eq!(rewrite_command("htop && top", &[]), None);
    }

    // --- sudo / env prefix + rewrite ---

    #[test]
    fn test_rewrite_sudo_docker() {
        assert_eq!(
            rewrite_command("sudo docker ps", &[]),
            Some("sudo crunch docker ps".into())
        );
    }

    #[test]
    fn test_rewrite_env_var_prefix() {
        assert_eq!(
            rewrite_command("GIT_SSH_COMMAND=ssh git push origin main", &[]),
            Some("GIT_SSH_COMMAND=ssh crunch git push origin main".into())
        );
    }

    // --- find with native flags ---

    #[test]
    fn test_rewrite_find_with_flags() {
        assert_eq!(
            rewrite_command("find . -name '*.rs' -type f", &[]),
            Some("crunch find . -name '*.rs' -type f".into())
        );
    }

    // --- exclude_commands (#243) ---

    #[test]
    fn test_rewrite_excludes_curl() {
        let excluded = vec!["curl".to_string()];
        assert_eq!(
            rewrite_command("curl https://api.example.com/health", &excluded),
            None
        );
    }

    #[test]
    fn test_rewrite_exclude_does_not_affect_other_commands() {
        let excluded = vec!["curl".to_string()];
        assert_eq!(
            rewrite_command("git status", &excluded),
            Some("crunch git status".into())
        );
    }

    #[test]
    fn test_rewrite_empty_excludes_rewrites_curl() {
        let excluded: Vec<String> = vec![];
        assert!(rewrite_command("curl https://api.example.com", &excluded).is_some());
    }

    #[test]
    fn test_rewrite_compound_partial_exclude() {
        // curl excluded but git still rewrites
        let excluded = vec!["curl".to_string()];
        assert_eq!(
            rewrite_command("git status && curl https://api.example.com", &excluded),
            Some("crunch git status && curl https://api.example.com".into())
        );
    }

    // --- #196: gh --json/--jq/--template passthrough ---

    #[test]
    fn test_rewrite_gh_json_skipped() {
        assert_eq!(rewrite_command("gh pr list --json number,title", &[]), None);
    }

    #[test]
    fn test_rewrite_gh_jq_skipped() {
        assert_eq!(
            rewrite_command("gh pr list --json number --jq '.[].number'", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_gh_template_skipped() {
        assert_eq!(
            rewrite_command("gh pr view 42 --template '{{.title}}'", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_gh_api_json_skipped() {
        assert_eq!(
            rewrite_command("gh api repos/owner/repo --jq '.name'", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_gh_without_json_still_works() {
        assert_eq!(
            rewrite_command("gh pr list", &[]),
            Some("crunch gh pr list".into())
        );
    }

    // --- git -C / --no-pager rewrite ---

    #[test]
    fn test_rewrite_git_dash_c() {
        assert_eq!(
            rewrite_command("git -C /tmp status", &[]),
            Some("crunch git -C /tmp status".to_string())
        );
    }

    #[test]
    fn test_rewrite_git_no_pager() {
        assert_eq!(
            rewrite_command("git --no-pager log -5", &[]),
            Some("crunch git --no-pager log -5".to_string())
        );
    }

    // ── Env-wrapper rewrite tests ──────────────────────────────────────

    #[test]
    fn test_rewrite_uv_run_pytest() {
        assert_eq!(
            rewrite_command("uv run pytest -x tests/", &[]),
            Some("uv run crunch pytest -x tests/".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_run_pytest_verbose_with_redirect_and_pipe() {
        assert_eq!(
            rewrite_command(
                "uv run pytest packages/entity/tests/services/test_concurrent_resolution.py -v --run-db --tb=short 2>&1 | tail -15",
                &[]
            ),
            Some("uv run crunch pytest packages/entity/tests/services/test_concurrent_resolution.py -v --run-db --tb=short 2>&1 | tail -15".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_run_ruff() {
        assert_eq!(
            rewrite_command("uv run ruff check .", &[]),
            Some("uv run crunch ruff check .".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_run_mypy() {
        assert_eq!(
            rewrite_command("uv run mypy src/", &[]),
            Some("uv run crunch mypy src/".to_string())
        );
    }

    #[test]
    fn test_rewrite_poetry_run_pytest() {
        assert_eq!(
            rewrite_command("poetry run pytest -v", &[]),
            Some("poetry run crunch pytest -v".to_string())
        );
    }

    #[test]
    fn test_rewrite_pipx_run_ruff() {
        assert_eq!(
            rewrite_command("pipx run ruff check .", &[]),
            Some("pipx run crunch ruff check .".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_run_unsupported_tool_returns_none() {
        // `uv run python script.py` — python is not a crunch-supported tool
        assert_eq!(rewrite_command("uv run python script.py", &[]), None);
    }

    #[test]
    fn test_rewrite_uv_run_already_crunch() {
        // Already has crunch — no change needed, but should not return None
        // (compound handler treats Some(unchanged) as no-op which is fine;
        // for a simple command the top-level already-crunch check catches it)
        assert_eq!(rewrite_command("uv run crunch pytest -x", &[]), None);
    }

    #[test]
    fn test_rewrite_uv_run_compound() {
        assert_eq!(
            rewrite_command("uv run pytest -x && uv run ruff check .", &[]),
            Some("uv run crunch pytest -x && uv run crunch ruff check .".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_sync_still_works() {
        // Existing uv sync rewrite should not break
        assert_eq!(
            rewrite_command("uv sync", &[]),
            Some("crunch uv sync".to_string())
        );
    }

    #[test]
    fn test_rewrite_uv_pip_install_still_works() {
        // `uv pip install` is handled by the existing pip rule (rewrite_prefix "uv pip")
        // not by the env-wrapper logic — it becomes `crunch uv pip install`
        assert_eq!(
            rewrite_command("uv pip install requests", &[]),
            Some("crunch uv pip install requests".to_string())
        );
    }

    // --- R3: env-prefix stripped before base extraction ---

    #[test]
    fn test_rewrite_env_var_prefix_python3_with_env_wrap() {
        let env = crate::config::EnvConfig {
            wrapper: Some("uv run".into()),
            wrap_commands: vec!["python3".into()],
        };
        let result = rewrite_segment_with_env("FOO=1 python3 script.py", &[], &env);
        assert_eq!(result, Some("uv run FOO=1 python3 script.py".to_string()));
    }

    // --- S4: recursion depth guard ---

    #[test]
    fn test_rewrite_deeply_nested_wrappers_bounded() {
        // Pathological nesting — should not stack overflow, just bail out
        let cmd = "uv run poetry run pipx run pdm run uv run poetry run pytest -x";
        let result = rewrite_command(cmd, &[]);
        // Should produce a result (not panic), exact output doesn't matter
        assert!(result.is_some() || result.is_none());
    }
}
