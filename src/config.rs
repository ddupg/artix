use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UiMode {
    #[default]
    Auto,
    Plain,
    Tui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TrashBackend {
    #[default]
    Auto,
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub mode: UiMode,
    pub icons: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PerformanceConfig {
    pub fs_concurrency: usize,
    pub git_concurrency: usize,
    pub tui_entry_concurrency: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeleteConfig {
    pub trash_backend: TrashBackend,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub ui: UiConfig,
    pub performance: PerformanceConfig,
    pub delete: DeleteConfig,
}

#[derive(Debug, Clone)]
pub struct AppContext {
    config: Arc<Config>,
    git_semaphore: Arc<Semaphore>,
    fs_semaphore: Arc<Semaphore>,
}

#[derive(Debug, Clone)]
pub struct ConfigLoadReport {
    pub config: Config,
    pub source_path: Option<PathBuf>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigPathKind {
    Primary,
    CompatDotfile,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            ui: UiConfig::default(),
            performance: PerformanceConfig::default(),
            delete: DeleteConfig::default(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            mode: UiMode::Auto,
            icons: true,
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            fs_concurrency: default_fs_concurrency(),
            git_concurrency: default_git_concurrency(),
            tui_entry_concurrency: default_tui_entry_concurrency(),
        }
    }
}

impl Default for DeleteConfig {
    fn default() -> Self {
        Self {
            trash_backend: TrashBackend::Auto,
        }
    }
}

impl Default for AppContext {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

impl Config {
    pub fn from_toml_str(contents: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(contents).map_err(|err| err.to_string())?;
        config.validate()
    }

    fn validate(self) -> Result<Self, String> {
        if self.version != CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {}; expected {CONFIG_VERSION}",
                self.version
            ));
        }
        validate_positive_usize(
            "performance.fs_concurrency",
            self.performance.fs_concurrency,
        )?;
        validate_positive_usize(
            "performance.git_concurrency",
            self.performance.git_concurrency,
        )?;
        validate_positive_usize(
            "performance.tui_entry_concurrency",
            self.performance.tui_entry_concurrency,
        )?;

        Ok(self)
    }
}

pub fn default_config_path() -> Result<PathBuf, String> {
    home_dir()
        .map(|home| home.join(".config").join("artix").join(CONFIG_FILE_NAME))
        .ok_or_else(|| "could not determine the home directory for config path".to_string())
}

pub fn render_default_config_toml() -> String {
    toml::to_string_pretty(&Config::default()).expect("default config must serialize")
}

pub fn init_default_config_file() -> Result<PathBuf, String> {
    let target_path = default_config_path()?;
    let existing_path = discover_existing_config_path().map(|(path, _)| path);
    init_default_config_file_at(target_path, existing_path)
}

impl AppContext {
    pub fn new(config: Config) -> Self {
        Self {
            git_semaphore: Arc::new(Semaphore::new(config.performance.git_concurrency)),
            fs_semaphore: Arc::new(Semaphore::new(config.performance.fs_concurrency)),
            config: Arc::new(config),
        }
    }

    pub fn config(&self) -> &Config {
        self.config.as_ref()
    }

    pub fn git_semaphore(&self) -> Arc<Semaphore> {
        self.git_semaphore.clone()
    }

    pub fn fs_semaphore(&self) -> Arc<Semaphore> {
        self.fs_semaphore.clone()
    }
}

pub fn load_config() -> Result<ConfigLoadReport, String> {
    let mut warnings = Vec::new();
    let existing = discover_existing_config_path();

    let config = match &existing {
        Some((path, kind)) => {
            let contents = fs::read_to_string(path)
                .map_err(|err| format!("failed to read config file {}: {err}", path.display()))?;
            if !matches!(kind, ConfigPathKind::Primary) {
                warnings.push(format!(
                    "config loaded from compatibility path {}; prefer {}",
                    path.display(),
                    default_config_path()
                        .map(|value| value.display().to_string())
                        .unwrap_or_else(|_| "~/.config/artix/config.toml".to_string())
                ));
            }
            let config = toml::from_str::<Config>(&contents)
                .map_err(|err| format!("failed to parse config file {}: {err}", path.display()))?;
            config.validate()?
        }
        None => Config::default(),
    };

    Ok(ConfigLoadReport {
        config,
        source_path: existing.map(|(path, _)| path),
        warnings,
    })
}

fn validate_positive_usize(field: &str, value: usize) -> Result<(), String> {
    if value == 0 {
        Err(format!("{field} must be greater than 0"))
    } else {
        Ok(())
    }
}

fn default_fs_concurrency() -> usize {
    default_parallelism().saturating_mul(2).clamp(2, 16)
}

fn default_git_concurrency() -> usize {
    default_parallelism().clamp(2, 8)
}

fn default_tui_entry_concurrency() -> usize {
    default_parallelism().saturating_mul(2).clamp(4, 32)
}

fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
}

fn discover_existing_config_path() -> Option<(PathBuf, ConfigPathKind)> {
    candidate_config_paths()
        .into_iter()
        .find(|(path, _)| path.is_file())
}

fn candidate_config_paths() -> Vec<(PathBuf, ConfigPathKind)> {
    candidate_config_paths_for(home_dir())
}

fn candidate_config_paths_for(home_dir: Option<PathBuf>) -> Vec<(PathBuf, ConfigPathKind)> {
    let mut seen = HashSet::<PathBuf>::new();
    let mut paths = Vec::new();

    if let Some(home_dir) = home_dir {
        push_config_path(
            &mut paths,
            &mut seen,
            home_dir
                .join(".config")
                .join("artix")
                .join(CONFIG_FILE_NAME),
            ConfigPathKind::Primary,
        );
        push_config_path(
            &mut paths,
            &mut seen,
            home_dir.join(".artix").join(CONFIG_FILE_NAME),
            ConfigPathKind::CompatDotfile,
        );
    }

    paths
}

fn push_config_path(
    paths: &mut Vec<(PathBuf, ConfigPathKind)>,
    seen: &mut HashSet<PathBuf>,
    path: PathBuf,
    kind: ConfigPathKind,
) {
    if seen.insert(path.clone()) {
        paths.push((path, kind));
    }
}

fn init_default_config_file_at(
    target_path: PathBuf,
    existing_path: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = existing_path {
        return Err(format!("config file already exists at {}", path.display()));
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create config directory {}: {err}",
                parent.display()
            )
        })?;
    }

    fs::write(&target_path, render_default_config_toml()).map_err(|err| {
        format!(
            "failed to write config file {}: {err}",
            target_path.display()
        )
    })?;

    Ok(target_path)
}

fn home_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        AppContext, CONFIG_FILE_NAME, Config, ConfigPathKind, TrashBackend, UiMode,
        candidate_config_paths_for, init_default_config_file_at, render_default_config_toml,
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn config_from_toml_parses_expected_fields() {
        let config = Config::from_toml_str(
            r#"
version = 1

[ui]
mode = "plain"
icons = false

[performance]
fs_concurrency = 9
git_concurrency = 3
tui_entry_concurrency = 7

[delete]
trash_backend = "builtin"
"#,
        )
        .unwrap();

        assert_eq!(config.ui.mode, UiMode::Plain);
        assert!(!config.ui.icons);
        assert_eq!(config.performance.fs_concurrency, 9);
        assert_eq!(config.performance.git_concurrency, 3);
        assert_eq!(config.performance.tui_entry_concurrency, 7);
        assert_eq!(config.delete.trash_backend, TrashBackend::Builtin);
    }

    #[test]
    fn partial_nested_sections_inherit_runtime_defaults() {
        let defaults = Config::default();
        let config = Config::from_toml_str(
            r#"
[ui]
mode = "plain"

[performance]
git_concurrency = 3
"#,
        )
        .unwrap();

        assert_eq!(config.ui.mode, UiMode::Plain);
        assert_eq!(config.ui.icons, defaults.ui.icons);
        assert_eq!(
            config.performance.fs_concurrency,
            defaults.performance.fs_concurrency
        );
        assert_eq!(config.performance.git_concurrency, 3);
        assert_eq!(
            config.performance.tui_entry_concurrency,
            defaults.performance.tui_entry_concurrency
        );
        assert_eq!(config.delete, defaults.delete);
    }

    #[test]
    fn config_from_toml_rejects_removed_tui_size_budget() {
        let err = Config::from_toml_str(
            r#"
[scan.tui_size_budget]
max_entries = 10
timeout_ms = 50
"#,
        )
        .unwrap_err();

        assert!(err.contains("unknown field"));
    }

    #[test]
    fn config_from_toml_rejects_unsupported_version() {
        let err = Config::from_toml_str("version = 2\n").unwrap_err();

        assert_eq!(err, "unsupported config version 2; expected 1");
    }

    #[test]
    fn config_from_toml_rejects_zero_concurrency() {
        for field in ["fs_concurrency", "git_concurrency", "tui_entry_concurrency"] {
            let contents = format!(
                r#"
[performance]
{field} = 0
"#
            );
            let err = Config::from_toml_str(&contents).unwrap_err();

            assert_eq!(err, format!("performance.{field} must be greater than 0"));
        }
    }

    #[test]
    fn render_default_config_round_trips_through_parser() {
        let rendered = render_default_config_toml();
        let parsed = Config::from_toml_str(&rendered).unwrap();
        let rendered_value: toml::Value = toml::from_str(&rendered).unwrap();
        let rendered_table = rendered_value.as_table().expect("top-level table");

        assert_eq!(parsed, Config::default());
        assert_eq!(rendered_table.len(), 4);
        for field in ["version", "ui", "performance", "delete"] {
            assert!(rendered_table.contains_key(field));
        }
    }

    #[test]
    fn candidate_config_paths_prefer_xdg_then_dotfile_paths() {
        let home = Some(PathBuf::from("/Users/tester"));

        let paths = candidate_config_paths_for(home);

        assert_eq!(
            paths,
            vec![
                (
                    PathBuf::from(format!("/Users/tester/.config/artix/{CONFIG_FILE_NAME}")),
                    ConfigPathKind::Primary,
                ),
                (
                    PathBuf::from(format!("/Users/tester/.artix/{CONFIG_FILE_NAME}")),
                    ConfigPathKind::CompatDotfile,
                ),
            ]
        );
    }

    #[test]
    fn app_context_uses_configured_semaphore_sizes() {
        let mut config = Config::default();
        config.performance.fs_concurrency = 6;
        config.performance.git_concurrency = 5;

        let ctx = AppContext::new(config);

        assert_eq!(ctx.fs_semaphore().available_permits(), 6);
        assert_eq!(ctx.git_semaphore().available_permits(), 5);
    }

    #[test]
    fn init_default_config_writes_rendered_contents() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("artix/config.toml");

        let written = init_default_config_file_at(target.clone(), None).unwrap();
        let contents = fs::read_to_string(&target).unwrap();

        assert_eq!(written, target);
        assert_eq!(contents, render_default_config_toml());
    }

    #[test]
    fn init_default_config_rejects_existing_config_path() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("artix/config.toml");
        let existing = dir.path().join(".artix/config.toml");

        let err = init_default_config_file_at(target, Some(existing.clone())).unwrap_err();

        assert_eq!(
            err,
            format!("config file already exists at {}", existing.display())
        );
    }
}
