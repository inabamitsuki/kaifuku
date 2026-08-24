use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// When running under sudo the environment reports root's home ("/root"),
// but the user really wants their own home directory. Recover it from the
// SUDO_USER environment variable via /etc/passwd.
fn invoking_home_dir() -> PathBuf {
    if let Some(user) = std::env::var("SUDO_USER").ok().filter(|s| !s.is_empty()) {
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
            for line in passwd.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 6 && parts[0] == user && !parts[5].is_empty() {
                    return PathBuf::from(parts[5]);
                }
            }
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub default_recovery_folder: String,
    pub auto_save_log: bool,
    pub photorec_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_recovery_folder: invoking_home_dir()
                .join("recovery")
                .to_string_lossy()
                .to_string(),
            auto_save_log: true,
            photorec_path: None,
        }
    }
}

impl Config {
    /// Load configuration from file
    pub fn load(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let config: Config = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save configuration to file
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get default config path
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("kaifuku")
            .join("config.json")
    }
}
