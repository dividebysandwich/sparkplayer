use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::backend::{
    AlbumArtRenderer, AudioBackend, ConfigStore, CoreKey, CoreKeyEvent, CoreMouseEvent,
    CoreMouseKind, MediaLibrary, VideoBackend,
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

/// Disagreement (seconds) between the smoothed video clock and the raw audio
/// position that forces a hard resync. Must sit *above* one audio-buffer
/// staircase step (hundreds of ms on high-latency backends) so the normal
/// wobble is filtered, not snapped, yet *below* a seek (>=5 s here) so seeks
/// snap instantly instead of slewing.
const VIDEO_CLOCK_RESYNC_SECS: f64 = 1.0;
/// A gap between ticks larger than this means we were parked (pause/resume) or
/// stalled; re-seed from the audio position rather than free-running across it.
const VIDEO_CLOCK_STALL_SECS: f64 = 0.5;
/// Time constant of the master-clock low-pass filter. Long enough to attenuate
/// a coarse (hundreds-of-ms) audio-callback staircase down to a few ms of
/// ripple — well under one frame — short enough to settle within a couple of
/// seconds after the initial sync. Audio-vs-CPU crystal drift is only ~ppm, so
/// this slow correction still tracks it easily.
const VIDEO_CLOCK_SMOOTH_TAU_SECS: f64 = 1.5;

/// The project's GitHub page, opened from the escape menu's "GitHub" entry.
pub const GITHUB_URL: &str = "https://github.com/dividebysandwich/sparkplayer";

fn baseline_av_offset(audio_lat_secs: f64) -> f64 {
    (audio_lat_secs * AV_OFFSET_BACKEND_MULT).max(BASELINE_AV_OFFSET_SECS)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FocusPane {
    Playlist,
    Browser,
}

/// A clickable playback control in the "Now Playing" badge row. The UI records
/// each badge's on-screen rect during draw (into `App::control_hits`) so
/// `handle_mouse` can map a click back to the action.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MouseControl {
    /// Transport buttons (cassette-deck style).
    SeekBack,
    Stop,
    PlayPause,
    SeekForward,
    CycleRepeat,
    ToggleShuffle,
    ToggleFavorite,
    CycleSubtitle,
}

/// Whether the app is capturing typed text, and for what. In `Filter` mode
/// keystrokes edit the live `filter_query`; in `SavePlaylist` mode they edit the
/// `input_buffer` (the filename to write).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum InputMode {
    Normal,
    Filter,
    SavePlaylist,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum EscapeMenuKind {
    Volume,
    AudioTrack,
    Subtitle,
    AvOffset,
    Visualizer,
    FftSize,
    Theme,
    Fullscreen,
    VideoWindow,
    Repeat,
    Shuffle,
    Help,
    Github,
    Separator,
    Quit,
}

pub struct EscapeMenuItem {
    pub kind: EscapeMenuKind,
    pub enabled: bool,
    pub label: &'static str,
    pub value: String,
}

/// Which slice of the library the global search overlay browses. `Tab` cycles
/// through these; the query then fuzzy-matches within the slice.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SearchScope {
    All,
    Favorites,
    Recent,
    MostPlayed,
}

impl SearchScope {
    pub fn cycle(self) -> Self {
        match self {
            SearchScope::All => SearchScope::Favorites,
            SearchScope::Favorites => SearchScope::Recent,
            SearchScope::Recent => SearchScope::MostPlayed,
            SearchScope::MostPlayed => SearchScope::All,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SearchScope::All => "All",
            SearchScope::Favorites => "Favorites",
            SearchScope::Recent => "Recent",
            SearchScope::MostPlayed => "Most Played",
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

/// The in-app fullscreen display modes. `f` cycles through these (the external
/// video window is a separate step handled in [`App::cycle_display_mode`]).
/// `AlbumArt` and `AlbumArtVis` are music-only — they show the track artwork
/// (rendered graphically when the terminal supports it, ASCII otherwise).
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum FullscreenMode {
    /// Not fullscreen — the normal multi-panel layout.
    #[default]
    Off,
    /// The active visualizer (or the video, when one is loaded) fills the screen.
    Visualizer,
    /// The album/song art fills the screen.
    AlbumArt,
    /// The album/song art fills the screen with the active visualizer overlaid
    /// in the bottom-right quarter.
    AlbumArtVis,
}

impl FullscreenMode {
    /// Whether any fullscreen mode is active (i.e. not [`FullscreenMode::Off`]).
    pub fn is_on(self) -> bool {
        !matches!(self, FullscreenMode::Off)
    }

    /// Short label for the escape-menu "Fullscreen" row.
    pub fn label(self) -> &'static str {
        match self {
            FullscreenMode::Off => "Off",
            FullscreenMode::Visualizer => "Visualizer",
            FullscreenMode::AlbumArt => "Album Art",
            FullscreenMode::AlbumArtVis => "Art + Visualizer",
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

    /// Text-capture state for the search/filter and save-playlist prompts.
    pub input_mode: InputMode,
    /// Filename being typed in `SavePlaylist` mode.
    pub input_buffer: String,
    /// Active search filter (empty = no filter). Applies to `filter_pane`.
    pub filter_query: String,
    /// Which pane the current `filter_query` narrows.
    pub filter_pane: FocusPane,

    pub current_meta: TrackMeta,
    pub current_duration: Option<Duration>,
    pub status: String,

    pub repeat: RepeatMode,
    pub shuffle: bool,

    /// Favorited track locators (paths). Toggled with `F`, persisted.
    pub favorites: HashSet<String>,
    /// Recently-played track locators, most-recent first (capped).
    pub recent: Vec<String>,
    /// Per-track play counts keyed by locator.
    pub play_counts: HashMap<String, u32>,

    /// Whole-library track index for the global search overlay. Empty until the
    /// platform finishes its background scan (see `set_search_index`); always
    /// empty on web (no filesystem).
    pub search_index: Vec<Track>,
    /// Whether the background index scan has reported in at least once.
    pub search_index_ready: bool,
    pub show_search: bool,
    pub search_query: String,
    /// Cursor position *within the current result list* (not the index).
    pub search_selected: usize,
    pub search_scope: SearchScope,

    // A/V sync (pure arithmetic, shared). On web `advance` returns None so the
    // offset stays at its baseline and the `<video>` element self-syncs.
    pub av_offset_secs: f64,
    pub audio_output_latency_secs: f64,
    pub video_render_ewma_secs: f64,
    pub auto_av_offset: bool,

    /// Free-running media clock used to pace video, slewed toward the audio
    /// position rather than read from it raw. The audio sink consumes samples
    /// a whole buffer at a time, so `audio.position()` is flat for several
    /// video frames and then jumps by a buffer's worth — driving the frame
    /// selector straight off that staircase makes motion freeze-then-skip.
    /// Instead `video_clock_secs` advances by real wall-clock time each tick
    /// and is nudged toward the audio position, so video moves smoothly while
    /// staying locked to audio over time (the approach VLC/mpv take).
    video_clock_secs: f64,
    /// `clock_secs` captured at the last `smooth_video_clock` update, used to
    /// measure real elapsed time between video ticks.
    video_clock_wall: f64,
    /// Whether `video_clock_secs` has been seeded from a real audio position.
    video_clock_init: bool,

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
    /// Vertical scroll offset (in lines) of the help overlay. Clamped to the
    /// content height during rendering.
    pub help_scroll: u16,
    pub show_escape_menu: bool,
    pub escape_menu_selected: usize,
    pub fullscreen: FullscreenMode,

    /// Whether the platform can open external URLs (web: yes via the browser;
    /// native: no). Gates the escape menu's "GitHub" entry.
    pub url_open_supported: bool,
    /// A URL the platform should open, set when the user activates the GitHub
    /// entry and drained by the run loop.
    pending_url_open: Option<String>,

    pub theme: Theme,

    /// Whether the display can render distinct 24-bit fg/bg colors per cell.
    /// Lets the spectrogram use half-block cells (two colors → double vertical
    /// resolution); when false it falls back to solid full-cell blocks. Native
    /// sets it from `COLORTERM`; defaults on (the palette is 24-bit throughout).
    pub truecolor: bool,

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

    /// Mouse hit-test rects, recorded each `ui::draw` and consumed by
    /// `handle_mouse`. The inner (bordered) content rects of the two lists, the
    /// progress/seek bar, the visualizer, and each clickable control badge.
    pub playlist_hit: Option<Rect>,
    pub browser_hit: Option<Rect>,
    pub progress_hit: Option<Rect>,
    pub visualizer_hit: Option<Rect>,
    pub volume_hit: Option<Rect>,
    pub control_hits: Vec<(Rect, MouseControl)>,
    /// Last list click (pane, full index, `clock_secs`) for double-click detection.
    last_click: Option<(FocusPane, usize, f64)>,
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
        visualizer.set_fft_size(cfg.fft_size);
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
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            filter_query: String::new(),
            filter_pane: FocusPane::Playlist,
            current_meta: TrackMeta::default(),
            current_duration: None,
            status: String::from("Ready"),
            repeat: match cfg.repeat.as_str() {
                "all" => RepeatMode::All,
                "one" => RepeatMode::One,
                _ => RepeatMode::Off,
            },
            shuffle: cfg.shuffle,
            favorites: cfg.favorites.iter().cloned().collect(),
            recent: cfg.recent.clone(),
            play_counts: cfg.play_counts.iter().cloned().collect(),
            search_index: Vec::new(),
            search_index_ready: false,
            show_search: false,
            search_query: String::new(),
            search_selected: 0,
            search_scope: SearchScope::All,
            av_offset_secs: initial_av_offset,
            audio_output_latency_secs,
            video_render_ewma_secs: 0.0,
            auto_av_offset: true,
            video_clock_secs: 0.0,
            video_clock_wall: 0.0,
            video_clock_init: false,
            subtitles: SubtitleSet::default(),
            active_subtitle_track: None,
            current_subtitle_text: None,
            subtitle_announcement_until: None,
            last_subtitle_track_count: 0,
            preferred_subtitle_lang: None,
            preferred_subtitle_applied: false,
            should_quit: false,
            show_help: false,
            help_scroll: 0,
            show_escape_menu: false,
            escape_menu_selected: 0,
            fullscreen: FullscreenMode::Off,
            url_open_supported: false,
            pending_url_open: None,
            theme,
            truecolor: true,
            clock_secs: 0.0,
            last_video_rect: None,
            last_art_rect: None,
            last_browser_rect: None,
            playlist_hit: None,
            browser_hit: None,
            progress_hit: None,
            visualizer_hit: None,
            volume_hit: None,
            control_hits: Vec::new(),
            last_click: None,
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

    /// Indices into a pane's full list that pass the active filter. When no
    /// filter applies to `pane`, this is simply every index in order. The
    /// `selected`/`browser_selected` cursors always index the *full* lists; this
    /// is the set navigation is constrained to and the panels render.
    pub fn visible_indices(&self, pane: FocusPane) -> Vec<usize> {
        let q = if pane == self.filter_pane {
            self.filter_query.as_str()
        } else {
            ""
        };
        match pane {
            FocusPane::Playlist => {
                filtered_indices(self.tracks.iter().map(|t| t.display.as_str()), q)
            }
            FocusPane::Browser => {
                let names: Vec<String> =
                    self.browser_entries.iter().map(|p| short_name(p)).collect();
                filtered_indices(names.iter().map(|s| s.as_str()), q)
            }
        }
    }

    fn selection(&self, pane: FocusPane) -> usize {
        match pane {
            FocusPane::Playlist => self.selected,
            FocusPane::Browser => self.browser_selected,
        }
    }

    fn set_selection(&mut self, pane: FocusPane, idx: usize) {
        match pane {
            FocusPane::Playlist => {
                self.selected = idx;
                self.playlist_state.select(Some(idx));
            }
            FocusPane::Browser => {
                self.browser_selected = idx;
                self.browser_state.select(Some(idx));
            }
        }
    }

    /// Move the cursor by `delta` over the focused pane's *visible* entries,
    /// wrapping around and skipping rows hidden by the filter.
    pub fn move_selection(&mut self, delta: i32) {
        let pane = self.focus;
        let vis = self.visible_indices(pane);
        if vis.is_empty() {
            return;
        }
        let cur = self.selection(pane);
        let pos = vis.iter().position(|&i| i == cur).unwrap_or(0);
        let new_pos = (pos as i32 + delta).rem_euclid(vis.len() as i32) as usize;
        self.set_selection(pane, vis[new_pos]);
    }

    pub fn page(&mut self, dir: i32) {
        self.move_selection(dir * 10);
    }

    pub fn select_first(&mut self) {
        let pane = self.focus;
        if let Some(&first) = self.visible_indices(pane).first() {
            self.set_selection(pane, first);
        }
    }

    pub fn select_last(&mut self) {
        let pane = self.focus;
        if let Some(&last) = self.visible_indices(pane).last() {
            self.set_selection(pane, last);
        }
    }

    /// Keep the cursor on a visible row after the filter changes.
    fn snap_selection_to_visible(&mut self) {
        let pane = self.filter_pane;
        let vis = self.visible_indices(pane);
        if vis.is_empty() {
            return;
        }
        if !vis.contains(&self.selection(pane)) {
            self.set_selection(pane, vis[0]);
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
                self.note_play(&source);
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

    /// Advance the smoothed video clock toward `raw_audio_secs` and return its
    /// new value. See `video_clock_secs` for the rationale. Free-runs forward
    /// by real wall-clock time, then corrects a small fraction of the residual
    /// error toward the (bursty) audio truth; large discontinuities snap.
    fn smooth_video_clock(&mut self, raw_audio_secs: f64) -> f64 {
        let wall = self.clock_secs;
        if !self.video_clock_init {
            self.video_clock_secs = raw_audio_secs;
            self.video_clock_wall = wall;
            self.video_clock_init = true;
            return self.video_clock_secs;
        }
        let dt = wall - self.video_clock_wall;
        self.video_clock_wall = wall;
        // Snap (don't slew) on a backward/oversized gap (platform clock jump,
        // pause/resume, long stall) or any large discontinuity (a seek or track
        // change). Everything else is the normal staircase, which we filter.
        if dt < 0.0
            || dt > VIDEO_CLOCK_STALL_SECS
            || (raw_audio_secs - self.video_clock_secs).abs() > VIDEO_CLOCK_RESYNC_SECS
        {
            self.video_clock_secs = raw_audio_secs;
            return self.video_clock_secs;
        }
        // Run forward in real time, then correct toward audio with a slow,
        // time-constant-based gain. Because the raw clock is centered on true
        // playback, filtering its staircase introduces no net A/V lag; `alpha`
        // is derived from `dt` so the time constant — and thus the smoothing —
        // is independent of how often this is called.
        self.video_clock_secs += dt;
        let err = raw_audio_secs - self.video_clock_secs;
        let alpha = 1.0 - (-dt / VIDEO_CLOCK_SMOOTH_TAU_SECS).exp();
        self.video_clock_secs += err * alpha;
        self.video_clock_secs
    }

    /// Force the smoothed video clock to re-seed from the audio position on the
    /// next tick. Call after seeks/track changes so the clock snaps rather than
    /// slewing across the discontinuity.
    fn reset_video_clock(&mut self) {
        self.video_clock_init = false;
    }

    /// Drive video display: select the current subtitle cue and hand the
    /// display position to the video backend, then fold the backend's reported
    /// render time into the auto A/V offset (native only; web returns `None`).
    pub fn tick_video(&mut self) {
        if !self.video.is_loaded() {
            return;
        }
        let raw = self.position().as_secs_f64();
        let duration = self.current_duration.map(|d| d.as_secs_f64());
        if self.audio.is_paused() {
            // Keep the window's clock fed (paused, so it holds the frame) and
            // its OSD progress bar accurate even while we don't advance video.
            self.video
                .publish_clock((raw - self.av_offset_secs).max(0.0), true, duration);
            return;
        }
        let pos = self.smooth_video_clock(raw) - self.av_offset_secs;
        self.video.publish_clock(pos.max(0.0), false, duration);
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
                self.reset_video_clock();
                if self.auto_av_offset {
                    self.av_offset_secs = baseline_av_offset(self.audio_output_latency_secs);
                    self.video_render_ewma_secs = 0.0;
                }
                self.status = format!("Seek: {} ({:+.0}s)", fmt_short(pos), delta);
                // Flash the fullscreen OSD (progress bar + time) on seek.
                self.video.show_osd(None);
            }
            Err(e) => self.status = format!("Seek error: {e}"),
        }
    }

    pub fn queue_selected_browser(&mut self) {
        // The first entry is the ".." parent shortcut (mirrors the browser UI).
        // Queuing it would recursively scan the parent tree — near the
        // filesystem root that walks /proc, /sys, mounts, etc. and hangs — so
        // refuse it; navigation (Enter) is the way to move up.
        if self.browser_selected == 0 && self.browser_dir.parent().is_some() {
            self.status = String::from("Can't queue the \"..\" entry");
            return;
        }
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

    // --- Search filter / text input ---------------------------------------

    /// Begin filtering the focused pane (the `/` key).
    pub fn start_filter(&mut self) {
        self.filter_pane = self.focus;
        self.filter_query.clear();
        self.input_mode = InputMode::Filter;
    }

    /// Begin the "save playlist as" prompt (the `w` key).
    pub fn start_save_playlist(&mut self) {
        if self.tracks.is_empty() {
            self.status = String::from("Playlist is empty — nothing to save");
            return;
        }
        self.input_buffer = String::from("playlist");
        self.input_mode = InputMode::SavePlaylist;
    }

    /// Append a typed character to the active text buffer.
    pub fn input_push(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        match self.input_mode {
            InputMode::Filter => {
                self.filter_query.push(c);
                self.snap_selection_to_visible();
            }
            InputMode::SavePlaylist => self.input_buffer.push(c),
            InputMode::Normal => {}
        }
    }

    /// Delete the last character of the active text buffer (Backspace).
    pub fn input_backspace(&mut self) {
        match self.input_mode {
            InputMode::Filter => {
                self.filter_query.pop();
                self.snap_selection_to_visible();
            }
            InputMode::SavePlaylist => {
                self.input_buffer.pop();
            }
            InputMode::Normal => {}
        }
    }

    /// Cancel text input (Esc). For a filter this also clears the query.
    pub fn cancel_input(&mut self) {
        if self.input_mode == InputMode::Filter {
            self.filter_query.clear();
        }
        self.input_buffer.clear();
        self.input_mode = InputMode::Normal;
    }

    /// Commit text input (Enter). A filter stays applied but leaves typing mode;
    /// a save prompt writes the playlist.
    pub fn confirm_input(&mut self) {
        match self.input_mode {
            InputMode::Filter => self.input_mode = InputMode::Normal,
            InputMode::SavePlaylist => {
                let name = self.input_buffer.clone();
                self.input_mode = InputMode::Normal;
                self.input_buffer.clear();
                self.export_playlist(&name);
            }
            InputMode::Normal => {}
        }
    }

    /// Write the current playlist to `<browser_dir>/<name>.m3u`, then refresh the
    /// browser so the new file shows up as visible confirmation.
    fn export_playlist(&mut self, name: &str) {
        let name = name.trim();
        let name = if name.is_empty() { "playlist" } else { name };
        let file = self.browser_dir.join(format!("{name}.m3u"));
        match self.library.save_playlist(&file, &self.tracks) {
            Ok(()) => {
                self.status = format!("Saved playlist: {}", file.display());
                self.refresh_browser();
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    // --- Playlist editing --------------------------------------------------

    /// Remove the highlighted playlist track. If it was playing, playback stops.
    pub fn remove_selected_track(&mut self) {
        if self.focus != FocusPane::Playlist || self.tracks.is_empty() {
            return;
        }
        let idx = self.selected.min(self.tracks.len() - 1);
        match self.playing_index {
            Some(p) if p == idx => {
                self.audio.stop();
                self.video.close();
                self.playing_index = None;
                self.playing_track = None;
                self.current_meta = TrackMeta::default();
                self.current_duration = None;
                self.art.set_artwork(None, None);
            }
            Some(p) if p > idx => self.playing_index = Some(p - 1),
            _ => {}
        }
        self.tracks.remove(idx);
        if self.selected >= self.tracks.len() {
            self.selected = self.tracks.len().saturating_sub(1);
        }
        self.refresh_playlist_state();
        self.status = String::from("Removed track");
    }

    /// Move the highlighted playlist track up (`delta = -1`) or down (`+1`),
    /// keeping the cursor and the playing marker on it. Disabled while a playlist
    /// filter is active (neighbours in the full list may be hidden).
    pub fn move_selected_track(&mut self, delta: i32) {
        if self.focus != FocusPane::Playlist {
            return;
        }
        if self.filter_pane == FocusPane::Playlist && !self.filter_query.trim().is_empty() {
            return;
        }
        let n = self.tracks.len();
        if n < 2 {
            return;
        }
        let from = self.selected.min(n - 1);
        let to = from as i32 + delta;
        if to < 0 || to as usize >= n {
            return;
        }
        let to = to as usize;
        self.tracks.swap(from, to);
        self.playing_index = self.playing_index.map(|p| {
            if p == from {
                to
            } else if p == to {
                from
            } else {
                p
            }
        });
        self.selected = to;
        self.playlist_state.select(Some(to));
    }

    // --- Session persistence ----------------------------------------------

    /// Snapshot the full persistable state (settings + resumable session).
    fn current_config(&self) -> Config {
        let playlist = self
            .tracks
            .iter()
            .filter_map(|t| match &t.source {
                TrackRef::Path(p) => Some(p.to_string_lossy().to_string()),
                TrackRef::Url(..) => None,
            })
            .collect();
        Config {
            theme: self.theme.name.to_string(),
            volume: self.audio.volume(),
            visualizer: self.visualizer.mode.name().to_string(),
            fft_size: self.visualizer.fft_size(),
            last_dir: Some(self.browser_dir.to_string_lossy().to_string()),
            repeat: self.repeat.label().to_ascii_lowercase(),
            shuffle: self.shuffle,
            playlist,
            playing_index: self.playing_index,
            position_secs: self.audio.position().as_secs_f64(),
            favorites: self.favorites.iter().cloned().collect(),
            recent: self.recent.clone(),
            play_counts: self
                .play_counts
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        }
    }

    /// Persist everything (called on quit and on settings changes).
    pub fn save_session(&self) {
        self.config.save(&self.current_config());
    }

    /// Seek the current track to an absolute position (used to resume a session).
    pub fn seek_to_secs(&mut self, secs: f64) {
        let cur = self.audio.position().as_secs_f64();
        self.seek_seconds(secs - cur);
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
        self.video.show_osd(Some(self.status.clone()));
    }

    /// Step the active audio track by ±1, wrapping around. Audio tracks (unlike
    /// subtitles) have no "off" slot — there is always one playing.
    pub fn step_audio_track(&mut self, delta: i32) {
        let tracks = self.audio.audio_tracks();
        if tracks.len() < 2 {
            return;
        }
        let n = tracks.len() as i32;
        let current = self.audio.active_audio_track().unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(n) as usize;
        match self.audio.set_audio_track(next) {
            Ok(()) => {
                self.status = format!(
                    "Audio track: {}",
                    tracks
                        .get(next)
                        .cloned()
                        .unwrap_or_else(|| format!("Track {}", next + 1))
                );
            }
            Err(e) => self.status = format!("Audio track switch failed: {e}"),
        }
        // Surface the change on the fullscreen OSD when a video window is up.
        self.video.show_osd(Some(self.status.clone()));
    }

    /// Advance to the next audio track (the `b` key), mirroring `c` for subtitles.
    pub fn cycle_audio_track(&mut self) {
        if self.audio.audio_tracks().len() < 2 {
            self.status = String::from("Only one audio track");
            return;
        }
        self.step_audio_track(1);
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

    /// Step the spectrum FFT window to the next/previous supported size. Larger
    /// windows give finer frequency resolution (esp. bass) but slower response.
    pub fn step_fft_size(&mut self, delta: i32) {
        let sizes = crate::visualizer::FFT_SIZES;
        let cur = self.visualizer.fft_size();
        let pos = sizes.iter().position(|&s| s == cur).unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, sizes.len() as i32 - 1) as usize;
        let size = sizes[next];
        self.visualizer.set_fft_size(size);
        let hz = self.audio.tap().sample_rate() as f64 / size as f64;
        self.status = format!("FFT window: {size} samples ({hz:.1} Hz/bin)");
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
        self.video.show_osd(Some(self.status.clone()));
    }

    pub fn reset_av_offset_auto(&mut self) {
        self.auto_av_offset = true;
        self.video_render_ewma_secs = 0.0;
        self.av_offset_secs = baseline_av_offset(self.audio_output_latency_secs);
        self.status = String::from("A/V offset: auto");
    }

    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = if self.fullscreen.is_on() {
            FullscreenMode::Off
        } else {
            FullscreenMode::Visualizer
        };
    }

    /// Cycle the display mode with `f`.
    ///
    /// With a video loaded: Normal → in-app Fullscreen → external Video Window
    /// → Normal (the window step is skipped when the backend doesn't support
    /// one, e.g. web).
    ///
    /// For music: Normal → fullscreen Visualizer → fullscreen Album Art →
    /// fullscreen Album Art + Visualizer → Normal. The two album-art steps are
    /// skipped when the track has no artwork, so it collapses to Normal ↔
    /// fullscreen Visualizer.
    pub fn cycle_display_mode(&mut self) {
        if self.video.is_loaded() {
            if self.video.external_window_enabled() {
                self.video.set_external_window(false);
                self.fullscreen = FullscreenMode::Off;
                self.status = String::from("Display: normal");
            } else if self.fullscreen.is_on() {
                self.fullscreen = FullscreenMode::Off;
                if self.video.supports_external_window() {
                    self.video.set_external_window(true);
                    self.status = String::from("Display: video window");
                } else {
                    self.status = String::from("Display: normal");
                }
            } else {
                self.fullscreen = FullscreenMode::Visualizer;
                self.status = String::from("Display: fullscreen");
            }
            return;
        }

        // Music path.
        let has_art = self.art.has_art();
        self.fullscreen = match self.fullscreen {
            FullscreenMode::Off => FullscreenMode::Visualizer,
            FullscreenMode::Visualizer if has_art => FullscreenMode::AlbumArt,
            FullscreenMode::Visualizer => FullscreenMode::Off,
            FullscreenMode::AlbumArt => FullscreenMode::AlbumArtVis,
            FullscreenMode::AlbumArtVis => FullscreenMode::Off,
        };
        self.status = String::from(match self.fullscreen {
            FullscreenMode::Off => "Display: normal",
            FullscreenMode::Visualizer => "Display: fullscreen visualizer",
            FullscreenMode::AlbumArt => "Display: fullscreen album art",
            FullscreenMode::AlbumArtVis => "Display: fullscreen album art + visualizer",
        });
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
        let audio_tracks = self.audio.audio_tracks();
        let audio_label = if !has_video || audio_tracks.len() < 2 {
            "—".to_string()
        } else {
            let cur = self.audio.active_audio_track().unwrap_or(0);
            audio_tracks
                .get(cur)
                .cloned()
                .unwrap_or_else(|| format!("Track {}", cur + 1))
        };
        let mut items = vec![
            EscapeMenuItem {
                kind: EscapeMenuKind::Volume,
                enabled: true,
                label: "Volume",
                value: format!("{:>3.0}%", self.audio.volume() * 100.0),
            },
            EscapeMenuItem {
                kind: EscapeMenuKind::AudioTrack,
                enabled: has_video && audio_tracks.len() > 1,
                label: "Audio Track",
                value: audio_label,
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
                kind: EscapeMenuKind::FftSize,
                enabled: true,
                label: "FFT Resolution",
                value: format!("{} samples", self.visualizer.fft_size()),
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
                value: self.fullscreen.label().to_string(),
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
        ];
        items.push(EscapeMenuItem {
            kind: EscapeMenuKind::Help,
            enabled: true,
            label: "Help",
            value: "View ↗".to_string(),
        });
        if self.url_open_supported {
            items.push(EscapeMenuItem {
                kind: EscapeMenuKind::Github,
                enabled: true,
                label: "GitHub",
                value: "Open ↗".to_string(),
            });
        }
        items.push(EscapeMenuItem {
            kind: EscapeMenuKind::Separator,
            enabled: false,
            label: "",
            value: String::new(),
        });
        items.push(EscapeMenuItem {
            kind: EscapeMenuKind::Quit,
            enabled: true,
            label: "Quit",
            value: String::new(),
        });
        items
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
            EscapeMenuKind::AudioTrack => self.step_audio_track(delta),
            EscapeMenuKind::Subtitle => self.step_subtitle_track(delta),
            EscapeMenuKind::AvOffset => self.adjust_av_offset(AV_OFFSET_STEP_SECS * delta as f64),
            EscapeMenuKind::Visualizer => {
                if delta > 0 {
                    self.cycle_visualizer();
                } else {
                    self.cycle_visualizer_back();
                }
            }
            EscapeMenuKind::FftSize => self.step_fft_size(delta),
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
            EscapeMenuKind::Help
            | EscapeMenuKind::Github
            | EscapeMenuKind::Quit
            | EscapeMenuKind::Separator => {}
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
            EscapeMenuKind::Github => {
                self.pending_url_open = Some(GITHUB_URL.to_string());
                return Ok(true);
            }
            EscapeMenuKind::Help => {
                self.show_help = true;
                self.help_scroll = 0;
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
        self.config.save(&self.current_config());
    }

    // --- Favorites / play counts / recently-played ------------------------

    /// Record that `source` just started playing: bump its play count and move
    /// it to the front of the recently-played list.
    fn note_play(&mut self, source: &TrackRef) {
        let key = source.locator();
        *self.play_counts.entry(key.clone()).or_insert(0) += 1;
        self.recent.retain(|p| p != &key);
        self.recent.insert(0, key);
        self.recent.truncate(crate::config::RECENT_CAP);
    }

    /// Whether `source` is favorited.
    pub fn is_favorite(&self, source: &TrackRef) -> bool {
        self.favorites.contains(&source.locator())
    }

    /// The track `F` should favorite: the focused selection (a playlist track or
    /// a browsed audio file), falling back to whatever is playing.
    fn favorite_target(&self) -> Option<String> {
        let focused = match self.focus {
            FocusPane::Playlist => self.tracks.get(self.selected).map(|t| t.source.locator()),
            FocusPane::Browser => self
                .browser_entries
                .get(self.browser_selected)
                .filter(|p| p.is_file() && library::is_audio_file(p))
                .map(|p| p.to_string_lossy().to_string()),
        };
        focused.or_else(|| self.playing_track.as_ref().map(|s| s.locator()))
    }

    /// Toggle the favorite flag on the focused/playing track and persist.
    pub fn toggle_favorite(&mut self) {
        let Some(key) = self.favorite_target() else {
            self.status = String::from("No track to favorite");
            return;
        };
        if self.favorites.remove(&key) {
            self.status = String::from("Removed from favorites");
        } else {
            self.favorites.insert(key);
            self.status = String::from("★ Added to favorites");
        }
        self.save_config();
    }

    // --- Global library search overlay ------------------------------------

    /// Replace the whole-library search index (called by the platform once its
    /// background scan finishes). Keeps the cursor in range.
    pub fn set_search_index(&mut self, tracks: Vec<Track>) {
        self.search_index = tracks;
        self.search_index_ready = true;
        self.clamp_search_selection();
    }

    pub fn open_search(&mut self) {
        self.show_search = true;
        self.search_query.clear();
        self.search_selected = 0;
    }

    pub fn close_search(&mut self) {
        self.show_search = false;
    }

    pub fn search_input_push(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.search_query.push(c);
        self.search_selected = 0;
    }

    pub fn search_input_backspace(&mut self) {
        self.search_query.pop();
        self.search_selected = 0;
    }

    pub fn search_cycle_scope(&mut self) {
        self.search_scope = self.search_scope.cycle();
        self.search_selected = 0;
    }

    pub fn search_move(&mut self, delta: i32) {
        let len = self.search_results().len();
        if len == 0 {
            self.search_selected = 0;
            return;
        }
        let cur = self.search_selected.min(len - 1) as i32;
        self.search_selected = (cur + delta).rem_euclid(len as i32) as usize;
    }

    fn clamp_search_selection(&mut self) {
        let len = self.search_results().len();
        if len == 0 {
            self.search_selected = 0;
        } else if self.search_selected >= len {
            self.search_selected = len - 1;
        }
    }

    /// Indices into `search_index` matching the current scope + query, ordered
    /// for display (recency for Recent, descending count for Most Played).
    pub fn search_results(&self) -> Vec<usize> {
        let q = self.search_query.trim().to_ascii_lowercase();
        let mut idxs: Vec<usize> = self
            .search_index
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                let loc = t.source.locator();
                match self.search_scope {
                    SearchScope::All => true,
                    SearchScope::Favorites => self.favorites.contains(&loc),
                    SearchScope::Recent => self.recent.contains(&loc),
                    SearchScope::MostPlayed => {
                        self.play_counts.get(&loc).copied().unwrap_or(0) > 0
                    }
                }
            })
            .filter(|(_, t)| q.is_empty() || fuzzy_match(&t.display, &q))
            .map(|(i, _)| i)
            .collect();
        match self.search_scope {
            SearchScope::Recent => idxs.sort_by_key(|&i| {
                let loc = self.search_index[i].source.locator();
                self.recent.iter().position(|p| *p == loc).unwrap_or(usize::MAX)
            }),
            SearchScope::MostPlayed => idxs.sort_by(|&a, &b| {
                let ca = self
                    .play_counts
                    .get(&self.search_index[a].source.locator())
                    .copied()
                    .unwrap_or(0);
                let cb = self
                    .play_counts
                    .get(&self.search_index[b].source.locator())
                    .copied()
                    .unwrap_or(0);
                cb.cmp(&ca)
            }),
            _ => {}
        }
        idxs
    }

    /// Play count for a track, for display.
    pub fn play_count(&self, source: &TrackRef) -> u32 {
        self.play_counts.get(&source.locator()).copied().unwrap_or(0)
    }

    /// Append the highlighted search result to the playlist and play it.
    pub fn activate_search(&mut self) -> Result<()> {
        let results = self.search_results();
        let Some(&idx) = results.get(self.search_selected) else {
            return Ok(());
        };
        let track = self.search_index[idx].clone();
        self.tracks.push(track);
        let new_idx = self.tracks.len() - 1;
        self.selected = new_idx;
        self.focus = FocusPane::Playlist;
        self.refresh_playlist_state();
        self.close_search();
        self.play_index(new_idx)
    }

    pub fn position(&self) -> Duration {
        self.audio.position()
    }

    /// Take any URL queued for the platform to open (e.g. the GitHub entry).
    pub fn take_pending_url_open(&mut self) -> Option<String> {
        self.pending_url_open.take()
    }

    /// Seek to `frac` (0..1) of the current track's duration, absolute.
    pub fn seek_to_fraction(&mut self, frac: f64) {
        let Some(dur) = self.current_duration else {
            return;
        };
        if self.playing_index.is_none() {
            return;
        }
        let target = frac.clamp(0.0, 1.0) * dur.as_secs_f64();
        let cur = self.position().as_secs_f64();
        self.seek_seconds(target - cur);
    }

    /// Dispatch a platform-neutral mouse event. Hit-tests against the rects the
    /// UI recorded during the last draw. Native only for now (the web build
    /// uses native DOM controls); harmless if the rects are unset.
    pub fn handle_mouse(&mut self, ev: CoreMouseEvent) -> Result<()> {
        // Modal overlays and text entry own the screen — ignore the mouse.
        if self.show_help
            || self.show_escape_menu
            || self.show_search
            || self.input_mode != InputMode::Normal
        {
            return Ok(());
        }
        let (col, row) = (ev.col, ev.row);
        match ev.kind {
            CoreMouseKind::ScrollUp | CoreMouseKind::ScrollDown => {
                let delta = if ev.kind == CoreMouseKind::ScrollUp { -3 } else { 3 };
                if rect_hit(self.playlist_hit, col, row) {
                    self.scroll_pane(FocusPane::Playlist, delta);
                } else if rect_hit(self.browser_hit, col, row) {
                    self.scroll_pane(FocusPane::Browser, delta);
                } else if rect_hit(self.visualizer_hit, col, row) {
                    // Scrolling over the visualizer nudges the volume.
                    self.volume_step(if delta < 0 { 0.05 } else { -0.05 });
                }
            }
            CoreMouseKind::Down => {
                // Seek bar takes priority (it can overlap the now-playing panel).
                if rect_hit(self.progress_hit, col, row) {
                    self.seek_to_fraction(self.frac_at(self.progress_hit, col));
                    return Ok(());
                }
                // Clickable control badges.
                let mut control = None;
                for (r, c) in &self.control_hits {
                    if rect_hit(Some(*r), col, row) {
                        control = Some(*c);
                        break;
                    }
                }
                if let Some(c) = control {
                    self.apply_mouse_control(c)?;
                    return Ok(());
                }
                // Volume column: set level from the click height.
                if let Some(r) = self.volume_hit.filter(|r| rect_hit(Some(*r), col, row)) {
                    self.set_volume_abs(self.volume_at_row(r, row), true);
                    return Ok(());
                }
                // Clicking the visualizer switches to the next mode.
                if rect_hit(self.visualizer_hit, col, row) {
                    self.cycle_visualizer();
                    return Ok(());
                }
                if let Some(r) = self.playlist_hit.filter(|r| rect_hit(Some(*r), col, row)) {
                    self.click_list(FocusPane::Playlist, r, row)?;
                    return Ok(());
                }
                if let Some(r) = self.browser_hit.filter(|r| rect_hit(Some(*r), col, row)) {
                    self.click_list(FocusPane::Browser, r, row)?;
                }
            }
            CoreMouseKind::Drag => {
                // Dragging in the volume column tracks the level (column-gated,
                // so it never collides with the seek bar's row-gated scrub).
                if let Some(r) = self
                    .volume_hit
                    .filter(|r| col >= r.x && col < r.x.saturating_add(r.width))
                {
                    self.set_volume_abs(self.volume_at_row(r, row), false);
                    return Ok(());
                }
                // Scrub the seek bar: accept any drag on the bar's row(s).
                if let Some(r) = self
                    .progress_hit
                    .filter(|r| row >= r.y && row < r.y.saturating_add(r.height.max(1)))
                {
                    self.seek_to_fraction(self.frac_at(Some(r), col));
                }
            }
        }
        Ok(())
    }

    /// Fraction (0..1) of a horizontal rect that `col` lands at, clamped.
    fn frac_at(&self, rect: Option<Rect>, col: u16) -> f64 {
        let Some(r) = rect else { return 0.0 };
        if r.width == 0 {
            return 0.0;
        }
        let cx = col.clamp(r.x, r.x + r.width - 1);
        (cx - r.x) as f64 / r.width as f64
    }

    /// Move a pane's selection by `delta` visible rows, clamped (no wrap), for
    /// the scroll wheel. Leaves focus untouched so scrolling doesn't steal it.
    fn scroll_pane(&mut self, pane: FocusPane, delta: i32) {
        let vis = self.visible_indices(pane);
        if vis.is_empty() {
            return;
        }
        let cur = self.selection(pane);
        let pos = vis.iter().position(|&i| i == cur).unwrap_or(0) as i32;
        let new_pos = (pos + delta).clamp(0, vis.len() as i32 - 1) as usize;
        self.set_selection(pane, vis[new_pos]);
    }

    /// Handle a click on a list row: focus the pane and select the row; a
    /// second click on the same row within the double-click window activates it
    /// (play the track / enter the directory).
    fn click_list(&mut self, pane: FocusPane, inner: Rect, row: u16) -> Result<()> {
        let vis = self.visible_indices(pane);
        if vis.is_empty() {
            self.focus = pane;
            return Ok(());
        }
        let offset = match pane {
            FocusPane::Playlist => self.playlist_state.offset(),
            FocusPane::Browser => self.browser_state.offset(),
        };
        let vis_idx = offset + (row - inner.y) as usize;
        self.focus = pane;
        let Some(&full_idx) = vis.get(vis_idx) else {
            return Ok(()); // click below the last row
        };
        self.set_selection(pane, full_idx);
        let now = self.clock_secs;
        let double = matches!(
            self.last_click,
            Some((p, i, t)) if p == pane && i == full_idx && now - t < 0.45
        );
        if double {
            self.last_click = None;
            self.activate_selection()?;
        } else {
            self.last_click = Some((pane, full_idx, now));
        }
        Ok(())
    }

    /// Volume (0..1.5) for a click at `row` in the vertical volume bar `r`:
    /// top of the bar is full, bottom is muted.
    fn volume_at_row(&self, r: Rect, row: u16) -> f32 {
        if r.height <= 1 {
            return self.audio.volume();
        }
        let ry = row.clamp(r.y, r.y + r.height - 1);
        let frac_from_top = (ry - r.y) as f32 / (r.height - 1) as f32;
        ((1.0 - frac_from_top) * 1.5).clamp(0.0, 1.5)
    }

    /// Set the volume to an absolute level. `persist` writes the config (used on
    /// the initial click, but skipped mid-drag to avoid thrashing the file).
    fn set_volume_abs(&mut self, v: f32, persist: bool) {
        let v = v.clamp(0.0, 1.5);
        self.audio.set_volume(v);
        self.status = format!("Volume: {:>3.0}%", v * 100.0);
        if persist {
            self.save_config();
        }
    }

    fn apply_mouse_control(&mut self, control: MouseControl) -> Result<()> {
        match control {
            MouseControl::SeekBack => self.seek_seconds(-10.0),
            MouseControl::Stop => self.stop_playback(),
            MouseControl::PlayPause => {
                // Cassette Play: resume/pause, or start the selection if stopped.
                if self.playing_index.is_none() {
                    if !self.tracks.is_empty() {
                        self.play_index(self.selected)?;
                    }
                } else {
                    self.audio.toggle_pause();
                }
            }
            MouseControl::SeekForward => self.seek_seconds(10.0),
            MouseControl::CycleRepeat => self.cycle_repeat(),
            MouseControl::ToggleShuffle => self.toggle_shuffle(),
            MouseControl::ToggleFavorite => self.toggle_favorite(),
            MouseControl::CycleSubtitle => self.cycle_subtitle_track(),
        }
        Ok(())
    }

    /// Stop playback: halt audio and any video, and clear the "now playing"
    /// state (the selection is kept, so the Play button restarts from it).
    pub fn stop_playback(&mut self) {
        self.audio.stop();
        self.playing_index = None;
        self.playing_track = None;
        if self.video.is_loaded() {
            self.video.close();
        }
        self.status = String::from("Stopped");
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
                CoreKey::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                CoreKey::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                CoreKey::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                CoreKey::PageDown => self.help_scroll = self.help_scroll.saturating_add(10),
                CoreKey::Home => self.help_scroll = 0,
                // Clamped to the content height when the overlay is rendered.
                CoreKey::End => self.help_scroll = u16::MAX,
                _ => {}
            }
            return Ok(());
        }

        if self.input_mode != InputMode::Normal {
            match code {
                CoreKey::Esc => self.cancel_input(),
                CoreKey::Enter => self.confirm_input(),
                CoreKey::Backspace => self.input_backspace(),
                CoreKey::Char(c) => self.input_push(c),
                _ => {}
            }
            return Ok(());
        }

        if self.show_search {
            if matches!(code, CoreKey::Char('c')) && ctrl {
                self.should_quit = true;
                return Ok(());
            }
            match code {
                CoreKey::Esc => self.close_search(),
                CoreKey::Up => self.search_move(-1),
                CoreKey::Down => self.search_move(1),
                CoreKey::PageUp => self.search_move(-10),
                CoreKey::PageDown => self.search_move(10),
                CoreKey::Tab => self.search_cycle_scope(),
                CoreKey::Enter => self.activate_search()?,
                CoreKey::Backspace => self.search_input_backspace(),
                CoreKey::Char(c) => self.search_input_push(c),
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

        // While the fullscreen SDL video window owns the screen, the playlist
        // and file-browser panels are hidden behind it, so suppress every key
        // that would manipulate them (selection, queueing, removing, filtering,
        // search, focus switching). Only transport / playback controls remain.
        if self.video.external_window_enabled() {
            match code {
                CoreKey::Char('q') => self.should_quit = true,
                CoreKey::Char('c') if ctrl => self.should_quit = true,
                CoreKey::Esc | CoreKey::Char('f') => {
                    self.video.set_external_window(false);
                    self.fullscreen = FullscreenMode::Off;
                    self.status = String::from("Exited fullscreen video");
                }
                CoreKey::Char(' ') => self.audio.toggle_pause(),
                CoreKey::Char('n') => self.next_track()?,
                CoreKey::Char('p') => self.prev_track()?,
                CoreKey::Left if ctrl => self.seek_seconds(-30.0),
                CoreKey::Right if ctrl => self.seek_seconds(30.0),
                CoreKey::Left => self.seek_seconds(-10.0),
                CoreKey::Right => self.seek_seconds(10.0),
                CoreKey::Char('-') | CoreKey::Char('_') => self.volume_step(-0.05),
                CoreKey::Char('+') | CoreKey::Char('=') => self.volume_step(0.05),
                CoreKey::Char('b') => self.cycle_audio_track(),
                CoreKey::Char('c') => self.cycle_subtitle_track(),
                CoreKey::Char('[') => self.adjust_av_offset(-AV_OFFSET_STEP_SECS),
                CoreKey::Char(']') => self.adjust_av_offset(AV_OFFSET_STEP_SECS),
                CoreKey::Char('r') => self.cycle_repeat(),
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
            CoreKey::Char('b') => self.cycle_audio_track(),
            CoreKey::Char('?') | CoreKey::Char('h') => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            CoreKey::Char('/') => self.start_filter(),
            CoreKey::Char('g') => self.open_search(),
            CoreKey::Char('F') => self.toggle_favorite(),
            CoreKey::Char('w') => self.start_save_playlist(),
            CoreKey::Char('d') | CoreKey::Delete => self.remove_selected_track(),
            CoreKey::Tab => self.focus_next(),
            CoreKey::Up if ctrl => self.move_selected_track(-1),
            CoreKey::Down if ctrl => self.move_selected_track(1),
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

/// Whether the cell `(col, row)` falls inside `rect` (if set).
fn rect_hit(rect: Option<Rect>, col: u16, row: u16) -> bool {
    match rect {
        Some(r) => {
            col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y
                && row < r.y.saturating_add(r.height)
        }
        None => false,
    }
}

fn plural_s(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn short_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// Indices of `items` whose text contains `query`, case-insensitively. A blank
/// query matches everything (all indices, in order) so an inactive filter is a
/// no-op.
fn filtered_indices<'a>(items: impl Iterator<Item = &'a str>, query: &str) -> Vec<usize> {
    let q = query.trim().to_ascii_lowercase();
    items
        .enumerate()
        .filter(|(_, s)| q.is_empty() || s.to_ascii_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Case-insensitive *subsequence* match: every character of `needle_lower` (which
/// the caller has already lowercased) appears in `haystack` in order, not
/// necessarily contiguously. An empty needle matches everything. Powers the
/// global search overlay's fuzzy "jump to track".
pub fn fuzzy_match(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let mut needle = needle_lower.chars();
    let mut want = needle.next();
    for h in haystack.chars().flat_map(|c| c.to_lowercase()) {
        match want {
            Some(n) if h == n => want = needle.next(),
            Some(_) => {}
            None => break,
        }
    }
    want.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_subsequence_case_insensitively() {
        assert!(fuzzy_match("The Beatles - Help", "beat"));
        assert!(fuzzy_match("The Beatles - Help", "thlp")); // gapped subsequence
        assert!(fuzzy_match("Beethoven", "BTHVN".to_ascii_lowercase().as_str()));
        assert!(fuzzy_match("anything", ""));
        assert!(!fuzzy_match("short", "longer"));
        assert!(!fuzzy_match("abc", "acb")); // order matters
    }

    #[test]
    fn filter_empty_query_matches_all() {
        let items = ["Alpha", "Beta", "Gamma"];
        assert_eq!(filtered_indices(items.iter().copied(), ""), vec![0, 1, 2]);
        assert_eq!(filtered_indices(items.iter().copied(), "   "), vec![0, 1, 2]);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let items = ["The Beatles - Help", "Beethoven", "beat it"];
        assert_eq!(filtered_indices(items.iter().copied(), "beat"), vec![0, 2]);
        assert_eq!(filtered_indices(items.iter().copied(), "BEET"), vec![1]);
        assert!(filtered_indices(items.iter().copied(), "zzz").is_empty());
    }
}
