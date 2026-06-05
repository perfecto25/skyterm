use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// User-editable settings persisted to TOML. Fields are all optional so old
/// configs from earlier versions keep loading; missing fields fall back to
/// the in-app defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Absolute path to the chosen monospace font. `None` means "let the
    /// app pick whatever it finds first on the system."
    #[serde(default)]
    pub font_path: Option<PathBuf>,
    /// Pixel size used for new panes; existing panes will resize on apply.
    #[serde(default)]
    pub font_size: Option<u32>,
    /// Name of a built-in theme (`Theme::presets()`) or "Custom" — currently
    /// only built-ins are supported in Settings.
    #[serde(default)]
    pub theme_name: Option<String>,
    /// Maximum scrollback line count per pane.
    #[serde(default)]
    pub scrollback_lines: Option<usize>,
    /// Whether the cursor blinks. `None` / absent means enabled.
    #[serde(default)]
    pub cursor_blink: Option<bool>,
    /// Whether double-click selects a word and triple-click selects the whole
    /// (wrapped) line. `None` / absent means enabled.
    #[serde(default)]
    pub click_word_select: Option<bool>,
    /// Whether making a selection (word, line, or drag) copies it to the system
    /// clipboard automatically. `None` / absent means disabled.
    #[serde(default)]
    pub copy_on_select: Option<bool>,
    /// Maximum number of tabs allowed. `None` / absent means 20.
    #[serde(default)]
    pub tab_max_number: Option<u32>,
    /// Prompt before closing a tab. `None` / absent means enabled.
    #[serde(default)]
    pub confirm_tab_close: Option<bool>,
    /// Prompt before closing a pane. `None` / absent means enabled.
    #[serde(default)]
    pub confirm_pane_close: Option<bool>,
    /// Prompt before closing the terminal window. `None` / absent means enabled.
    #[serde(default)]
    pub confirm_window_close: Option<bool>,
    /// Show the floating drag/close toolbar on split panes. `None` / absent means enabled.
    #[serde(default)]
    pub show_pane_toolbar: Option<bool>,
}

impl Config {
    /// Default config-file location. Honors `$XDG_CONFIG_HOME`; falls back to
    /// `$HOME/.config/skyterm/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("skyterm").join("config.toml"))
    }

    /// Load from `path`. Missing file → `Config::default()` (not an error).
    /// Parse errors are logged and also fall through to default.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        match toml::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("config parse failed at {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write to `path`. Creates the parent directory if needed.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}
