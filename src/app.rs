use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ratatui::widgets::ListState;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;

use crate::audio::AudioPlayer;
use crate::library::{self, Track};
use crate::metadata::{self, TrackMeta};
use crate::visualizer::Visualizer;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FocusPane {
    Playlist,
    Browser,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum GraphicsChoice {
    Auto,
    Halfblocks,
    Sixel,
    Kitty,
    Iterm,
}

impl GraphicsChoice {
    fn into_protocol(self) -> Option<ProtocolType> {
        match self {
            GraphicsChoice::Auto => None,
            GraphicsChoice::Halfblocks => Some(ProtocolType::Halfblocks),
            GraphicsChoice::Sixel => Some(ProtocolType::Sixel),
            GraphicsChoice::Kitty => Some(ProtocolType::Kitty),
            GraphicsChoice::Iterm => Some(ProtocolType::Iterm2),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            RepeatMode::Off => "Off",
            RepeatMode::All => "All",
            RepeatMode::One => "One",
        }
    }
}

pub struct App {
    pub player: AudioPlayer,
    pub visualizer: Visualizer,

    pub tracks: Vec<Track>,
    pub selected: usize,
    pub playing_index: Option<usize>,
    pub playlist_state: ListState,

    pub browser_dir: PathBuf,
    pub browser_entries: Vec<PathBuf>,
    pub browser_selected: usize,
    pub browser_state: ListState,

    pub focus: FocusPane,

    pub current_meta: TrackMeta,
    pub current_duration: Option<Duration>,
    pub status: String,

    pub repeat: RepeatMode,
    pub shuffle: bool,

    pub picker: Option<Picker>,
    pub album_protocol: Option<StatefulProtocol>,
    pub album_dims: Option<(u32, u32)>,
    pub last_artwork_key: Option<(usize, usize)>,

    pub should_quit: bool,
    pub show_help: bool,

    graphics_choice: GraphicsChoice,
}

impl App {
    pub fn new(
        initial_tracks: Vec<Track>,
        initial_dir: PathBuf,
        graphics_choice: GraphicsChoice,
    ) -> Result<Self> {
        let player = AudioPlayer::new()?;
        let visualizer = Visualizer::new();
        let mut playlist_state = ListState::default();
        if !initial_tracks.is_empty() {
            playlist_state.select(Some(0));
        }
        let mut app = App {
            player,
            visualizer,
            tracks: initial_tracks,
            selected: 0,
            playing_index: None,
            playlist_state,
            browser_dir: initial_dir.clone(),
            browser_entries: Vec::new(),
            browser_selected: 0,
            browser_state: ListState::default(),
            focus: FocusPane::Playlist,
            current_meta: TrackMeta::default(),
            current_duration: None,
            status: String::from("Ready"),
            repeat: RepeatMode::Off,
            shuffle: false,
            picker: None,
            album_protocol: None,
            album_dims: None,
            last_artwork_key: None,
            should_quit: false,
            show_help: false,
            graphics_choice,
        };
        app.refresh_browser();
        Ok(app)
    }

    /// Probe the terminal for graphics capabilities. Must be called after raw
    /// mode is enabled. Falls back to a fixed font size (halfblock rendering)
    /// when the terminal doesn't respond to graphics queries.
    pub fn init_graphics(&mut self) {
        let mut picker = Picker::from_query_stdio()
            .ok()
            .unwrap_or_else(|| Picker::from_fontsize((8, 16)));
        if let Some(forced) = self.graphics_choice.into_protocol() {
            picker.set_protocol_type(forced);
        }
        self.picker = Some(picker);
        if self.current_meta.artwork.is_some() {
            self.last_artwork_key = None;
            self.refresh_album_art();
        }
    }

    pub fn refresh_browser(&mut self) {
        let mut entries: Vec<PathBuf> = Vec::new();
        if let Some(parent) = self.browser_dir.parent() {
            entries.push(parent.to_path_buf());
        }
        if let Ok(read) = std::fs::read_dir(&self.browser_dir) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();
            for e in read.flatten() {
                let p = e.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                }
                if p.is_dir() {
                    dirs.push(p);
                } else if library::is_audio_file(&p) || library::is_playlist_file(&p) {
                    files.push(p);
                }
            }
            dirs.sort();
            files.sort();
            entries.extend(dirs);
            entries.extend(files);
        }
        self.browser_entries = entries;
        if self.browser_selected >= self.browser_entries.len() {
            self.browser_selected = self.browser_entries.len().saturating_sub(1);
        }
        if self.browser_entries.is_empty() {
            self.browser_state.select(None);
        } else {
            self.browser_state.select(Some(self.browser_selected));
        }
    }

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            FocusPane::Playlist => FocusPane::Browser,
            FocusPane::Browser => FocusPane::Playlist,
        };
    }

    pub fn move_selection(&mut self, delta: i32) {
        match self.focus {
            FocusPane::Playlist => {
                if self.tracks.is_empty() {
                    return;
                }
                let len = self.tracks.len() as i32;
                let new = (self.selected as i32 + delta).rem_euclid(len);
                self.selected = new as usize;
                self.playlist_state.select(Some(self.selected));
            }
            FocusPane::Browser => {
                if self.browser_entries.is_empty() {
                    return;
                }
                let len = self.browser_entries.len() as i32;
                let new = (self.browser_selected as i32 + delta).rem_euclid(len);
                self.browser_selected = new as usize;
                self.browser_state.select(Some(self.browser_selected));
            }
        }
    }

    pub fn page(&mut self, dir: i32) {
        self.move_selection(dir * 10);
    }

    pub fn select_first(&mut self) {
        match self.focus {
            FocusPane::Playlist if !self.tracks.is_empty() => {
                self.selected = 0;
                self.playlist_state.select(Some(0));
            }
            FocusPane::Browser if !self.browser_entries.is_empty() => {
                self.browser_selected = 0;
                self.browser_state.select(Some(0));
            }
            _ => {}
        }
    }

    pub fn select_last(&mut self) {
        match self.focus {
            FocusPane::Playlist if !self.tracks.is_empty() => {
                let i = self.tracks.len() - 1;
                self.selected = i;
                self.playlist_state.select(Some(i));
            }
            FocusPane::Browser if !self.browser_entries.is_empty() => {
                let i = self.browser_entries.len() - 1;
                self.browser_selected = i;
                self.browser_state.select(Some(i));
            }
            _ => {}
        }
    }

    pub fn activate_selection(&mut self) -> Result<()> {
        match self.focus {
            FocusPane::Playlist => {
                if !self.tracks.is_empty() {
                    self.play_index(self.selected)?;
                }
            }
            FocusPane::Browser => {
                if let Some(path) = self.browser_entries.get(self.browser_selected).cloned() {
                    if path.is_dir() {
                        self.browser_dir = path;
                        self.browser_selected = 0;
                        self.refresh_browser();
                    } else if library::is_playlist_file(&path) {
                        match library::load_playlist(&path) {
                            Ok(tracks) => {
                                self.tracks = tracks;
                                self.selected = 0;
                                self.playlist_state.select(if self.tracks.is_empty() {
                                    None
                                } else {
                                    Some(0)
                                });
                                self.focus = FocusPane::Playlist;
                                self.status = format!(
                                    "Loaded playlist: {}",
                                    path.file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default()
                                );
                                if !self.tracks.is_empty() {
                                    self.play_index(0)?;
                                }
                            }
                            Err(e) => self.status = format!("Playlist error: {e}"),
                        }
                    } else if library::is_audio_file(&path) {
                        self.tracks.push(Track::from_path(path));
                        let idx = self.tracks.len() - 1;
                        self.selected = idx;
                        self.playlist_state.select(Some(idx));
                        self.focus = FocusPane::Playlist;
                        self.play_index(idx)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn play_index(&mut self, idx: usize) -> Result<()> {
        if idx >= self.tracks.len() {
            return Ok(());
        }
        let path = self.tracks[idx].path.clone();
        self.current_meta = metadata::read_metadata(&path).unwrap_or_default();
        if self.current_meta.artwork.is_none() {
            if let Some(bytes) = library::find_local_cover(&path) {
                self.current_meta.artwork = Some(bytes);
            }
        }
        match self.player.play_file(&path) {
            Ok(dur_hint) => {
                self.current_duration = self.current_meta.duration.or(dur_hint);
                self.playing_index = Some(idx);
                self.selected = idx;
                self.playlist_state.select(Some(idx));
                self.status = format!(
                    "Playing: {}",
                    self.tracks[idx]
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                );
                self.refresh_album_art();
            }
            Err(e) => {
                self.status = format!("Decode error: {e}");
                self.playing_index = None;
                self.current_duration = None;
            }
        }
        Ok(())
    }

    pub fn next_track(&mut self) -> Result<()> {
        if self.tracks.is_empty() {
            return Ok(());
        }
        let next = match self.playing_index {
            Some(i) if i + 1 < self.tracks.len() => i + 1,
            Some(_) if matches!(self.repeat, RepeatMode::All) => 0,
            Some(_) => return Ok(()),
            None => 0,
        };
        self.play_index(next)
    }

    pub fn prev_track(&mut self) -> Result<()> {
        if self.tracks.is_empty() {
            return Ok(());
        }
        let prev = match self.playing_index {
            Some(0) if matches!(self.repeat, RepeatMode::All) => self.tracks.len() - 1,
            Some(0) => return Ok(()),
            Some(i) => i - 1,
            None => 0,
        };
        self.play_index(prev)
    }

    pub fn check_advance(&mut self) -> Result<()> {
        if self.playing_index.is_some() && self.player.is_finished() {
            match self.repeat {
                RepeatMode::One => {
                    if let Some(i) = self.playing_index {
                        self.play_index(i)?;
                    }
                }
                _ => self.next_track()?,
            }
        }
        Ok(())
    }

    pub fn seek_seconds(&mut self, delta: f64) {
        if self.playing_index.is_none() || self.player.current_path.is_none() {
            return;
        }
        let total = self.current_duration;
        match self.player.seek_relative(delta, total) {
            Ok(()) => {
                let pos = self.player.tap.position();
                self.status = format!(
                    "Seek: {} ({:+.0}s)",
                    fmt_short(pos),
                    delta
                );
            }
            Err(e) => self.status = format!("Seek error: {e}"),
        }
    }

    pub fn queue_selected_browser(&mut self) {
        let Some(path) = self.browser_entries.get(self.browser_selected).cloned() else {
            return;
        };
        if path.is_dir() {
            let added = self.queue_directory(&path);
            self.status = if added > 0 {
                format!(
                    "Queued {added} track{} from {}",
                    plural_s(added),
                    short_name(&path)
                )
            } else {
                String::from("No audio files in directory")
            };
        } else if library::is_playlist_file(&path) {
            match library::load_playlist(&path) {
                Ok(more) => {
                    let n = more.len();
                    self.tracks.extend(more);
                    self.refresh_playlist_state();
                    self.status =
                        format!("Queued playlist ({n} track{})", plural_s(n));
                }
                Err(e) => self.status = format!("Playlist error: {e}"),
            }
        } else if library::is_audio_file(&path) {
            let name = short_name(&path);
            self.tracks.push(Track::from_path(path));
            self.refresh_playlist_state();
            self.status = format!("Queued: {name}");
        }
    }

    pub fn queue_browser_directory(&mut self) {
        let dir = self.browser_dir.clone();
        let added = self.queue_directory(&dir);
        self.status = if added > 0 {
            format!(
                "Queued {added} track{} from {}",
                plural_s(added),
                dir.display()
            )
        } else {
            String::from("No audio files in directory")
        };
    }

    fn queue_directory(&mut self, dir: &Path) -> usize {
        let new_tracks = library::scan_directory(dir);
        let n = new_tracks.len();
        if n > 0 {
            self.tracks.extend(new_tracks);
            self.refresh_playlist_state();
        }
        n
    }

    fn refresh_playlist_state(&mut self) {
        if self.tracks.is_empty() {
            self.playlist_state.select(None);
        } else {
            let i = self.selected.min(self.tracks.len() - 1);
            self.selected = i;
            self.playlist_state.select(Some(i));
        }
    }

    pub fn clear_playlist(&mut self) {
        self.player.stop();
        self.tracks.clear();
        self.selected = 0;
        self.playing_index = None;
        self.playlist_state.select(None);
        self.current_meta = TrackMeta::default();
        self.current_duration = None;
        self.album_protocol = None;
        self.album_dims = None;
        self.last_artwork_key = None;
        self.status = String::from("Playlist cleared");
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.cycle();
        self.status = format!("Repeat: {}", self.repeat.label());
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.shuffle_remaining();
            self.status = String::from("Shuffle: On");
        } else {
            self.status = String::from("Shuffle: Off");
        }
    }

    fn shuffle_remaining(&mut self) {
        let start = self.playing_index.map(|i| i + 1).unwrap_or(0);
        if start >= self.tracks.len() {
            return;
        }
        let mut seed: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let len = self.tracks.len() - start;
        for i in (1..len).rev() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (seed as usize) % (i + 1);
            self.tracks.swap(start + i, start + j);
        }
    }

    fn refresh_album_art(&mut self) {
        let Some(picker) = self.picker.as_mut() else {
            self.album_protocol = None;
            self.album_dims = None;
            return;
        };
        let Some(bytes) = self.current_meta.artwork.as_ref() else {
            self.album_protocol = None;
            self.album_dims = None;
            self.last_artwork_key = None;
            return;
        };
        let key = (bytes.len(), self.playing_index.unwrap_or(usize::MAX));
        if self.last_artwork_key == Some(key) && self.album_protocol.is_some() {
            return;
        }
        match image::load_from_memory(bytes) {
            Ok(img) => {
                self.album_dims = Some((img.width(), img.height()));
                let proto = picker.new_resize_protocol(img);
                self.album_protocol = Some(proto);
                self.last_artwork_key = Some(key);
            }
            Err(_) => {
                self.album_protocol = None;
                self.album_dims = None;
                self.last_artwork_key = None;
            }
        }
    }

    pub fn volume_step(&mut self, delta: f32) {
        let v = (self.player.volume() + delta).clamp(0.0, 1.5);
        self.player.set_volume(v);
        self.status = format!("Volume: {:>3.0}%", v * 100.0);
    }

    pub fn position(&self) -> Duration {
        self.player.tap.position()
    }
}

fn fmt_short(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn plural_s(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn short_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}
