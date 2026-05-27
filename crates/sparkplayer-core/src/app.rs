use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::backend::{
    AlbumArtRenderer, AudioBackend, ConfigStore, CoreKey, CoreKeyEvent, MediaLibrary, VideoBackend,
};
use crate::config::Config;
use crate::library::{self, Track, TrackRef};
use crate::metadata::TrackMeta;
use crate::subtitles::SubtitleSet;
use crate::theme::{self, Theme};
use crate::visualizer::{VisMode, Visualizer};

/// Empirical baseline for the audio path lag (CPAL ring + OS audio server
/// queue + DAC) on a typical PulseAudio/PipeWire setup.
const BASELINE_AV_OFFSET_SECS: f64 = 0.02;
/// Multiplier applied to the measured CPAL buffer to estimate the *minimum*
/// plausible audio-path lag for unusually high-latency backends.
const AV_OFFSET_BACKEND_MULT: f64 = 2.0;
/// Floor on the slewed offset.
const MIN_AV_OFFSET_SECS: f64 = 0.050;
/// Max change per tick when auto-tracking.
const AV_OFFSET_SLEW_PER_TICK: f64 = 0.005;
pub const AV_OFFSET_STEP_SECS: f64 = 0.025;

fn baseline_av_offset(audio_lat_secs: f64) -> f64 {
    (audio_lat_secs * AV_OFFSET_BACKEND_MULT).max(BASELINE_AV_OFFSET_SECS)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FocusPane {
    Playlist,
    Browser,
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

/// Central application state. Platform behavior is reached only through the
/// boxed backend trait objects; everything else (selection, playlist, A/V
/// arithmetic, escape menu, subtitle bookkeeping) is platform-agnostic and
/// shared verbatim between the native and web builds.
pub struct App {
    pub audio: Box<dyn AudioBackend>,
    pub video: Box<dyn VideoBackend>,
    pub library: Box<dyn MediaLibrary>,
    pub config: Box<dyn ConfigStore>,
    pub art: Box<dyn AlbumArtRenderer>,
    pub visualizer: Visualizer,

    pub tracks: Vec<Track>,
    pub selected: usize,
    pub playing_index: Option<usize>,
    pub playing_track: Option<TrackRef>,
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

    // A/V sync (pure arithmetic, shared). On web `advance` returns None so the
    // offset stays at its baseline and the `<video>` element self-syncs.
    pub av_offset_secs: f64,
    pub audio_output_latency_secs: f64,
    pub video_render_ewma_secs: f64,
    pub auto_av_offset: bool,

    pub subtitles: SubtitleSet,
    pub active_subtitle_track: Option<usize>,
    pub current_subtitle_text: Option<String>,
    /// Deadline (in `clock_secs`) until which the "subtitles available" hint shows.
    subtitle_announcement_until: Option<f64>,
    last_subtitle_track_count: usize,
    pub preferred_subtitle_lang: Option<String>,
    preferred_subtitle_applied: bool,

    pub should_quit: bool,
    pub show_help: bool,
    pub show_escape_menu: bool,
    pub escape_menu_selected: usize,
    pub fullscreen_vis: bool,

    pub theme: Theme,

    /// Monotonic wall-clock seconds, supplied by the platform each frame. Used
    /// for UI animation and timed announcements. `std::time::Instant` is
    /// unavailable on wasm, so time always flows in through here instead.
    pub clock_secs: f64,

    /// Layout rectangles recorded during the last `ui::draw`, used by the web
    /// build to position the `<video>` / `<img>` overlays and the file-picker
    /// buttons over the canvas.
    pub last_video_rect: Option<Rect>,
    pub last_art_rect: Option<Rect>,
    pub last_browser_rect: Option<Rect>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        audio: Box<dyn AudioBackend>,
        video: Box<dyn VideoBackend>,
        library: Box<dyn MediaLibrary>,
        config: Box<dyn ConfigStore>,
        art: Box<dyn AlbumArtRenderer>,
        initial_tracks: Vec<Track>,
        initial_dir: PathBuf,
        cfg: &Config,
    ) -> Self {
        let mut audio = audio;
        audio.set_volume(cfg.volume);
        let audio_output_latency_secs = audio.output_buffer_latency().as_secs_f64();
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
            audio,
            video,
            library,
            config,
            art,
            visualizer,
            tracks: initial_tracks,
            selected: 0,
            playing_index: None,
            playing_track: None,
            playlist_state,
            browser_dir: initial_dir,
            browser_entries: Vec::new(),
            browser_selected: 0,
            browser_state: ListState::default(),
            focus: FocusPane::Playlist,
            current_meta: TrackMeta::default(),
            current_duration: None,
            status: String::from("Ready"),
            repeat: RepeatMode::Off,
            shuffle: false,
            av_offset_secs: initial_av_offset,
            audio_output_latency_secs,
            video_render_ewma_secs: 0.0,
            auto_av_offset: true,
            subtitles: SubtitleSet::default(),
            active_subtitle_track: None,
            current_subtitle_text: None,
            subtitle_announcement_until: None,
            last_subtitle_track_count: 0,
            preferred_subtitle_lang: None,
            preferred_subtitle_applied: false,
            should_quit: false,
            show_help: false,
            show_escape_menu: false,
            escape_menu_selected: 0,
            fullscreen_vis: false,
            theme,
            clock_secs: 0.0,
            last_video_rect: None,
            last_art_rect: None,
            last_browser_rect: None,
        };
        app.refresh_browser();
        app
    }

    /// Advance the platform clock. Called once per frame by the run loop.
    pub fn set_clock(&mut self, secs: f64) {
        self.clock_secs = secs;
    }

    pub fn refresh_browser(&mut self) {
        self.browser_entries = self.library.browse(&self.browser_dir);
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
                        match self.library.load_playlist(&TrackRef::Path(path.clone())) {
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
        let source = self.tracks[idx].source.clone();
        self.current_meta = self.library.read_metadata(&source);
        if self.current_meta.artwork.is_none() {
            if let Some(bytes) = self.library.find_cover(&source) {
                self.current_meta.artwork = Some(bytes);
            }
        }
        // Tear down any previous video pipeline / subtitles before swapping.
        self.video.close();
        self.subtitles.cancel();
        self.subtitles = SubtitleSet::default();
        self.active_subtitle_track = None;
        self.current_subtitle_text = None;
        self.subtitle_announcement_until = None;
        self.last_subtitle_track_count = 0;
        self.preferred_subtitle_applied = false;

        match self.audio.play(&source) {
            Ok(dur_hint) => {
                self.current_duration = self.current_meta.duration.or(dur_hint);
                self.playing_index = Some(idx);
                self.playing_track = Some(source.clone());
                self.selected = idx;
                self.playlist_state.select(Some(idx));
                self.status = format!("Playing: {}", self.tracks[idx].display);
                self.refresh_album_art();
                if library::is_video(&source) {
                    self.video.open(&source);
                    self.subtitles = self.library.load_subtitles(&source);
                }
            }
            Err(e) => {
                self.status = format!("Decode error: {e}");
                self.playing_index = None;
                self.playing_track = None;
                self.current_duration = None;
            }
        }
        Ok(())
    }

    /// Drive video display: select the current subtitle cue and hand the
    /// display position to the video backend, then fold the backend's reported
    /// render time into the auto A/V offset (native only; web returns `None`).
    pub fn tick_video(&mut self) {
        if !self.video.is_loaded() {
            return;
        }
        if self.audio.is_paused() {
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
            self.subtitle_announcement_until = Some(self.clock_secs + 5.0);
        }
        self.last_subtitle_track_count = count;
        if !self.preferred_subtitle_applied && self.preferred_subtitle_lang.is_some() && count > 0 {
            let lang = self.preferred_subtitle_lang.clone().unwrap();
            if let Some(idx) = self.subtitles.find_track_by_language(&lang) {
                self.active_subtitle_track = Some(idx);
                self.current_subtitle_text = None;
                self.status = format!(
                    "Subtitles: {}",
                    self.subtitles
                        .track_label(idx)
                        .unwrap_or_else(|| format!("Track {}", idx + 1))
                );
                self.preferred_subtitle_applied = true;
            }
        }
        let sub = self.current_subtitle_text.clone();
        let elapsed = self.video.advance(pos, self.audio.is_paused(), sub.as_deref());
        if let Some(elapsed) = elapsed {
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
        if self.playing_index.is_some() && self.audio.is_finished() {
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
        if self.playing_index.is_none() || self.playing_track.is_none() {
            return;
        }
        let total = self.current_duration;
        match self.audio.seek_relative(delta, total) {
            Ok(()) => {
                let pos = self.audio.position();
                self.video.seek(pos);
                if self.auto_av_offset {
                    self.av_offset_secs = baseline_av_offset(self.audio_output_latency_secs);
                    self.video_render_ewma_secs = 0.0;
                }
                self.status = format!("Seek: {} ({:+.0}s)", fmt_short(pos), delta);
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
            match self.library.load_playlist(&TrackRef::Path(path)) {
                Ok(more) => {
                    let n = more.len();
                    self.tracks.extend(more);
                    self.refresh_playlist_state();
                    self.status = format!("Queued playlist ({n} track{})", plural_s(n));
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
        let new_tracks = self.library.scan_directory(dir);
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
        self.audio.stop();
        self.tracks.clear();
        self.selected = 0;
        self.playing_index = None;
        self.playing_track = None;
        self.playlist_state.select(None);
        self.current_meta = TrackMeta::default();
        self.current_duration = None;
        self.art.set_artwork(None, None);
        self.video.close();
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
    /// available. Returns None once the window expires or no tracks are loaded.
    pub fn subtitle_announcement(&self) -> Option<String> {
        let deadline = self.subtitle_announcement_until?;
        if self.clock_secs >= deadline {
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
        // Seed from the platform clock — `SystemTime` is unavailable on wasm.
        let mut seed: u64 = (self.clock_secs * 1_000_000.0) as u64 ^ 0x9E3779B97F4A7C15;
        if seed == 0 {
            seed = 1;
        }
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
        let key = self
            .current_meta
            .artwork
            .as_ref()
            .map(|b| (b.len(), self.playing_index.unwrap_or(usize::MAX)));
        self.art
            .set_artwork(self.current_meta.artwork.as_deref(), key);
    }

    /// Nudge the A/V sync offset. Positive `delta` pushes the picked video
    /// frame further into the past; negative pulls it forward.
    pub fn adjust_av_offset(&mut self, delta: f64) {
        self.av_offset_secs = (self.av_offset_secs + delta).clamp(-0.5, 2.0);
        self.auto_av_offset = false;
        self.status = format!("A/V offset: {:+.0} ms (manual)", self.av_offset_secs * 1000.0);
    }

    pub fn volume_step(&mut self, delta: f32) {
        let v = (self.audio.volume() + delta).clamp(0.0, 1.5);
        self.audio.set_volume(v);
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

    /// Step the active subtitle track by ±1, including the "Off" slot.
    pub fn step_subtitle_track(&mut self, delta: i32) {
        let count = self.subtitles.track_count();
        if count == 0 {
            self.status = String::from("No subtitles available (still loading?)");
            return;
        }
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

    /// Cycle the display mode: Normal → in-app Fullscreen → external Video
    /// Window → Normal. The external-window step is a no-op where the video
    /// backend doesn't support one (web), so it collapses to Normal↔Fullscreen.
    pub fn cycle_display_mode(&mut self) {
        if self.video.external_window_enabled() {
            self.video.set_external_window(false);
            self.fullscreen_vis = false;
            self.status = String::from("Display: normal");
        } else if self.fullscreen_vis {
            self.fullscreen_vis = false;
            if self.video.supports_external_window() {
                self.video.set_external_window(true);
                self.status = if self.video.is_loaded() {
                    String::from("Display: video window")
                } else {
                    String::from("Display: video window (armed — opens on next video)")
                };
            } else {
                self.status = String::from("Display: normal");
            }
        } else {
            self.fullscreen_vis = true;
            self.status = String::from("Display: fullscreen");
        }
    }

    /// Flip the dedicated-window option from the escape menu.
    pub fn toggle_video_window(&mut self) {
        if !self.video.supports_external_window() {
            return;
        }
        let on = !self.video.external_window_enabled();
        self.video.set_external_window(on);
        self.status = if on {
            if self.video.is_loaded() {
                String::from("Video window: opened")
            } else {
                String::from("Video window: armed — opens when next video plays")
            }
        } else {
            String::from("Video window: closed")
        };
    }

    /// Drain keys forwarded from the external playback window (native only).
    pub fn drain_external_keys(&self) -> Vec<CoreKeyEvent> {
        self.video.drain_external_keys()
    }

    /// Build the escape-menu rows. Rows whose preconditions aren't met are
    /// returned `enabled = false` so the renderer dims them and the key handler
    /// skips over them.
    pub fn escape_menu_items(&self) -> Vec<EscapeMenuItem> {
        let has_video = self.video.is_loaded();
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
                value: format!("{:>3.0}%", self.audio.volume() * 100.0),
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
                enabled: self.video.supports_external_window(),
                label: "Video Window",
                value: if self.video.external_window_enabled() {
                    "On"
                } else {
                    "Off"
                }
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

    /// Apply a horizontal adjustment (left/right arrow) to the highlighted row.
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

    /// Apply an "activate" (Enter / Space) to the highlighted row. Returns
    /// `true` when the menu should close as a result.
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
            EscapeMenuKind::AvOffset => self.reset_av_offset_auto(),
            _ => {}
        }
        Ok(false)
    }

    /// Persist the user-tunable bits (theme, volume, visualizer).
    pub fn save_config(&self) {
        let cfg = Config {
            theme: self.theme.name.to_string(),
            volume: self.audio.volume(),
            visualizer: self.visualizer.mode.name().to_string(),
        };
        self.config.save(&cfg);
    }

    pub fn position(&self) -> Duration {
        self.audio.position()
    }

    /// Dispatch a platform-neutral key event. Shared by both builds so the
    /// keymap stays identical.
    pub fn handle_key(&mut self, ev: CoreKeyEvent) -> Result<()> {
        let code = ev.code;
        let ctrl = ev.ctrl;

        if self.show_help {
            match code {
                CoreKey::Esc
                | CoreKey::Char('?')
                | CoreKey::Char('h')
                | CoreKey::Char('q') => self.show_help = false,
                _ => {}
            }
            return Ok(());
        }

        if self.show_escape_menu {
            if matches!(code, CoreKey::Char('c')) && ctrl {
                self.should_quit = true;
                return Ok(());
            }
            match code {
                CoreKey::Esc => self.close_escape_menu(),
                CoreKey::Up => self.escape_menu_move(-1),
                CoreKey::Down => self.escape_menu_move(1),
                CoreKey::Left => self.escape_menu_adjust(-1)?,
                CoreKey::Right => self.escape_menu_adjust(1)?,
                CoreKey::Enter | CoreKey::Char(' ') => {
                    if self.escape_menu_activate()? {
                        self.close_escape_menu();
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        match code {
            CoreKey::Char('q') => self.should_quit = true,
            CoreKey::Esc => self.open_escape_menu(),
            CoreKey::Char('c') if ctrl => self.should_quit = true,
            CoreKey::Char(' ') => self.audio.toggle_pause(),
            CoreKey::Char('n') => self.next_track()?,
            CoreKey::Char('p') => self.prev_track()?,
            CoreKey::Char('v') => self.cycle_visualizer(),
            CoreKey::Char('t') => self.cycle_theme(),
            CoreKey::Char('f') => self.cycle_display_mode(),
            CoreKey::Char('r') => self.cycle_repeat(),
            CoreKey::Char('s') => self.toggle_shuffle(),
            CoreKey::Char('a') => self.queue_selected_browser(),
            CoreKey::Char('A') => self.queue_browser_directory(),
            CoreKey::Char('C') => self.clear_playlist(),
            CoreKey::Char('c') => self.cycle_subtitle_track(),
            CoreKey::Char('?') | CoreKey::Char('h') => self.show_help = true,
            CoreKey::Tab => self.focus_next(),
            CoreKey::Up => self.move_selection(-1),
            CoreKey::Down => self.move_selection(1),
            CoreKey::PageUp => self.page(-1),
            CoreKey::PageDown => self.page(1),
            CoreKey::Home => self.select_first(),
            CoreKey::End => self.select_last(),
            CoreKey::Left if ctrl => self.seek_seconds(-30.0),
            CoreKey::Right if ctrl => self.seek_seconds(30.0),
            CoreKey::Left => self.seek_seconds(-10.0),
            CoreKey::Right => self.seek_seconds(10.0),
            CoreKey::Char('-') | CoreKey::Char('_') => self.volume_step(-0.05),
            CoreKey::Char('+') | CoreKey::Char('=') => self.volume_step(0.05),
            CoreKey::Char('[') => self.adjust_av_offset(-AV_OFFSET_STEP_SECS),
            CoreKey::Char(']') => self.adjust_av_offset(AV_OFFSET_STEP_SECS),
            CoreKey::Enter => self.activate_selection()?,
            _ => {}
        }
        Ok(())
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
