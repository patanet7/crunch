use crate::config::Config;
use std::collections::HashMap;
use std::path::PathBuf;

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
        sanitized[..40].to_string()
    } else {
        sanitized
    }
}

/// Get the tee directory, respecting config and env overrides.
fn get_tee_dir(config: &Config) -> Option<PathBuf> {
    // Env var override
    if let Ok(dir) = std::env::var("CRUNCH_TEE_DIR") {
        return Some(PathBuf::from(dir));
    }

    // Config override
    if let Some(ref dir) = config.tee.directory {
        return Some(dir.clone());
    }

    // Default: ~/.local/share/crunch/tee/
    dirs::data_local_dir().map(|d| d.join("crunch").join("tee"))
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

    // Sort by filename (which starts with epoch timestamp = chronological)
    entries.sort_by_key(|e| e.file_name());

    let to_remove = entries.len() - max_files;
    for entry in entries.iter().take(to_remove) {
        let _ = std::fs::remove_file(entry.path());
    }
}

/// Check if tee should be skipped based on config, mode, exit code, and size.
/// Returns None if should skip, Some(tee_dir) if should proceed.
fn should_tee(
    config: &TeeConfig,
    raw_len: usize,
    exit_code: i32,
    tee_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if !config.enabled {
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

    if raw_len < MIN_TEE_SIZE {
        return None;
    }

    tee_dir
}

/// Write raw output to a tee file in the given directory.
/// Returns file path on success.
fn write_tee_file(
    raw: &str,
    command_slug: &str,
    tee_dir: &std::path::Path,
    max_file_size: usize,
    max_files: usize,
) -> Option<PathBuf> {
    std::fs::create_dir_all(tee_dir).ok()?;

    let slug = sanitize_slug(command_slug);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let filename = format!("{}_{}.log", epoch, slug);
    let filepath = tee_dir.join(filename);

    // Safe truncation at char boundary
    let end = if max_file_size >= raw.len() {
        raw.len()
    } else {
        let mut end = max_file_size;
        while end > 0 && !raw.is_char_boundary(end) {
            end -= 1;
        }
        end
    };

    let content = if end < raw.len() {
        format!(
            "{}\n\n--- truncated at {} bytes ---",
            &raw[..end],
            max_file_size
        )
    } else {
        raw.to_string()
    };

    std::fs::write(&filepath, content).ok()?;

    // Rotate old files
    cleanup_old_files(tee_dir, max_files);

    Some(filepath)
}

/// Write raw output to tee file if conditions are met.
/// Returns file path on success, None if skipped/failed.
pub fn tee_raw(raw: &str, command_slug: &str, exit_code: i32) -> Option<PathBuf> {
    // Check CRUNCH_TEE=0 env override (disable)
    if std::env::var("CRUNCH_TEE").ok().as_deref() == Some("0") {
        return None;
    }

    let config = Config::load().ok()?;
    let tee_dir = get_tee_dir(&config)?;

    let tee_dir = should_tee(&config.tee, raw.len(), exit_code, Some(tee_dir))?;

    write_tee_file(
        raw,
        command_slug,
        &tee_dir,
        config.tee.max_file_size,
        config.tee.max_files,
    )
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

/// Convenience: tee + format hint in one call.
/// Returns hint string if file was written, None if skipped.
pub fn tee_and_hint(raw: &str, command_slug: &str, exit_code: i32) -> Option<String> {
    let path = tee_raw(raw, command_slug, exit_code)?;
    Some(format_hint(&path))
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

/// Build log path: /tmp/crunch/{project}/{tool}-{scope}-{timestamp}.log
pub fn build_log_path(project: &str, tool: &str, scope: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!(
        "{}-{}-{}.log",
        sanitize_slug(tool),
        sanitize_slug(scope),
        timestamp
    );
    PathBuf::from("/tmp/crunch")
        .join(sanitize_slug(project))
        .join(filename)
}

/// Write raw output to project-scoped tee file.
pub fn tee_raw_scoped(raw: &str, tool: &str, args: &[String], exit_code: i32) -> Option<PathBuf> {
    if std::env::var("CRUNCH_TEE").ok().as_deref() == Some("0") {
        return None;
    }

    let config = Config::load().ok().map(|c| c.tee).unwrap_or_default();

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

    let project = detect_project_name();
    let scope = detect_scope(args);
    let log_path = build_log_path(&project, tool, &scope);

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    // Safe truncation at char boundary
    let max_file_size = config.max_file_size;
    let end = if max_file_size >= raw.len() {
        raw.len()
    } else {
        let mut end = max_file_size;
        while end > 0 && !raw.is_char_boundary(end) {
            end -= 1;
        }
        end
    };

    let content = if end < raw.len() {
        format!(
            "{}\n\n--- truncated at {} bytes ---",
            &raw[..end],
            max_file_size
        )
    } else {
        raw.to_string()
    };

    std::fs::write(&log_path, content).ok()?;

    if let Some(parent) = log_path.parent() {
        cleanup_old_files(parent, config.max_files);
    }

    Some(log_path)
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
    fn test_should_tee_disabled() {
        let config = TeeConfig {
            enabled: false,
            ..TeeConfig::default()
        };
        let dir = PathBuf::from("/tmp/tee");
        assert!(should_tee(&config, 1000, 1, Some(dir)).is_none());
    }

    #[test]
    fn test_should_tee_never_mode() {
        let config = TeeConfig {
            mode: TeeMode::Never,
            ..TeeConfig::default()
        };
        let dir = PathBuf::from("/tmp/tee");
        assert!(should_tee(&config, 1000, 1, Some(dir)).is_none());
    }

    #[test]
    fn test_should_tee_skip_small_output() {
        let config = TeeConfig::default();
        let dir = PathBuf::from("/tmp/tee");
        // Below MIN_TEE_SIZE (500)
        assert!(should_tee(&config, 100, 1, Some(dir)).is_none());
    }

    #[test]
    fn test_should_tee_skip_success_in_failures_mode() {
        let config = TeeConfig {
            mode: TeeMode::Failures,
            ..TeeConfig::default()
        };
        let dir = PathBuf::from("/tmp/tee");
        assert!(should_tee(&config, 1000, 0, Some(dir)).is_none());
    }

    #[test]
    fn test_should_tee_proceed_on_failure() {
        let config = TeeConfig {
            mode: TeeMode::Failures,
            ..TeeConfig::default()
        };
        let dir = PathBuf::from("/tmp/tee");
        assert!(should_tee(&config, 1000, 1, Some(dir)).is_some());
    }

    #[test]
    fn test_should_tee_always_mode_success() {
        let config = TeeConfig {
            mode: TeeMode::Always,
            ..TeeConfig::default()
        };
        let dir = PathBuf::from("/tmp/tee");
        assert!(should_tee(&config, 1000, 0, Some(dir)).is_some());
    }

    #[test]
    fn test_write_tee_file_creates_file() {
        let tmpdir = tempfile::tempdir().unwrap();
        let content = "error: test failed\n".repeat(50);
        let result = write_tee_file(
            &content,
            "cargo_test",
            tmpdir.path(),
            DEFAULT_MAX_FILE_SIZE,
            20,
        );
        assert!(result.is_some());

        let path = result.unwrap();
        assert!(path.exists());
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("error: test failed"));
    }

    #[test]
    fn test_write_tee_file_truncation() {
        let tmpdir = tempfile::tempdir().unwrap();
        let big_output = "x".repeat(2000);
        // Set max_file_size to 1000 bytes
        let result = write_tee_file(&big_output, "test", tmpdir.path(), 1000, 20);
        assert!(result.is_some());

        let path = result.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("--- truncated at 1000 bytes ---"));
        assert!(content.len() < 2000);
    }

    #[test]
    fn test_write_tee_file_truncation_multibyte_utf8() {
        let tmpdir = tempfile::tempdir().unwrap();
        // Each emoji is 4 bytes. 250 emojis = 1000 bytes exactly.
        // Truncating at 999 bytes would land inside an emoji without char-boundary fix.
        let emoji_output = "🦀".repeat(250); // 1000 bytes
        assert_eq!(emoji_output.len(), 1000);

        // Truncate at 999 — lands inside a 4-byte char
        let result = write_tee_file(&emoji_output, "test", tmpdir.path(), 999, 20);
        assert!(result.is_some());

        let path = result.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        // Must not panic, and content must be valid UTF-8 (fs::read_to_string proves this)
        assert!(content.contains("--- truncated at 999 bytes ---"));
        // The truncated content should end at a char boundary (996 = 249 * 4)
        assert!(content.starts_with("🦀"));
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
    fn test_format_hint() {
        let path = PathBuf::from("/tmp/rtk/tee/123_cargo_test.log");
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
directory = "/tmp/rtk-tee"
"#;
        let config: TeeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.mode, TeeMode::Always);
        assert_eq!(config.max_files, 10);
        assert_eq!(config.max_file_size, 524288);
        assert_eq!(config.directory, Some(PathBuf::from("/tmp/rtk-tee")));

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
    fn test_hint_format_no_tilde_for_tmp() {
        let path = PathBuf::from("/tmp/crunch/myproject/pytest-all-20260327-143012.log");
        let hint = format_hint(&path);
        assert!(hint.starts_with("[full output: /tmp/crunch/"));
    }
}
