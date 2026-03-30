use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Minimum output size to tee (smaller outputs don't need recovery)
const MIN_TEE_SIZE: usize = 500;

/// Default max files to keep in tee directory
const DEFAULT_MAX_FILES: usize = 20;

/// Default max file size (1MB)
const DEFAULT_MAX_FILE_SIZE: usize = 1_048_576;

/// Sanitize a command slug for use in filenames.
/// Replaces non-alphanumeric chars (except underscore/hyphen) with underscore,
/// truncates at 40 chars.
fn sanitize_slug(slug: &str) -> String {
    let sanitized: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() > 40 {
        // Safe to slice at byte offset: the .map() above guarantees all chars are ASCII
        sanitized[..40].to_string()
    } else {
        sanitized
    }
}

/// Rotate old tee files: keep only the last `max_files`, delete oldest.
fn cleanup_old_files(dir: &std::path::Path, max_files: usize) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .collect();

    if entries.len() <= max_files {
        return;
    }

    // Sort by modification time (oldest first) for true chronological rotation
    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    let to_remove = entries.len() - max_files;
    for entry in entries.iter().take(to_remove) {
        let _ = std::fs::remove_file(entry.path());
    }
}

/// Format the hint line with ~ shorthand for home directory.
fn format_hint(path: &std::path::Path) -> String {
    let display = if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            format!("~/{}", relative.display())
        } else {
            path.display().to_string()
        }
    } else {
        path.display().to_string()
    };

    format!("[full output: {}]", display)
}

/// Detect project name from git root or cwd basename.
pub fn detect_project_name() -> String {
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
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

static CACHED_PROJECT: OnceLock<String> = OnceLock::new();

/// Get the project name, detecting only once per process.
pub fn cached_project_name() -> &'static str {
    CACHED_PROJECT.get_or_init(detect_project_name)
}

/// Derive scope from command args. Falls back to "all".
pub fn detect_scope(args: &[String]) -> String {
    for arg in args {
        if arg.starts_with('-') {
            continue;
        }
        let path = std::path::Path::new(arg);
        if let Some(stem) = path.file_stem() {
            return sanitize_slug(&stem.to_string_lossy());
        }
    }
    "all".to_string()
}

/// Build log path using a custom base directory.
pub fn build_log_path_with_base(base: &str, project: &str, tool: &str, scope: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!(
        "{}-{}-{}.log",
        sanitize_slug(tool),
        sanitize_slug(scope),
        timestamp
    );
    PathBuf::from(base)
        .join(sanitize_slug(project))
        .join(filename)
}

/// Build log path: {base_dir}/{project}/{tool}-{scope}-{timestamp}.log
/// Uses `[tee] directory` from config if set, otherwise defaults to `/tmp/crunch`.
pub fn build_log_path(project: &str, tool: &str, scope: &str) -> PathBuf {
    let config = crate::config::cached_config();
    let base = config
        .tee
        .directory
        .as_deref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/tmp/crunch".to_string());
    build_log_path_with_base(&base, project, tool, scope)
}

/// Write raw output to project-scoped tee file.
pub fn tee_raw_scoped(raw: &str, tool: &str, args: &[String], exit_code: i32) -> Option<PathBuf> {
    if std::env::var("CRUNCH_TEE").ok().as_deref() == Some("0") {
        return None;
    }

    let config = crate::config::cached_config().tee.clone();

    if !config.is_tool_enabled(tool) {
        return None;
    }

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

    let project = cached_project_name();
    let scope = detect_scope(args);
    let log_path = build_log_path(project, tool, &scope);

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    // Tail-biased truncation: keep head (10%) + tail (90%) with gap marker.
    // Pytest summary/failures are at the tail — preserving them is critical.
    let max_file_size = config.max_file_size;
    let content = truncate_tail_biased(raw, max_file_size);

    std::fs::write(&log_path, content).ok()?;

    if let Some(parent) = log_path.parent() {
        cleanup_old_files(parent, config.max_files);
    }

    Some(log_path)
}

/// Truncate large output using a tail-biased strategy: 10% head + 90% tail.
///
/// Test frameworks (pytest, cargo test, go test) put the summary and failure
/// details at the END of output. Head-only truncation discards the most
/// important part. This function keeps the beginning (session starts, collection)
/// AND the end (failures, summary) with a gap marker in the middle.
fn truncate_tail_biased(raw: &str, max_size: usize) -> String {
    if raw.len() <= max_size {
        return raw.to_string();
    }

    // Budget for the gap marker. The actual marker includes a dynamic byte count,
    // so we add 20 bytes of headroom beyond the static template.
    let gap_len = "\n\n--- truncated: 000000000 bytes omitted ---\n\n".len();

    // Need at least enough budget for gap + some content
    if max_size < gap_len + 200 {
        // Too small for split strategy — just keep tail
        let start = snap_to_char_boundary(raw, raw.len().saturating_sub(max_size));
        return format!("--- truncated: start omitted ---\n{}", &raw[start..]);
    }

    let budget = max_size - gap_len;
    let head_budget = budget / 10; // 10% for head
    let tail_budget = budget - head_budget; // 90% for tail

    // Find safe boundaries (char boundary + snap to line boundary)
    let head_end = snap_to_line_end(raw, head_budget);
    let tail_start_byte = raw.len().saturating_sub(tail_budget);
    let tail_start = snap_to_line_start(raw, tail_start_byte);

    let omitted = tail_start.saturating_sub(head_end);

    format!(
        "{}\n\n--- truncated: {} bytes omitted ---\n\n{}",
        &raw[..head_end],
        omitted,
        &raw[tail_start..]
    )
}

/// Find the nearest char boundary at or after `pos`.
fn snap_to_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

/// Find the end of the last complete line within `budget` bytes.
fn snap_to_line_end(s: &str, budget: usize) -> usize {
    let mut end = budget.min(s.len());
    // Snap to char boundary first to avoid panic on s[..end] with multi-byte UTF-8
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    // Walk backwards to find a newline
    match s[..end].rfind('\n') {
        Some(pos) => pos + 1, // include the newline
        None => end,          // no newline found, use full budget
    }
}

/// Find the start of the first complete line at or after `pos`.
fn snap_to_line_start(s: &str, pos: usize) -> usize {
    let p = snap_to_char_boundary(s, pos);
    // Walk forward to find a newline, then start after it
    match s[p..].find('\n') {
        Some(offset) => p + offset + 1,
        None => p,
    }
}

/// Convenience: tee + format hint for project-scoped logs.
pub fn tee_and_hint_scoped(
    raw: &str,
    tool: &str,
    args: &[String],
    exit_code: i32,
) -> Option<String> {
    let path = tee_raw_scoped(raw, tool, args, exit_code)?;
    Some(format_hint(&path))
}

/// TeeMode controls when tee writes files.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TeeMode {
    Failures,
    #[default]
    Always,
    Never,
}

/// Per-tool tee override.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeeToolOverride {
    pub enabled: bool,
}

/// Configuration for the tee feature.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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

impl Default for TeeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: TeeMode::default(),
            max_files: DEFAULT_MAX_FILES,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            directory: None,
            overrides: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_sanitize_slug() {
        assert_eq!(sanitize_slug("cargo_test"), "cargo_test");
        assert_eq!(sanitize_slug("cargo test"), "cargo_test");
        assert_eq!(sanitize_slug("cargo-test"), "cargo-test");
        assert_eq!(sanitize_slug("go/test/./pkg"), "go_test___pkg");
        // Truncate at 40
        let long = "a".repeat(50);
        assert_eq!(sanitize_slug(&long).len(), 40);
    }

    #[test]
    fn test_cleanup_old_files() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();

        // Create 25 .log files
        for i in 0..25 {
            let filename = format!("{:010}_{}.log", 1000000 + i, "test");
            fs::write(dir.join(&filename), "content").unwrap();
        }

        cleanup_old_files(dir, 20);

        let remaining: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(remaining.len(), 20);

        // Oldest 5 should be removed
        for i in 0..5 {
            let filename = format!("{:010}_{}.log", 1000000 + i, "test");
            assert!(!dir.join(&filename).exists());
        }
        // Newest 20 should remain
        for i in 5..25 {
            let filename = format!("{:010}_{}.log", 1000000 + i, "test");
            assert!(dir.join(&filename).exists());
        }
    }

    #[test]
    fn test_cleanup_rotates_by_mtime_not_filename() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();

        // Create 20 "ruff" logs first (they'll have older mtime)
        for i in 0..20 {
            let filename = format!("ruff-check-{:04}.log", i);
            fs::write(dir.join(&filename), "ruff content").unwrap();
        }

        // Small sleep so the cargo log has a strictly newer mtime
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Create 1 "cargo" log (newest by mtime, but sorts before "ruff" alphabetically)
        let cargo_log = dir.join("cargo_test-all-20260328.log");
        fs::write(&cargo_log, "cargo content").unwrap();

        // 21 files, max 20 — the oldest ruff log should be deleted,
        // NOT the cargo log (which would be deleted under alphabetical sort)
        cleanup_old_files(dir, 20);

        assert!(
            cargo_log.exists(),
            "Newest file (cargo) must survive rotation even though it sorts first alphabetically"
        );

        let remaining: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
            .collect();
        assert_eq!(remaining.len(), 20);
    }

    #[test]
    fn test_format_hint() {
        let path = PathBuf::from("/tmp/crunch/tee/123_cargo_test.log");
        let hint = format_hint(&path);
        assert!(hint.starts_with("[full output: "));
        assert!(hint.ends_with(']'));
        assert!(hint.contains("123_cargo_test.log"));
    }

    #[test]
    fn test_tee_config_default() {
        let config = TeeConfig::default();
        assert!(config.enabled);
        assert_eq!(config.mode, TeeMode::Always);
        assert_eq!(config.max_files, 20);
        assert_eq!(config.max_file_size, 1_048_576);
        assert!(config.directory.is_none());
        assert!(config.overrides.is_empty());
    }

    #[test]
    fn test_tee_config_deserialize() {
        let toml_str = r#"
enabled = true
mode = "always"
max_files = 10
max_file_size = 524288
directory = "/tmp/crunch-tee"
"#;
        let config: TeeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.mode, TeeMode::Always);
        assert_eq!(config.max_files, 10);
        assert_eq!(config.max_file_size, 524288);
        assert_eq!(config.directory, Some(PathBuf::from("/tmp/crunch-tee")));

        // Round-trip
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: TeeConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.mode, TeeMode::Always);
        assert_eq!(deserialized.max_files, 10);
    }

    #[test]
    fn test_tee_mode_serde() {
        // Test all modes via JSON
        let mode: TeeMode = serde_json::from_str(r#""always""#).unwrap();
        assert_eq!(mode, TeeMode::Always);

        let mode: TeeMode = serde_json::from_str(r#""failures""#).unwrap();
        assert_eq!(mode, TeeMode::Failures);

        let mode: TeeMode = serde_json::from_str(r#""never""#).unwrap();
        assert_eq!(mode, TeeMode::Never);
    }

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
        assert!(config.is_tool_enabled("cargo")); // inherits global
    }

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
        let name = detect_project_name();
        assert!(!name.is_empty());
    }

    #[test]
    fn test_cached_project_name_consistent() {
        let n1 = cached_project_name();
        let n2 = cached_project_name();
        assert_eq!(n1, n2);
    }

    #[test]
    fn test_detect_scope_from_args() {
        assert_eq!(
            detect_scope(&["tests/test_build.py".to_string()]),
            "test_build"
        );
        assert_eq!(detect_scope(&["-x".to_string(), "-q".to_string()]), "all");
        assert_eq!(detect_scope(&[]), "all");
    }

    #[test]
    fn test_tee_default_is_always() {
        let config = TeeConfig::default();
        assert_eq!(config.mode, TeeMode::Always);
    }

    #[test]
    fn test_hint_shown_for_tmp_path() {
        let path = PathBuf::from("/tmp/crunch/myproject/pytest-test_build-20260327-143012.log");
        let hint = format_hint(&path);
        assert!(hint.contains("/tmp/crunch/myproject/pytest-test_build-20260327-143012.log"));
    }

    #[test]
    fn test_build_log_path_uses_custom_directory() {
        let path = build_log_path_with_base("/var/log/crunch", "myproject", "pytest", "test_build");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.starts_with("/var/log/crunch/myproject/"),
            "Should use custom base dir, got: {}",
            path_str
        );
        assert!(path_str.contains("pytest-test_build-"));
        assert!(path_str.ends_with(".log"));
    }

    #[test]
    fn test_build_log_path_default_base() {
        let path = build_log_path_with_base("/tmp/crunch", "proj", "git", "all");
        let path_str = path.to_string_lossy();
        assert!(path_str.starts_with("/tmp/crunch/proj/"));
    }

    // ── Phase 5: Tail-biased truncation tests ──

    #[test]
    fn test_truncate_small_output_unchanged() {
        let raw = "short output\nsummary: 5 passed";
        let result = truncate_tail_biased(raw, 1_048_576);
        assert_eq!(result, raw);
    }

    #[test]
    fn test_truncate_preserves_tail() {
        // Create output where the summary is at the very end
        let header = "=== test session starts ===\n".repeat(100); // ~2800 bytes
        let middle = "tests/test_foo.py .....\n".repeat(500); // ~11500 bytes
        let summary = "=== 500 passed in 5.00s ===\n";
        let raw = format!("{}{}{}", header, middle, summary);

        // Truncate to 5000 bytes
        let result = truncate_tail_biased(&raw, 5000);

        assert!(
            result.contains("500 passed"),
            "Summary must be preserved in tail, got: ...{}",
            &result[result.len().saturating_sub(200)..]
        );
        assert!(result.contains("truncated"));
        assert!(result.len() <= 5200); // some slack for gap marker
    }

    #[test]
    fn test_truncate_preserves_head() {
        let header = "=== test session starts ===\ncollected 100 items\n";
        let middle = "x".repeat(100_000);
        let summary = "\n=== 100 passed in 10.00s ===\n";
        let raw = format!("{}{}{}", header, middle, summary);

        let result = truncate_tail_biased(&raw, 5000);

        assert!(
            result.contains("test session starts"),
            "Head must be preserved: {}",
            &result[..200.min(result.len())]
        );
        assert!(result.contains("100 passed"), "Tail must be preserved");
    }

    #[test]
    fn test_truncate_utf8_safety() {
        // Create output with multi-byte UTF-8 chars near truncation boundary
        let header = "header\n";
        let emoji_line = "test 🎉 passed\n".repeat(10000); // lots of 4-byte emojis
        let summary = "=== 5 passed in 0.50s ===\n";
        let raw = format!("{}{}{}", header, emoji_line, summary);

        let result = truncate_tail_biased(&raw, 5000);
        // Should not panic and should be valid UTF-8
        assert!(result.is_ascii() || !result.is_empty());
        assert!(
            result.contains("5 passed"),
            "Summary preserved: {}",
            &result[result.len().saturating_sub(100)..]
        );
    }

    #[test]
    fn test_truncate_gap_marker_present() {
        let raw = "a\n".repeat(100_000);
        let result = truncate_tail_biased(&raw, 5000);
        assert!(
            result.contains("truncated") && result.contains("omitted"),
            "Gap marker must be present: {}",
            &result[..500.min(result.len())]
        );
    }

    #[test]
    fn test_hint_format_no_tilde_for_tmp() {
        let path = PathBuf::from("/tmp/crunch/myproject/pytest-all-20260327-143012.log");
        let hint = format_hint(&path);
        assert!(hint.starts_with("[full output: /tmp/crunch/"));
    }
}
