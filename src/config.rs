use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub wallpaper_dirs: Vec<PathBuf>,
    pub exposed_threshold_percent: f64,
    pub obscured_threshold_percent: f64,
    pub cooldown_seconds: u64,
    pub debounce_millis: u64,
    pub history_count: usize,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wallpaper_dirs: vec![dirs_wallpaper()],
            exposed_threshold_percent: 15.0,
            obscured_threshold_percent: 5.0,
            cooldown_seconds: 300,
            debounce_millis: 400,
            history_count: 10,
            log_level: default_log_level(),
        }
    }
}

impl AppConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {}", path.display(), e))?;
        let mut cfg: AppConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config JSON: {}", e))?;
        // Seamlessly migrate legacy defaults (20.0% / 10.0%) to new calibrated thresholds (15.0% / 5.0%)
        if (cfg.exposed_threshold_percent - 20.0).abs() < f64::EPSILON
            && (cfg.obscured_threshold_percent - 10.0).abs() < f64::EPSILON
        {
            cfg.exposed_threshold_percent = 15.0;
            cfg.obscured_threshold_percent = 5.0;
        }
        Ok(cfg)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir {}: {}", parent.display(), e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(path, json)
            .map_err(|e| format!("Failed to write config file {}: {}", path.display(), e))
    }
}

fn dirs_wallpaper() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures").join("Wallpapers"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_json_roundtrip() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            cfg.exposed_threshold_percent,
            parsed.exposed_threshold_percent
        );
        assert_eq!(cfg.history_count, parsed.history_count);
    }
}
