use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Application configuration.
///
/// Loaded from `~/.config/giv/config.toml` if present; falls back to
/// `Default` silently so the app always starts.  Create the file to override
/// any of the settings below.
///
/// # Example `~/.config/giv/config.toml`
///
/// ```toml
/// # Active color theme.
/// # Available values: "tokyonight" (default), "catppuccin", "nord", "gruvbox"
/// theme = "tokyonight"
///
/// # Commit graph density.
/// # "compact"  (default) — 1 row per commit, tight spacing, more history at once.
/// # "spacious"           — 2 rows per commit (a blank edge row between commits).
/// graph_mode = "compact"
///
/// # Diff presentation style.
/// # "unified"    (default) — single-pane unified diff (like git diff).
/// # "side-by-side"         — two-pane view (future; falls back to unified).
/// diff_view = "unified"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Name of the active theme (e.g. `"tokyonight"`).
    #[serde(default = "default_theme")]
    pub theme: String,

    /// Graph render mode: `"compact"` (1-row per commit, default) or `"spacious"`.
    #[serde(default = "default_graph_mode")]
    pub graph_mode: String,

    /// Diff view style: `"unified"` (default) or `"side-by-side"`.
    #[serde(default = "default_diff_view")]
    pub diff_view: String,
}

fn default_theme() -> String {
    "tokyonight".into()
}

fn default_graph_mode() -> String {
    "compact".into()
}

fn default_diff_view() -> String {
    "unified".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            graph_mode: default_graph_mode(),
            diff_view: default_diff_view(),
        }
    }
}

impl Config {
    /// Whether the graph renders in spacious mode (a blank edge row between
    /// commits). Compact mode is 1 row per commit.
    pub fn graph_spacious(&self) -> bool {
        self.graph_mode != "compact"
    }

    /// Number of graph rows each commit occupies: 2 in spacious mode (a node
    /// row plus an edge row), 1 in compact mode.
    pub fn graph_row_step(&self) -> usize {
        if self.graph_spacious() {
            2
        } else {
            1
        }
    }
}

/// Load the configuration from disk.
///
/// Returns `Config::default()` if the file does not exist.
/// Returns an error only if the file is present but cannot be parsed.
pub fn load_config() -> anyhow::Result<Config> {
    use directories::ProjectDirs;

    let dirs = match ProjectDirs::from("", "", "giv") {
        Some(d) => d,
        None => return Ok(Config::default()),
    };

    let config_path = dirs.config_dir().join("config.toml");

    if !config_path.exists() {
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading config file {}", config_path.display()))?;

    let config: Config = toml::from_str(&raw)
        .with_context(|| format!("parsing config file {}", config_path.display()))?;

    Ok(config)
}

/// Persist the configuration to `~/.config/giv/config.toml`.
///
/// Creates the config directory if it does not exist. Used so runtime changes
/// (e.g. cycling the theme with `T`) survive across sessions instead of being
/// lost on exit.
pub fn save_config(config: &Config) -> anyhow::Result<()> {
    use directories::ProjectDirs;

    let dirs = ProjectDirs::from("", "", "giv")
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;

    let config_dir = dirs.config_dir();
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("creating config dir {}", config_dir.display()))?;

    let config_path = config_dir.join("config.toml");
    let serialized = toml::to_string_pretty(config).context("serializing config to TOML")?;
    std::fs::write(&config_path, serialized)
        .with_context(|| format!("writing config file {}", config_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let c = Config::default();
        assert_eq!(c.theme, "tokyonight");
        assert_eq!(c.graph_mode, "compact");
        assert_eq!(c.diff_view, "unified");
    }

    #[test]
    fn graph_spacious_false_for_compact() {
        let c = Config {
            graph_mode: "compact".into(),
            ..Config::default()
        };
        assert!(!c.graph_spacious());
        assert_eq!(c.graph_row_step(), 1);
    }

    #[test]
    fn graph_spacious_true_for_spacious() {
        let c = Config {
            graph_mode: "spacious".into(),
            ..Config::default()
        };
        assert!(c.graph_spacious());
        assert_eq!(c.graph_row_step(), 2);
    }

    #[test]
    fn graph_spacious_true_for_unknown_mode() {
        // Any value other than "compact" is treated as spacious.
        let c = Config {
            graph_mode: "unknown".into(),
            ..Config::default()
        };
        assert!(c.graph_spacious());
        assert_eq!(c.graph_row_step(), 2);
    }

    #[test]
    fn graph_spacious_true_for_empty_mode() {
        let c = Config {
            graph_mode: String::new(),
            ..Config::default()
        };
        assert!(c.graph_spacious());
        assert_eq!(c.graph_row_step(), 2);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let c = Config {
            theme: "nord".into(),
            graph_mode: "spacious".into(),
            diff_view: "side-by-side".into(),
        };
        let raw = toml::to_string(&c).expect("serialize");
        let parsed: Config = toml::from_str(&raw).expect("deserialize");
        assert_eq!(parsed.theme, "nord");
        assert_eq!(parsed.graph_mode, "spacious");
        assert_eq!(parsed.diff_view, "side-by-side");
    }

    #[test]
    fn config_uses_defaults_for_missing_fields() {
        // A TOML with only `theme` set should fall back to defaults for the rest.
        let raw = r#"theme = "gruvbox""#;
        let parsed: Config = toml::from_str(raw).expect("deserialize");
        assert_eq!(parsed.theme, "gruvbox");
        assert_eq!(parsed.graph_mode, "compact");
        assert_eq!(parsed.diff_view, "unified");
    }
}
