use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Tool-to-task mapping for mise integration.
/// Keys are tool names (e.g., "pytest"), values are mise task names (e.g., "test").
pub type MiseConfig = HashMap<String, String>;

#[derive(Debug, Serialize, Deserialize, Default)]
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
    pub hooks: HooksConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub mise: MiseConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    /// Commands to exclude from auto-rewrite (e.g. ["curl", "playwright"]).
    /// Survives `rtk init -g` re-runs since config.toml is user-owned.
    #[serde(default)]
    pub exclude_commands: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackingConfig {
    pub enabled: bool,
    pub history_days: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_path: Option<PathBuf>,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history_days: 90,
            database_path: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub colors: bool,
    pub emoji: bool,
    pub max_width: usize,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            colors: true,
            emoji: true,
            max_width: 120,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FilterConfig {
    pub ignore_dirs: Vec<String>,
    pub ignore_files: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            ignore_dirs: vec![
                ".git".into(),
                "node_modules".into(),
                "target".into(),
                "__pycache__".into(),
                ".venv".into(),
                "vendor".into(),
            ],
            ignore_files: vec!["*.lock".into(), "*.min.js".into(), "*.min.css".into()],
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LimitsConfig {
    /// Max total grep results to show (default: 200)
    pub grep_max_results: usize,
    /// Max matches per file in grep output (default: 25)
    pub grep_max_per_file: usize,
    /// Max staged/modified files shown in git status (default: 15)
    pub status_max_files: usize,
    /// Max untracked files shown in git status (default: 10)
    pub status_max_untracked: usize,
    /// Max chars for parser passthrough fallback (default: 2000)
    pub passthrough_max_chars: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            grep_max_results: 200,
            grep_max_per_file: 25,
            status_max_files: 15,
            status_max_untracked: 10,
            passthrough_max_chars: 2000,
        }
    }
}

/// Get limits config. Falls back to defaults if config can't be loaded.
pub fn limits() -> LimitsConfig {
    Config::load().map(|c| c.limits).unwrap_or_default()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = get_config_path()?;

        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = get_config_path()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn create_default() -> Result<PathBuf> {
        let config = Config::default();
        config.save()?;
        get_config_path()
    }
}

fn get_config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    Ok(config_dir.join("rtk").join("config.toml"))
}

/// Merge a project-level config on top of a global config.
/// Project values override global values key-by-key within each section.
pub fn merge_configs(global: Config, project: Config) -> Config {
    let mut merged = global;

    // Merge mise: project overrides/extends global
    for (k, v) in project.mise {
        merged.mise.insert(k, v);
    }

    // Merge tee overrides if project specifies non-default values
    // (We check if the project config has a non-default tee by comparing to defaults)
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

pub fn show_config() -> Result<()> {
    let path = get_config_path()?;
    println!("Config: {}", path.display());
    println!();

    if path.exists() {
        let config = Config::load()?;
        println!("{}", toml::to_string_pretty(&config)?);
    } else {
        println!("(default config, file not created)");
        println!();
        let config = Config::default();
        println!("{}", toml::to_string_pretty(&config)?);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hooks_config_deserialize() {
        let toml = r#"
[hooks]
exclude_commands = ["curl", "gh"]
"#;
        let config: Config = toml::from_str(toml).expect("valid toml");
        assert_eq!(config.hooks.exclude_commands, vec!["curl", "gh"]);
    }

    #[test]
    fn test_hooks_config_default_empty() {
        let config = Config::default();
        assert!(config.hooks.exclude_commands.is_empty());
    }

    #[test]
    fn test_config_without_hooks_section_is_valid() {
        let toml = r#"
[tracking]
enabled = true
history_days = 90
"#;
        let config: Config = toml::from_str(toml).expect("valid toml");
        assert!(config.hooks.exclude_commands.is_empty());
    }

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
}
