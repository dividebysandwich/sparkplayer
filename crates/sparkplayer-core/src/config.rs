use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub theme: String,
    pub volume: f32,
    pub visualizer: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            volume: 0.8,
            visualizer: "spectrum".to_string(),
        }
    }
}

/// Returns the cross-platform config file path: `$XDG_CONFIG_HOME/sparkplayer/config.toml`
/// on Linux, `~/Library/Application Support/sparkplayer/config.toml` on macOS,
/// `%APPDATA%\sparkplayer\config.toml` on Windows.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("sparkplayer").join("config.toml"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return Config::default();
    };
    let mut cfg = Config::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim().trim_matches('"');
        match key {
            "theme" => cfg.theme = val.to_string(),
            "volume" => {
                if let Ok(v) = val.parse::<f32>() {
                    cfg.volume = v.clamp(0.0, 1.5);
                }
            }
            "visualizer" => cfg.visualizer = val.to_string(),
            _ => {}
        }
    }
    cfg
}

pub fn save(cfg: &Config) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let content = format!(
        "# SparkPlayer configuration — managed by the app, edit while it's closed.\n\
         theme = \"{}\"\n\
         volume = {}\n\
         visualizer = \"{}\"\n",
        cfg.theme, cfg.volume, cfg.visualizer
    );
    let _ = fs::write(&path, content);
}
