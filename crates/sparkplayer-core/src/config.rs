//! User-tunable settings (theme, volume, visualizer) and their pure
//! (de)serialization. Where the bytes are stored is platform-specific and
//! handled by the [`crate::backend::ConfigStore`] trait: the native crate
//! writes a file under the OS config dir, the web crate uses `localStorage`.

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

impl Config {
    /// Parse the simple `key = value` config format. Unknown keys are ignored
    /// and missing keys keep their default.
    pub fn parse(content: &str) -> Config {
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

    pub fn serialize(&self) -> String {
        format!(
            "# SparkPlayer configuration — managed by the app, edit while it's closed.\n\
             theme = \"{}\"\n\
             volume = {}\n\
             visualizer = \"{}\"\n",
            self.theme, self.volume, self.visualizer
        )
    }
}
