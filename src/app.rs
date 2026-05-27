use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::widgets::ListState;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;

use crate::audio::AudioPlayer;
use crate::config;
use crate::external_window::ExternalVideoWindow;
use crate::library::{self, Track};
use crate::metadata::{self, TrackMeta};
use crate::subtitles::{self, SubtitleSet};
use crate::theme::{self, Theme};
use crate::video::VideoStream;
use crate::visualizer::{VisMode, Visualizer};

/// Empirical baseline for the audio path lag (CPAL ring + OS audio server
/// queue + DAC) on a typical PulseAudio/PipeWire setup. CPAL can only tell us
/// its own buffer size; the audio server's queue on top is opaque, so we
/// can't compute the true lag from inside the process. This 20 ms default
/// matches what the player shipped with before auto-tracking and is the
/// starting point in auto mode.
const BASELINE_AV_OFFSET_SECS: f64 = 0.02;
/// Multiplier applied to the measured CPAL buffer to estimate the *minimum*
/// plausible audio-path lag for unusually high-latency backends. Only kicks
/// in if `cpal_buffer × this > BASELINE`.
const AV_OFFSET_BACKEND_MULT: f64 = 2.0;
/// Floor on the slewed offset — never let auto-tracking push the video so
/// far forward that we'd ask for unready frames.
const MIN_AV_OFFSET_SECS: f64 = 0.050;
/// Max change per tick when auto-tracking, so the picture doesn't visibly jump.
const AV_OFFSET_SLEW_PER_TICK: f64 = 0.005;
pub const AV_OFFSET_STEP_SECS: f64 = 0.025;

/// Target offset before subtracting render time. Uses the empirical 300 ms
/// baseline, but bumps higher if the negotiated CPAL buffer × backend mult
/// implies we're on a particularly high-latency audio path.
fn baseline_av_offset(audio_lat_secs: f64) -> f64 {
    (audio_lat_secs * AV_OFFSET_BACKEND_MULT).max(BASELINE_AV_OFFSET_SECS)
}

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
pub enum EscapeMenuKind {
    Volume,
    Subtitle,
    AvOffset,
    Visualizer,
    Theme,
    Fullscreen,
    VideoWindow,
    Repeat,
    Shuffle,
    Separator,
    Quit,
}

pub struct EscapeMenuItem {
    pub kind: EscapeMenuKind,
    pub enabled: bool,
    pub label: &'static str,
    pub value: String,
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

    pub video: Option<VideoStream>,
    pub video_protocol: Option<StatefulProtocol>,
    pub video_dims: Option<(u32, u32)>,
    pub av_offset_secs: f64,
    pub audio_output_latency_secs: f64,
    pub video_render_ewma_secs: f64,
    pub auto_av_offset: bool,

    pub subtitles: SubtitleSet,
    pub active_subtitle_track: Option<usize>,
    pub current_subtitle_text: Option<String>,
    pub subtitle_announcement_until: Option<Instant>,
    last_subtitle_track_count: usize,

    pub should_quit: bool,
    pub show_help: bool,
    pub show_escape_menu: bool,
    pub escape_menu_selected: usize,
    pub fullscreen_vis: bool,
    pub external_window: Option<ExternalVideoWindow>,
    pub external_window_enabled: bool,

    pub theme: Theme,

    graphics_choice: GraphicsChoice,
}

impl App {
    pub fn new(
        initial_tracks: Vec<Track>,
        initial_dir: PathBuf,
        graphics_choice: GraphicsChoice,
        cfg: &config::Config,
    ) -> Result<Self> {
        let mut player = AudioPlayer::new()?;
        player.set_volume(cfg.volume);
        let audio_output_latency_secs = player.output_buffer_latency().as_secs_f64();
        let initial_av_offset = baseline_av_offset(audio_output_latency_secs);
        let mut visualizer = Visualizer::new();
        if let Some(mode) = VisMode::from_name(&cfg.visualizer) {
            visualizer.mode = mode;
        }
        let theme = theme::by_name(&cfg.theme);
        theme::set_current(theme);
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
            video: None,
            video_protocol: None,
            video_dims: None,
            av_offset_secs: initial_av_offset,
            audio_output_latency_secs,
            video_render_ewma_secs: 0.0,
            auto_av_offset: true,
            subtitles: SubtitleSet::default(),
            active_subtitle_track: None,
            current_subtitle_text: None,
            subtitle_announcement_until: None,
            last_subtitle_track_count: 0,
            should_quit: false,
            show_help: false,
            show_escape_menu: false,
            escape_menu_selected: 0,
            fullscreen_vis: false,
            external_window: None,
            external_window_enabled: false,
            theme,
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
            .unwrap_or_else(Picker::halfblocks);
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
        // Tear down any previous video pipeline before swapping audio sources.
        self.video = None;
        self.video_protocol = None;
        self.video_dims = None;
        self.external_window = None;
        self.subtitles.cancel();
        self.subtitles = SubtitleSet::default();
        self.active_subtitle_track = None;
        self.current_subtitle_text = None;
        self.subtitle_announcement_until = None;
        self.last_subtitle_track_count = 0;

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
                if library::is_video_file(&path) {
                    match VideoStream::open(&path) {
                        Ok(v) => {
                            self.video_dims = Some((v.width, v.height));
                            self.video = Some(v);
                            if self.external_window_enabled {
                                self.spawn_external_window();
                            }
                        }
                        Err(e) => {
                            self.status = format!("Video decode error: {e}");
                        }
                    }
                    self.subtitles = subtitles::load_for_video(&path);
                }
            }
            Err(e) => {
                self.status = format!("Decode error: {e}");
                self.playing_index = None;
                self.current_duration = None;
            }
        }
        Ok(())
    }

    /// Pull the latest video frame keyed to the audio output time and refresh
    /// the on-screen image protocol if a new frame is ready.
    pub fn tick_video(&mut self) {
        let Some(video) = self.video.as_ref() else {
            return;
        };
        if self.player.is_paused() {
            return;
        }
        let pos = self.position().as_secs_f64() - self.av_offset_secs;
        if pos < 0.0 {
            return;
        }
        self.current_subtitle_text = self
            .active_subtitle_track
            .and_then(|i| self.subtitles.cue_at(i, pos));
        let count = self.subtitles.track_count();
        if count > self.last_subtitle_track_count && self.last_subtitle_track_count == 0 {
            // First subtitle track(s) for this video just became available —
            // briefly announce them under the video.
            self.subtitle_announcement_until = Some(Instant::now() + Duration::from_secs(5));
        }
        self.last_subtitle_track_count = count;
        // Drop the external window if it died (close button, init failure).
        if self.external_window.as_ref().is_some_and(|w| !w.is_alive()) {
            self.external_window = None;
            self.external_window_enabled = false;
        }
        let Some(frame) = video.frame_at(pos) else {
            return;
        };
        // If the SDL window is up, render there exclusively — the in-terminal
        // image path is expensive and pointless when the user has the
        // dedicated playback window in focus.
        if let Some(window) = self.external_window.as_ref() {
            self.video_dims = Some((frame.width, frame.height));
            self.video_protocol = None;
            window.set_subtitle(self.current_subtitle_text.clone());
            window.submit_frame(frame);
            return;
        }
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let started = Instant::now();
        let buf = match image::RgbImage::from_raw(frame.width, frame.height, frame.data) {
            Some(b) => b,
            None => return,
        };
        let dyn_img = image::DynamicImage::ImageRgb8(buf);
        self.video_dims = Some((frame.width, frame.height));
        self.video_protocol = Some(picker.new_resize_protocol(dyn_img));
        let elapsed = started.elapsed().as_secs_f64();
        // EWMA alpha=0.1: smooths jitter, converges within ~10 ticks (~330 ms).
        self.video_render_ewma_secs = if self.video_render_ewma_secs == 0.0 {
            elapsed
        } else {
            self.video_render_ewma_secs * 0.9 + elapsed * 0.1
        };
        if self.auto_av_offset {
            let target = (baseline_av_offset(self.audio_output_latency_secs)
                - self.video_render_ewma_secs)
                .max(MIN_AV_OFFSET_SECS);
            let delta = (target - self.av_offset_secs)
                .clamp(-AV_OFFSET_SLEW_PER_TICK, AV_OFFSET_SLEW_PER_TICK);
            self.av_offset_secs += delta;
        }
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
                if let Some(video) = self.video.as_ref() {
                    video.seek(pos);
                }
                // Drop the render-time EWMA and re-anchor to the baseline:
                // after seek, the OS audio ring is briefly empty so the
                // effective lag changes for ~one buffer; slewing from the
                // pre-seek value would chase a moving target.
                if self.auto_av_offset {
                    self.av_offset_secs = baseline_av_offset(self.audio_output_latency_secs);
                    self.video_render_ewma_secs = 0.0;
                }
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
        self.video = None;
        self.video_protocol = None;
        self.video_dims = None;
        self.external_window = None;
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

    /// Text to show under the video for ~5s after subtitles first become
    /// available, listing the tracks and the hotkey. Returns None once the
    /// window expires or no tracks are loaded.
    pub fn subtitle_announcement(&self) -> Option<String> {
        let deadline = self.subtitle_announcement_until?;
        if Instant::now() >= deadline {
            return None;
        }
        let count = self.subtitles.track_count();
        if count == 0 {
            return None;
        }
        let labels: Vec<String> = (0..count)
            .filter_map(|i| self.subtitles.track_label(i))
            .collect();
        if labels.is_empty() {
            return None;
        }
        Some(format!(
            "Subtitles available: {} — press 'c' to cycle",
            labels.join(", ")
        ))
    }

    pub fn cycle_subtitle_track(&mut self) {
        let count = self.subtitles.track_count();
        if count == 0 {
            self.active_subtitle_track = None;
            self.current_subtitle_text = None;
            self.status = String::from("No subtitles available (still loading?)");
            return;
        }
        let next = match self.active_subtitle_track {
            None => Some(0),
            Some(i) if i + 1 < count => Some(i + 1),
            Some(_) => None,
        };
        self.active_subtitle_track = next;
        self.current_subtitle_text = None;
        self.status = match next {
            Some(i) => format!(
                "Subtitles: {}",
                self.subtitles
                    .track_label(i)
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            ),
            None => String::from("Subtitles: off"),
        };
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

    /// Nudge the A/V sync offset. Positive `delta` pushes the picked video
    /// frame further into the past (use when video is ahead of audio);
    /// negative pulls it forward.
    pub fn adjust_av_offset(&mut self, delta: f64) {
        self.av_offset_secs = (self.av_offset_secs + delta).clamp(-0.5, 2.0);
        self.auto_av_offset = false;
        self.status = format!(
            "A/V offset: {:+.0} ms (manual)",
            self.av_offset_secs * 1000.0
        );
    }

    pub fn volume_step(&mut self, delta: f32) {
        let v = (self.player.volume() + delta).clamp(0.0, 1.5);
        self.player.set_volume(v);
        self.status = format!("Volume: {:>3.0}%", v * 100.0);
        self.save_config();
    }

    pub fn cycle_visualizer(&mut self) {
        self.visualizer.toggle_mode();
        self.status = format!("Visualizer: {}", self.visualizer.mode.label());
        self.save_config();
    }

    pub fn cycle_visualizer_back(&mut self) {
        self.visualizer.toggle_mode_back();
        self.status = format!("Visualizer: {}", self.visualizer.mode.label());
        self.save_config();
    }

    pub fn cycle_theme(&mut self) {
        let next = theme::next_after(self.theme.name);
        self.theme = next;
        theme::set_current(next);
        self.status = format!("Theme: {}", next.label);
        self.save_config();
    }

    pub fn cycle_theme_back(&mut self) {
        let prev = theme::prev_before(self.theme.name);
        self.theme = prev;
        theme::set_current(prev);
        self.status = format!("Theme: {}", prev.label);
        self.save_config();
    }

    /// Step the active subtitle track by ±1, including the "Off" slot. Wraps
    /// at both ends. No-op when no subtitle tracks are loaded.
    pub fn step_subtitle_track(&mut self, delta: i32) {
        let count = self.subtitles.track_count();
        if count == 0 {
            self.status = String::from("No subtitles available (still loading?)");
            return;
        }
        // Slots: 0..count = tracks, count = Off.
        let total = (count + 1) as i32;
        let current = self
            .active_subtitle_track
            .map(|i| i as i32)
            .unwrap_or(count as i32);
        let next_slot = (current + delta).rem_euclid(total);
        self.active_subtitle_track = if (next_slot as usize) >= count {
            None
        } else {
            Some(next_slot as usize)
        };
        self.current_subtitle_text = None;
        self.status = match self.active_subtitle_track {
            Some(i) => format!(
                "Subtitles: {}",
                self.subtitles
                    .track_label(i)
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            ),
            None => String::from("Subtitles: off"),
        };
    }

    pub fn reset_av_offset_auto(&mut self) {
        self.auto_av_offset = true;
        self.video_render_ewma_secs = 0.0;
        self.av_offset_secs = baseline_av_offset(self.audio_output_latency_secs);
        self.status = String::from("A/V offset: auto");
    }

    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen_vis = !self.fullscreen_vis;
    }

    /// Cycle the display mode: Normal → in-terminal Fullscreen → external
    /// Video Window → Normal. Bound to `f`.
    pub fn cycle_display_mode(&mut self) {
        if self.external_window_enabled {
            // Video Window → Normal
            self.external_window_enabled = false;
            self.external_window = None;
            self.fullscreen_vis = false;
            self.status = String::from("Display: normal");
        } else if self.fullscreen_vis {
            // Fullscreen → Video Window
            self.fullscreen_vis = false;
            self.external_window_enabled = true;
            if self.video.is_some() {
                self.spawn_external_window();
            } else {
                self.status = String::from(
                    "Display: video window (armed — opens on next video)",
                );
            }
        } else {
            // Normal → Fullscreen
            self.fullscreen_vis = true;
            self.status = String::from("Display: fullscreen");
        }
    }

    /// Open a dedicated SDL window for fullscreen video playback. No-op if the
    /// window is already up or no video is loaded.
    fn spawn_external_window(&mut self) {
        if self.external_window.is_some() || self.video.is_none() {
            return;
        }
        match ExternalVideoWindow::spawn() {
            Ok(w) => {
                self.external_window = Some(w);
                // Stop rendering the terminal video tile — frames go to SDL now.
                self.video_protocol = None;
                self.status = String::from("Video window: opened");
            }
            Err(e) => {
                self.external_window_enabled = false;
                self.status = format!("Video window error: {e}");
            }
        }
    }

    /// Flip the dedicated-window option from the escape menu. Spawning happens
    /// immediately if a video is loaded; otherwise the flag persists and the
    /// window opens the next time a video starts.
    pub fn toggle_video_window(&mut self) {
        self.external_window_enabled = !self.external_window_enabled;
        if self.external_window_enabled {
            if self.video.is_some() {
                self.spawn_external_window();
            } else {
                self.status = String::from(
                    "Video window: armed — opens when next video plays",
                );
            }
        } else {
            self.external_window = None;
            self.status = String::from("Video window: closed");
        }
    }

    /// Drain queued keyboard events from the external playback window so the
    /// main run loop can dispatch them through the regular handler. Returns
    /// pairs of `(KeyCode, KeyModifiers)`.
    pub fn drain_external_keys(&self) -> Vec<(crossterm::event::KeyCode, crossterm::event::KeyModifiers)> {
        let Some(window) = self.external_window.as_ref() else {
            return Vec::new();
        };
        window
            .drain_keys()
            .into_iter()
            .map(|k| (k.code, k.mods))
            .collect()
    }

    /// Build the list of menu rows. Rows whose preconditions aren't met (e.g.
    /// subtitles when no video is playing) are returned with `enabled = false`
    /// so the renderer dims them and the key handler skips over them.
    pub fn escape_menu_items(&self) -> Vec<EscapeMenuItem> {
        let has_video = self.video.is_some();
        let sub_label = if !has_video {
            "—".to_string()
        } else if self.subtitles.track_count() == 0 {
            "None".to_string()
        } else {
            match self.active_subtitle_track {
                Some(i) => self
                    .subtitles
                    .track_label(i)
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                None => "Off".to_string(),
            }
        };
        let av_label = if !has_video {
            "—".to_string()
        } else if self.auto_av_offset {
            format!("Auto ({:+.0} ms)", self.av_offset_secs * 1000.0)
        } else {
            format!("{:+.0} ms", self.av_offset_secs * 1000.0)
        };
        vec![
            EscapeMenuItem {
                kind: EscapeMenuKind::Volume,
                enabled: true,
                label: "Volume",
                value: format!("{:>3.0}%", self.player.volume() * 100.0),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Subtitle,
                enabled: has_video && self.subtitles.track_count() > 0,
                label: "Subtitle",
                value: sub_label,
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::AvOffset,
                enabled: has_video,
                label: "A/V Offset",
                value: av_label,
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Visualizer,
                enabled: true,
                label: "Visualizer",
                value: self.visualizer.mode.label().to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Theme,
                enabled: true,
                label: "Theme",
                value: self.theme.label.to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Fullscreen,
                enabled: true,
                label: "Fullscreen",
                value: if self.fullscreen_vis { "On" } else { "Off" }.to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::VideoWindow,
                enabled: true,
                label: "Video Window",
                value: if self.external_window_enabled { "On" } else { "Off" }
                    .to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Repeat,
                enabled: true,
                label: "Repeat",
                value: self.repeat.label().to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Shuffle,
                enabled: true,
                label: "Shuffle",
                value: if self.shuffle { "On" } else { "Off" }.to_string(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Separator,
                enabled: false,
                label: "",
                value: String::new(),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::Quit,
                enabled: true,
                label: "Quit",
                value: String::new(),
            },
        ]
    }

    fn selectable_indices(&self) -> Vec<usize> {
        self.escape_menu_items()
            .iter()
            .enumerate()
            .filter(|(_, it)| it.enabled && it.kind != EscapeMenuKind::Separator)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn open_escape_menu(&mut self) {
        self.show_escape_menu = true;
        if let Some(&first) = self.selectable_indices().first() {
            self.escape_menu_selected = first;
        }
    }

    pub fn close_escape_menu(&mut self) {
        self.show_escape_menu = false;
    }

    pub fn escape_menu_move(&mut self, delta: i32) {
        let selectable = self.selectable_indices();
        if selectable.is_empty() {
            return;
        }
        let pos = selectable
            .iter()
            .position(|&i| i == self.escape_menu_selected)
            .unwrap_or(0);
        let n = selectable.len() as i32;
        let new_pos = (pos as i32 + delta).rem_euclid(n) as usize;
        self.escape_menu_selected = selectable[new_pos];
    }

    /// Apply a horizontal adjustment (left/right arrow) to the currently
    /// highlighted row.
    pub fn escape_menu_adjust(&mut self, delta: i32) -> Result<()> {
        let items = self.escape_menu_items();
        let Some(item) = items.get(self.escape_menu_selected) else {
            return Ok(());
        };
        if !item.enabled {
            return Ok(());
        }
        match item.kind {
            EscapeMenuKind::Volume => self.volume_step(0.05 * delta as f32),
            EscapeMenuKind::Subtitle => self.step_subtitle_track(delta),
            EscapeMenuKind::AvOffset => self.adjust_av_offset(AV_OFFSET_STEP_SECS * delta as f64),
            EscapeMenuKind::Visualizer => {
                if delta > 0 {
                    self.cycle_visualizer();
                } else {
                    self.cycle_visualizer_back();
                }
            }
            EscapeMenuKind::Theme => {
                if delta > 0 {
                    self.cycle_theme();
                } else {
                    self.cycle_theme_back();
                }
            }
            EscapeMenuKind::Fullscreen => self.toggle_fullscreen(),
            EscapeMenuKind::VideoWindow => self.toggle_video_window(),
            EscapeMenuKind::Repeat => self.cycle_repeat(),
            EscapeMenuKind::Shuffle => self.toggle_shuffle(),
            EscapeMenuKind::Quit | EscapeMenuKind::Separator => {}
        }
        Ok(())
    }

    /// Apply an "activate" (Enter / Space) to the currently highlighted row.
    /// Returns `true` when the menu should close as a result.
    pub fn escape_menu_activate(&mut self) -> Result<bool> {
        let items = self.escape_menu_items();
        let Some(item) = items.get(self.escape_menu_selected) else {
            return Ok(false);
        };
        if !item.enabled {
            return Ok(false);
        }
        match item.kind {
            EscapeMenuKind::Quit => {
                self.should_quit = true;
                return Ok(true);
            }
            EscapeMenuKind::Fullscreen => self.toggle_fullscreen(),
            EscapeMenuKind::VideoWindow => self.toggle_video_window(),
            EscapeMenuKind::Repeat => self.cycle_repeat(),
            EscapeMenuKind::Shuffle => self.toggle_shuffle(),
            // For A/V offset, Enter resets to Auto.
            EscapeMenuKind::AvOffset => self.reset_av_offset_auto(),
            _ => {}
        }
        Ok(false)
    }

    /// Persist the user-tunable bits (theme, volume, visualizer) to the
    /// platform config directory. Errors are swallowed: a write failure on a
    /// read-only home shouldn't crash the player mid-session.
    pub fn save_config(&self) {
        let cfg = config::Config {
            theme: self.theme.name.to_string(),
            volume: self.player.volume(),
            visualizer: self.visualizer.mode.name().to_string(),
        };
        config::save(&cfg);
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
