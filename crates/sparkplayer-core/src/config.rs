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
    /// FFT window size for the spectrum visualizers (power of two).
    pub fft_size: usize,
    /// Scroll speed (rows/columns per second) for the scrolling FFT views.
    pub scroll_speed: u32,

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

    /// Favorited track paths (order is not significant).
    pub favorites: Vec<String>,
    /// Recently-played track paths, most-recent first, capped at [`RECENT_CAP`].
    pub recent: Vec<String>,
    /// Per-track play counts as `(path, count)` pairs.
    pub play_counts: Vec<(String, u32)>,
}

/// How many entries the recently-played list keeps.
pub const RECENT_CAP: usize = 50;

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            volume: 0.8,
            visualizer: "spectrum".to_string(),
            fft_size: crate::visualizer::FFT_DEFAULT_SIZE,
            scroll_speed: crate::visualizer::SCROLL_SPEED_DEFAULT,
            last_dir: None,
            repeat: "off".to_string(),
            shuffle: false,
            playlist: Vec::new(),
            playing_index: None,
            position_secs: 0.0,
            favorites: Vec::new(),
            recent: Vec::new(),
            play_counts: Vec::new(),
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
                "fft_size" => {
                    if let Ok(v) = val.parse::<usize>() {
                        cfg.fft_size = crate::visualizer::clamp_fft_size(v);
                    }
                }
                "scroll_speed" => {
                    if let Ok(v) = val.parse::<u32>() {
                        cfg.scroll_speed = crate::visualizer::clamp_scroll_speed(v);
                    }
                }
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
                "favorite" if !val.is_empty() => cfg.favorites.push(val.to_string()),
                "recent" if !val.is_empty() => {
                    if cfg.recent.len() < RECENT_CAP {
                        cfg.recent.push(val.to_string());
                    }
                }
                // Format: `playcount = "<count>|<path>"`.
                "playcount" => {
                    if let Some((count, path)) = val.split_once('|') {
                        if let Ok(n) = count.trim().parse::<u32>() {
                            let path = path.trim();
                            if !path.is_empty() {
                                cfg.play_counts.push((path.to_string(), n));
                            }
                        }
                    }
                }
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
        out.push_str(&format!("fft_size = {}\n", self.fft_size));
        out.push_str(&format!("scroll_speed = {}\n", self.scroll_speed));
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
        for fav in &self.favorites {
            out.push_str(&format!("favorite = \"{}\"\n", fav));
        }
        for path in self.recent.iter().take(RECENT_CAP) {
            out.push_str(&format!("recent = \"{}\"\n", path));
        }
        for (path, count) in &self.play_counts {
            out.push_str(&format!("playcount = \"{}|{}\"\n", count, path));
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
            fft_size: 4096,
            scroll_speed: 45,
            last_dir: Some("/home/me/Music".to_string()),
            repeat: "all".to_string(),
            shuffle: true,
            playlist: vec![
                "/home/me/Music/a.flac".to_string(),
                "/home/me/Music/b.mp3".to_string(),
            ],
            playing_index: Some(1),
            position_secs: 42.5,
            favorites: vec!["/home/me/Music/a.flac".to_string()],
            recent: vec![
                "/home/me/Music/b.mp3".to_string(),
                "/home/me/Music/a.flac".to_string(),
            ],
            play_counts: vec![
                ("/home/me/Music/a.flac".to_string(), 7),
                ("/home/me/Music/b.mp3".to_string(), 3),
            ],
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
