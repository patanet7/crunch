use lazy_static::lazy_static;
use regex::{Regex, RegexSet};

use super::rules::{IGNORED_EXACT, IGNORED_PREFIXES, PATTERNS, RULES};

/// Result of classifying a command.
#[derive(Debug, PartialEq)]
pub enum Classification {
    Supported {
        crunch_equivalent: &'static str,
        category: &'static str,
        estimated_savings_pct: f64,
        status: super::report::CrunchStatus,
    },
    Unsupported {
        base_command: String,
    },
    Ignored,
}

/// Average token counts per category for estimation when no output_len available.
pub fn category_avg_tokens(category: &str, subcmd: &str) -> usize {
    match category {
        "Git" => match subcmd {
            "log" | "diff" | "show" => 200,
            _ => 40,
        },
        "Cargo" => match subcmd {
            "test" => 500,
            _ => 150,
        },
        "Tests" => 800,
        "Files" => 100,
        "Build" => 300,
        "Infra" => 120,
        "Network" => 150,
        "GitHub" => 200,
        "PackageManager" => 150,
        _ => 150,
    }
}

lazy_static! {
    pub(super) static ref REGEX_SET: RegexSet =
        RegexSet::new(PATTERNS).expect("invalid regex patterns");
    pub(super) static ref COMPILED: Vec<Regex> = PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("invalid regex"))
        .collect();
    pub(super) static ref ENV_PREFIX: Regex =
        Regex::new(r"^(?:sudo\s+|env\s+|[A-Z_][A-Z0-9_]*=[^\s]*\s+)+").unwrap();
    // Git global options that appear before the subcommand: -C <path>, -c <key=val>,
    // --git-dir <dir>, --work-tree <dir>, and flag-only options (#163)
    pub(super) static ref GIT_GLOBAL_OPT: Regex =
        Regex::new(r"^(?:(?:-C\s+\S+|-c\s+\S+|--git-dir(?:=\S+|\s+\S+)|--work-tree(?:=\S+|\s+\S+)|--no-pager|--no-optional-locks|--bare|--literal-pathspecs)\s+)+").unwrap();
}

/// Strip git global options before the subcommand (#163).
/// `git -C /tmp status` → `git status`, preserving the rest.
/// Returns the original string unchanged if not a git command.
pub(super) fn strip_git_global_opts(cmd: &str) -> String {
    // Only applies to commands starting with "git "
    if !cmd.starts_with("git ") {
        return cmd.to_string();
    }
    let after_git = &cmd[4..]; // skip "git "
    let stripped = GIT_GLOBAL_OPT.replace(after_git, "");
    format!("git {}", stripped.trim())
}

/// Normalize absolute binary paths: `/usr/bin/grep -rn foo` → `grep -rn foo` (#485)
/// Only strips if the first word contains a `/` (Unix path).
pub(super) fn strip_absolute_path(cmd: &str) -> String {
    let first_space = cmd.find(' ');
    let first_word = match first_space {
        Some(pos) => &cmd[..pos],
        None => cmd,
    };
    if first_word.contains('/') {
        // Extract basename
        let basename = first_word.rsplit('/').next().unwrap_or(first_word);
        if basename.is_empty() {
            return cmd.to_string();
        }
        match first_space {
            Some(pos) => format!("{}{}", basename, &cmd[pos..]),
            None => basename.to_string(),
        }
    } else {
        cmd.to_string()
    }
}

/// Extract the base command (first word, or first two if it looks like a subcommand pattern).
pub(super) fn extract_base_command(cmd: &str) -> &str {
    let parts: Vec<&str> = cmd.splitn(3, char::is_whitespace).collect();
    match parts.len() {
        0 => "",
        1 => parts[0],
        _ => {
            let second = parts[1];
            // If the second token looks like a subcommand (no leading -)
            if !second.starts_with('-') && !second.contains('/') && !second.contains('.') {
                // Return "cmd subcmd"
                let end = cmd
                    .find(char::is_whitespace)
                    .and_then(|i| {
                        let rest = &cmd[i..];
                        let trimmed = rest.trim_start();
                        trimmed
                            .find(char::is_whitespace)
                            .map(|j| i + (rest.len() - trimmed.len()) + j)
                    })
                    .unwrap_or(cmd.len());
                &cmd[..end]
            } else {
                parts[0]
            }
        }
    }
}

/// Classify a single (already-split) command.
pub fn classify_command(cmd: &str) -> Classification {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Classification::Ignored;
    }

    // Check ignored
    for exact in IGNORED_EXACT {
        if trimmed == *exact {
            return Classification::Ignored;
        }
    }
    for prefix in IGNORED_PREFIXES {
        if trimmed.starts_with(prefix) {
            return Classification::Ignored;
        }
    }

    // Strip env prefixes (sudo, env VAR=val, VAR=val)
    let stripped = ENV_PREFIX.replace(trimmed, "");
    let cmd_clean = stripped.trim();
    if cmd_clean.is_empty() {
        return Classification::Ignored;
    }

    // Normalize absolute binary paths: /usr/bin/grep → grep (#485)
    let cmd_normalized = strip_absolute_path(cmd_clean);
    // Strip git global options: git -C /tmp status → git status (#163)
    let cmd_normalized = strip_git_global_opts(&cmd_normalized);
    let cmd_clean = cmd_normalized.as_str();

    // Exclude cat/head/tail with redirect operators — these are writes, not reads (#315)
    if cmd_clean.starts_with("cat ")
        || cmd_clean.starts_with("head ")
        || cmd_clean.starts_with("tail ")
    {
        let has_redirect = cmd_clean
            .split_whitespace()
            .skip(1)
            .any(|t| t.starts_with('>') || t == "<" || t.starts_with(">>"));
        if has_redirect {
            return Classification::Unsupported {
                base_command: cmd_clean
                    .split_whitespace()
                    .next()
                    .unwrap_or("cat")
                    .to_string(),
            };
        }
    }

    // Fast check with RegexSet — take the last (most specific) match
    let matches: Vec<usize> = REGEX_SET.matches(cmd_clean).into_iter().collect();
    if let Some(&idx) = matches.last() {
        let rule = &RULES[idx];

        // Extract subcommand for savings override and status detection
        let (savings, status) = if let Some(caps) = COMPILED[idx].captures(cmd_clean) {
            if let Some(sub) = caps.get(1) {
                let subcmd = sub.as_str();
                // Check if this subcommand has a special status
                let status = rule
                    .subcmd_status
                    .iter()
                    .find(|(s, _)| *s == subcmd)
                    .map(|(_, st)| *st)
                    .unwrap_or(super::report::CrunchStatus::Existing);

                // Check if this subcommand has custom savings
                let savings = rule
                    .subcmd_savings
                    .iter()
                    .find(|(s, _)| *s == subcmd)
                    .map(|(_, pct)| *pct)
                    .unwrap_or(rule.savings_pct);

                (savings, status)
            } else {
                (rule.savings_pct, super::report::CrunchStatus::Existing)
            }
        } else {
            (rule.savings_pct, super::report::CrunchStatus::Existing)
        };

        Classification::Supported {
            crunch_equivalent: rule.crunch_cmd,
            category: rule.category,
            estimated_savings_pct: savings,
            status,
        }
    } else {
        // Extract base command for unsupported
        let base = extract_base_command(cmd_clean);
        if base.is_empty() {
            Classification::Ignored
        } else {
            Classification::Unsupported {
                base_command: base.to_string(),
            }
        }
    }
}

/// Check if a command has CRUNCH_DISABLED= prefix in its env prefix portion.
pub fn has_crunch_disabled_prefix(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    let stripped = ENV_PREFIX.replace(trimmed, "");
    let prefix_len = trimmed.len() - stripped.len();
    let prefix_part = &trimmed[..prefix_len];
    prefix_part.contains("CRUNCH_DISABLED=")
}

/// Strip CRUNCH_DISABLED=X and other env prefixes, return the actual command.
pub fn strip_disabled_prefix(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    let stripped = ENV_PREFIX.replace(trimmed, "");
    // stripped is a Cow<str> that borrows from trimmed when no replacement happens.
    // We need to return a &str into the original, so compute the offset.
    let prefix_len = trimmed.len() - stripped.len();
    trimmed[prefix_len..].trim_start()
}

/// Split a command chain on `&&`, `||`, `;` outside quotes.
/// For pipes `|`, only keep the first command.
/// Lines with `<<` (heredoc) or `$((` are returned whole.
pub fn split_command_chain(cmd: &str) -> Vec<&str> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    // Heredoc or arithmetic expansion: treat as single command
    if trimmed.contains("<<") || trimmed.contains("$((") {
        return vec![trimmed];
    }

    let mut results = Vec::new();
    let mut start = 0;
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut pipe_seen = false;

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
                    // ||
                    let segment = trimmed[start..i].trim();
                    if !segment.is_empty() {
                        results.push(segment);
                    }
                    i += 2;
                    start = i;
                } else {
                    // pipe: keep only first command
                    let segment = trimmed[start..i].trim();
                    if !segment.is_empty() {
                        results.push(segment);
                    }
                    pipe_seen = true;
                    break;
                }
            }
            b'&' if !in_single && !in_double && i + 1 < len && bytes[i + 1] == b'&' => {
                let segment = trimmed[start..i].trim();
                if !segment.is_empty() {
                    results.push(segment);
                }
                i += 2;
                start = i;
            }
            b';' if !in_single && !in_double => {
                let segment = trimmed[start..i].trim();
                if !segment.is_empty() {
                    results.push(segment);
                }
                i += 1;
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }

    if !pipe_seen && start < len {
        let segment = trimmed[start..].trim();
        if !segment.is_empty() {
            results.push(segment);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::super::report::CrunchStatus;
    use super::super::rules::PATTERNS;
    use super::super::rules::RULES;
    use super::*;

    #[test]
    fn test_classify_git_status() {
        assert_eq!(
            classify_command("git status"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_git_diff_cached() {
        assert_eq!(
            classify_command("git diff --cached"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_test_filter() {
        assert_eq!(
            classify_command("cargo test filter::"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                category: "Cargo",
                estimated_savings_pct: 90.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_npx_tsc() {
        assert_eq!(
            classify_command("npx tsc --noEmit"),
            Classification::Supported {
                crunch_equivalent: "crunch tsc",
                category: "Build",
                estimated_savings_pct: 83.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cat_file() {
        assert_eq!(
            classify_command("cat src/main.rs"),
            Classification::Supported {
                crunch_equivalent: "crunch read",
                category: "Files",
                estimated_savings_pct: 60.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cat_redirect_not_supported() {
        // cat > file and cat >> file are writes, not reads — should not be classified as supported
        let write_commands = [
            "cat > /tmp/output.txt",
            "cat >> /tmp/output.txt",
            "cat file.txt > output.txt",
            "cat -n file.txt >> log.txt",
            "head -10 README.md > output.txt",
            "tail -f app.log > /dev/null",
        ];
        for cmd in &write_commands {
            if let Classification::Supported { .. } = classify_command(cmd) {
                panic!("{} should NOT be classified as Supported", cmd)
            }
            // Unsupported or Ignored is fine
        }
    }

    #[test]
    fn test_classify_cd_ignored() {
        assert_eq!(classify_command("cd /tmp"), Classification::Ignored);
    }

    #[test]
    fn test_classify_rtk_already() {
        assert_eq!(
            classify_command("crunch git status"),
            Classification::Ignored
        );
    }

    #[test]
    fn test_classify_echo_ignored() {
        assert_eq!(
            classify_command("echo hello world"),
            Classification::Ignored
        );
    }

    #[test]
    fn test_classify_htop_unsupported() {
        match classify_command("htop -d 10") {
            Classification::Unsupported { base_command } => {
                assert_eq!(base_command, "htop");
            }
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_env_prefix_stripped() {
        assert_eq!(
            classify_command("GIT_SSH_COMMAND=ssh git push"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_sudo_stripped() {
        assert_eq!(
            classify_command("sudo docker ps"),
            Classification::Supported {
                crunch_equivalent: "crunch docker",
                category: "Infra",
                estimated_savings_pct: 85.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_check() {
        assert_eq!(
            classify_command("cargo check"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_check_all_targets() {
        assert_eq!(
            classify_command("cargo check --all-targets"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_fmt_passthrough() {
        assert_eq!(
            classify_command("cargo fmt"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Passthrough,
            }
        );
    }

    #[test]
    fn test_classify_cargo_clippy_savings() {
        assert_eq!(
            classify_command("cargo clippy --all-targets"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_patterns_rules_length_match() {
        assert_eq!(
            PATTERNS.len(),
            RULES.len(),
            "PATTERNS and RULES must be aligned"
        );
    }

    #[test]
    fn test_registry_covers_all_cargo_subcommands() {
        // Verify that every CargoCommand variant (Build, Test, Clippy, Check, Fmt)
        // except Other has a matching pattern in the registry
        for subcmd in ["build", "test", "clippy", "check", "fmt"] {
            let cmd = format!("cargo {subcmd}");
            match classify_command(&cmd) {
                Classification::Supported { .. } => {}
                other => panic!("cargo {subcmd} should be Supported, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_registry_covers_all_git_subcommands() {
        // Verify that every GitCommand subcommand has a matching pattern
        for subcmd in [
            "status", "log", "diff", "show", "add", "commit", "push", "pull", "branch", "fetch",
            "stash", "worktree",
        ] {
            let cmd = format!("git {subcmd}");
            match classify_command(&cmd) {
                Classification::Supported { .. } => {}
                other => panic!("git {subcmd} should be Supported, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_classify_find_not_blocked_by_fi() {
        // Regression: "fi" in IGNORED_PREFIXES used to shadow "find" commands
        // because "find".starts_with("fi") is true. "fi" should only match exactly.
        assert_eq!(
            classify_command("find . -name foo"),
            Classification::Supported {
                crunch_equivalent: "crunch find",
                category: "Files",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_fi_still_ignored_exact() {
        // Bare "fi" (shell keyword) should still be ignored
        assert_eq!(classify_command("fi"), Classification::Ignored);
    }

    #[test]
    fn test_done_still_ignored_exact() {
        // Bare "done" (shell keyword) should still be ignored
        assert_eq!(classify_command("done"), Classification::Ignored);
    }

    #[test]
    fn test_split_chain_and() {
        assert_eq!(split_command_chain("a && b"), vec!["a", "b"]);
    }

    #[test]
    fn test_split_chain_semicolon() {
        assert_eq!(split_command_chain("a ; b"), vec!["a", "b"]);
    }

    #[test]
    fn test_split_pipe_first_only() {
        assert_eq!(split_command_chain("a | b"), vec!["a"]);
    }

    #[test]
    fn test_split_single() {
        assert_eq!(split_command_chain("git status"), vec!["git status"]);
    }

    #[test]
    fn test_split_quoted_and() {
        assert_eq!(
            split_command_chain(r#"echo "a && b""#),
            vec![r#"echo "a && b""#]
        );
    }

    #[test]
    fn test_split_heredoc_no_split() {
        let cmd = "cat <<'EOF'\nhello && world\nEOF";
        assert_eq!(split_command_chain(cmd), vec![cmd]);
    }

    #[test]
    fn test_classify_mypy() {
        assert_eq!(
            classify_command("mypy src/"),
            Classification::Supported {
                crunch_equivalent: "crunch mypy",
                category: "Build",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_python_m_mypy() {
        assert_eq!(
            classify_command("python3 -m mypy --strict"),
            Classification::Supported {
                crunch_equivalent: "crunch mypy",
                category: "Build",
                estimated_savings_pct: 80.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_gh_release() {
        assert!(matches!(
            classify_command("gh release list"),
            Classification::Supported {
                crunch_equivalent: "crunch gh",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_cargo_install() {
        assert!(matches!(
            classify_command("cargo install rtk"),
            Classification::Supported {
                crunch_equivalent: "crunch cargo",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_docker_run() {
        assert!(matches!(
            classify_command("docker run --rm ubuntu bash"),
            Classification::Supported {
                crunch_equivalent: "crunch docker",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_docker_exec() {
        assert!(matches!(
            classify_command("docker exec -it mycontainer bash"),
            Classification::Supported {
                crunch_equivalent: "crunch docker",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_docker_build() {
        assert!(matches!(
            classify_command("docker build -t myimage ."),
            Classification::Supported {
                crunch_equivalent: "crunch docker",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_kubectl_describe() {
        assert!(matches!(
            classify_command("kubectl describe pod mypod"),
            Classification::Supported {
                crunch_equivalent: "crunch kubectl",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_kubectl_apply() {
        assert!(matches!(
            classify_command("kubectl apply -f deploy.yaml"),
            Classification::Supported {
                crunch_equivalent: "crunch kubectl",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_tree() {
        assert!(matches!(
            classify_command("tree src/"),
            Classification::Supported {
                crunch_equivalent: "crunch tree",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_diff() {
        assert!(matches!(
            classify_command("diff file1.txt file2.txt"),
            Classification::Supported {
                crunch_equivalent: "crunch diff",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_swift_test() {
        assert!(matches!(
            classify_command("swift test"),
            Classification::Supported {
                crunch_equivalent: "crunch swift",
                category: "Build",
                estimated_savings_pct: 90.0,
                status: CrunchStatus::Existing,
            }
        ));
    }

    #[test]
    fn test_classify_aws() {
        assert!(matches!(
            classify_command("aws s3 ls"),
            Classification::Supported {
                crunch_equivalent: "crunch aws",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_aws_ec2() {
        assert!(matches!(
            classify_command("aws ec2 describe-instances"),
            Classification::Supported {
                crunch_equivalent: "crunch aws",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_psql() {
        assert!(matches!(
            classify_command("psql -U postgres"),
            Classification::Supported {
                crunch_equivalent: "crunch psql",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_psql_url() {
        assert!(matches!(
            classify_command("psql postgres://localhost/mydb"),
            Classification::Supported {
                crunch_equivalent: "crunch psql",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_ruff_check() {
        assert!(matches!(
            classify_command("ruff check ."),
            Classification::Supported {
                crunch_equivalent: "crunch ruff",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_ruff_format() {
        assert!(matches!(
            classify_command("ruff format src/"),
            Classification::Supported {
                crunch_equivalent: "crunch ruff",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_pytest() {
        assert!(matches!(
            classify_command("pytest tests/"),
            Classification::Supported {
                crunch_equivalent: "crunch pytest",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_python_m_pytest() {
        assert!(matches!(
            classify_command("python -m pytest tests/"),
            Classification::Supported {
                crunch_equivalent: "crunch pytest",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_pip_list() {
        assert!(matches!(
            classify_command("pip list"),
            Classification::Supported {
                crunch_equivalent: "crunch pip",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_uv_pip_list() {
        assert!(matches!(
            classify_command("uv pip list"),
            Classification::Supported {
                crunch_equivalent: "crunch pip",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_go_test() {
        assert!(matches!(
            classify_command("go test ./..."),
            Classification::Supported {
                crunch_equivalent: "crunch go",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_go_build() {
        assert!(matches!(
            classify_command("go build ./..."),
            Classification::Supported {
                crunch_equivalent: "crunch go",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_go_vet() {
        assert!(matches!(
            classify_command("go vet ./..."),
            Classification::Supported {
                crunch_equivalent: "crunch go",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_golangci_lint() {
        assert!(matches!(
            classify_command("golangci-lint run"),
            Classification::Supported {
                crunch_equivalent: "crunch golangci-lint",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_vitest() {
        assert!(matches!(
            classify_command("vitest run"),
            Classification::Supported {
                crunch_equivalent: "crunch vitest",
                ..
            }
        ));
    }

    #[test]
    fn test_classify_prisma() {
        assert!(matches!(
            classify_command("npx prisma migrate dev"),
            Classification::Supported {
                crunch_equivalent: "crunch prisma",
                ..
            }
        ));
    }

    // --- #485: absolute path normalization ---

    #[test]
    fn test_classify_absolute_path_grep() {
        assert_eq!(
            classify_command("/usr/bin/grep -rni pattern"),
            Classification::Supported {
                crunch_equivalent: "crunch grep",
                category: "Files",
                estimated_savings_pct: 75.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_absolute_path_ls() {
        assert_eq!(
            classify_command("/bin/ls -la"),
            Classification::Supported {
                crunch_equivalent: "crunch ls",
                category: "Files",
                estimated_savings_pct: 65.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_absolute_path_git() {
        assert_eq!(
            classify_command("/usr/local/bin/git status"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_absolute_path_no_args() {
        // /usr/bin/find alone → still classified
        assert_eq!(
            classify_command("/usr/bin/find ."),
            Classification::Supported {
                crunch_equivalent: "crunch find",
                category: "Files",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_strip_absolute_path_helper() {
        assert_eq!(strip_absolute_path("/usr/bin/grep -rn foo"), "grep -rn foo");
        assert_eq!(strip_absolute_path("/bin/ls -la"), "ls -la");
        assert_eq!(strip_absolute_path("grep -rn foo"), "grep -rn foo");
        assert_eq!(strip_absolute_path("/usr/local/bin/git"), "git");
    }

    // --- #163: git global options ---

    #[test]
    fn test_classify_git_with_dash_c_path() {
        assert_eq!(
            classify_command("git -C /tmp status"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_git_no_pager_log() {
        assert_eq!(
            classify_command("git --no-pager log -5"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_git_git_dir() {
        assert_eq!(
            classify_command("git --git-dir /tmp/.git status"),
            Classification::Supported {
                crunch_equivalent: "crunch git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: CrunchStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_git_dash_c() {
        let result = classify_command("git -C /tmp status");
        assert!(
            matches!(
                result,
                Classification::Supported {
                    crunch_equivalent: "crunch git",
                    ..
                }
            ),
            "git -C should be classified as supported, got: {:?}",
            result
        );
    }

    #[test]
    fn test_strip_git_global_opts_helper() {
        assert_eq!(strip_git_global_opts("git -C /tmp status"), "git status");
        assert_eq!(strip_git_global_opts("git --no-pager log"), "git log");
        assert_eq!(strip_git_global_opts("git status"), "git status");
        assert_eq!(strip_git_global_opts("cargo test"), "cargo test");
    }

    // --- #508: CRUNCH_DISABLED detection helpers ---

    #[test]
    fn test_has_crunch_disabled_prefix() {
        assert!(has_crunch_disabled_prefix("CRUNCH_DISABLED=1 git status"));
        assert!(has_crunch_disabled_prefix(
            "FOO=1 CRUNCH_DISABLED=1 cargo test"
        ));
        assert!(has_crunch_disabled_prefix(
            "CRUNCH_DISABLED=true git log --oneline"
        ));
        assert!(!has_crunch_disabled_prefix("git status"));
        assert!(!has_crunch_disabled_prefix("crunch git status"));
        assert!(!has_crunch_disabled_prefix("SOME_VAR=1 git status"));
    }

    #[test]
    fn test_strip_disabled_prefix() {
        assert_eq!(
            strip_disabled_prefix("CRUNCH_DISABLED=1 git status"),
            "git status"
        );
        assert_eq!(
            strip_disabled_prefix("FOO=1 CRUNCH_DISABLED=1 cargo test"),
            "cargo test"
        );
        assert_eq!(strip_disabled_prefix("git status"), "git status");
    }

    // --- Ensure PATTERNS and RULES stay aligned after modifications ---

    #[test]
    fn test_patterns_rules_aligned_after_aws_psql() {
        // If this fails, someone added a PATTERN without a matching RULE (or vice versa)
        assert_eq!(
            PATTERNS.len(),
            RULES.len(),
            "PATTERNS[{}] != RULES[{}] — they must stay 1:1",
            PATTERNS.len(),
            RULES.len()
        );
    }

    // --- All RULES have non-empty crunch_cmd and at least one rewrite_prefix ---

    #[test]
    fn test_all_rules_have_valid_crunch_cmd() {
        for rule in RULES {
            assert!(
                !rule.crunch_cmd.is_empty(),
                "Rule with empty crunch_cmd found"
            );
            assert!(
                rule.crunch_cmd.starts_with("crunch "),
                "crunch_cmd '{}' must start with 'crunch '",
                rule.crunch_cmd
            );
            assert!(
                !rule.rewrite_prefixes.is_empty(),
                "Rule '{}' has no rewrite_prefixes",
                rule.crunch_cmd
            );
        }
    }

    // --- Every PATTERN compiles to a valid Regex ---

    #[test]
    fn test_all_patterns_are_valid_regex() {
        use regex::Regex;
        for (i, pattern) in PATTERNS.iter().enumerate() {
            assert!(
                Regex::new(pattern).is_ok(),
                "PATTERNS[{i}] = '{pattern}' is not a valid regex"
            );
        }
    }
}
