//! User-tunable settings (theme, volume, visualizer) plus the resumable
//! session state (last directory, playlist, playback position, repeat/shuffle)
//! and their pure (de)serialization. Where the bytes are stored is
//! platform-specific and handled by the [`crate::backend::ConfigStore`] trait:
//! the native crate writes a file under the OS config dir, the web crate uses
//! `localStorage`.

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub theme: String,
    pub volume: f32,
    pub visualizer: String,

    /// Last browser directory (native). Empty/None on first run or web.
    pub last_dir: Option<String>,
    /// Repeat mode as a lowercase string: "off" | "all" | "one".
    pub repeat: String,
    pub shuffle: bool,
    /// The playlist as file paths, in order (native; object-URL playlists on
    /// web are ephemeral and not persisted).
    pub playlist: Vec<String>,
    /// Index into `playlist` of the track that was playing.
    pub playing_index: Option<usize>,
    /// Playback position of that track, in seconds.
    pub position_secs: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            volume: 0.8,
            visualizer: "spectrum".to_string(),
            last_dir: None,
            repeat: "off".to_string(),
            shuffle: false,
            playlist: Vec::new(),
            playing_index: None,
            position_secs: 0.0,
        }
    }
}

impl Config {
    /// Parse the simple `key = value` config format. Unknown keys are ignored
    /// and missing keys keep their default. The `track = <path>` key may repeat,
    /// once per playlist entry, in order.
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
                "last_dir" if !val.is_empty() => cfg.last_dir = Some(val.to_string()),
                "repeat" => {
                    let v = val.to_ascii_lowercase();
                    if matches!(v.as_str(), "off" | "all" | "one") {
                        cfg.repeat = v;
                    }
                }
                "shuffle" => cfg.shuffle = matches!(val, "true" | "1" | "on"),
                "playing_index" => cfg.playing_index = val.parse::<usize>().ok(),
                "position_secs" => {
                    if let Ok(v) = val.parse::<f64>() {
                        cfg.position_secs = v.max(0.0);
                    }
                }
                "track" if !val.is_empty() => cfg.playlist.push(val.to_string()),
                _ => {}
            }
        }
        cfg
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# SparkPlayer configuration — managed by the app, edit while it's closed.\n",
        );
        out.push_str(&format!("theme = \"{}\"\n", self.theme));
        out.push_str(&format!("volume = {}\n", self.volume));
        out.push_str(&format!("visualizer = \"{}\"\n", self.visualizer));
        out.push_str(&format!("repeat = \"{}\"\n", self.repeat));
        out.push_str(&format!("shuffle = {}\n", self.shuffle));
        if let Some(dir) = &self.last_dir {
            out.push_str(&format!("last_dir = \"{}\"\n", dir));
        }
        if let Some(i) = self.playing_index {
            out.push_str(&format!("playing_index = {}\n", i));
        }
        out.push_str(&format!("position_secs = {}\n", self.position_secs));
        for track in &self.playlist {
            out.push_str(&format!("track = \"{}\"\n", track));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_session_state() {
        let cfg = Config {
            theme: "dracula".to_string(),
            volume: 0.65,
            visualizer: "waveform".to_string(),
            last_dir: Some("/home/me/Music".to_string()),
            repeat: "all".to_string(),
            shuffle: true,
            playlist: vec![
                "/home/me/Music/a.flac".to_string(),
                "/home/me/Music/b.mp3".to_string(),
            ],
            playing_index: Some(1),
            position_secs: 42.5,
        };
        let parsed = Config::parse(&cfg.serialize());
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn defaults_for_missing_keys() {
        let cfg = Config::parse("theme = \"nord\"\n");
        assert_eq!(cfg.theme, "nord");
        assert_eq!(cfg.repeat, "off");
        assert!(!cfg.shuffle);
        assert!(cfg.playlist.is_empty());
        assert_eq!(cfg.playing_index, None);
    }

    #[test]
    fn ignores_invalid_repeat_and_clamps_volume() {
        let cfg = Config::parse("repeat = bogus\nvolume = 9.0\n");
        assert_eq!(cfg.repeat, "off");
        assert_eq!(cfg.volume, 1.5);
    }
}
