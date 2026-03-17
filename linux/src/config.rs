use std::path::PathBuf;

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Human-readable name shown to other devices.
    pub device_name: String,
    /// Where received files are saved.
    pub download_dir: PathBuf,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_file()?;
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&raw)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_file()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Returns the directory where all Tunnel app data lives.
    pub fn data_dir() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("dev", "tunnel", "tunnel")
            .ok_or_else(|| anyhow::anyhow!("Could not resolve config directory"))?;
        Ok(dirs.data_dir().to_path_buf())
    }

    fn config_file() -> Result<PathBuf> {
        Ok(Self::data_dir()?.join("config.json"))
    }
}

impl Default for Config {
    fn default() -> Self {
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let hostname = gethostname::gethostname()
            .to_string_lossy()
            .trim()
            .to_string();
        let hostname = if hostname.is_empty() { "computer".to_string() } else { hostname };

        let device_name = format!("{username} @ {hostname}");

        let download_dir = directories::UserDirs::new()
            .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            device_name,
            download_dir,
        }
    }
}
