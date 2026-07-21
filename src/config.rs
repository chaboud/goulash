use serde::Deserialize;
use std::path::PathBuf;

/// Loaded from `~/.goulash/config.toml`; every field has a working default
/// so goulash runs with no config file at all.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub status: StatusConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StatusConfig {
    pub enabled: bool,
    pub rows: u16,
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rows: 1,
        }
    }
}

impl Config {
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("GOULASH_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".goulash")))
    }

    pub fn load() -> Config {
        let Some(path) = Self::dir().map(|d| d.join("config.toml")) else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("goulash: ignoring bad config {}: {err}", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Rows kept out of the inner PTY's world (status row; later + heckle band).
    pub fn reserved_rows(&self) -> u16 {
        if self.status.enabled {
            self.status.rows.clamp(1, 8)
        } else {
            0
        }
    }
}
