//! Platform-abstraction traits. `App` is generic over no types — it holds
//! these as boxed trait objects so the same state machine and UI drive both the
//! native (rodio/ffmpeg/SDL/crossterm) and web (Web Audio / `<video>` / ratzilla)
//! builds. The traits are deliberately `!Send`/`!Sync`-friendly: nothing here
//! requires those bounds, because web-sys handles are single-threaded.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::audio_tap::SampleBuffer;
use crate::config::Config;
use crate::library::{Track, TrackRef};
use crate::metadata::TrackMeta;
use crate::subtitles::SubtitleSet;

/// Platform-neutral key event fed to [`crate::app::App::handle_key`]. The
/// native crate maps crossterm `KeyEvent`s into this; the web crate maps
/// ratzilla `KeyEvent`s into it. Keeping it small and backend-free is the key
/// to behavior parity between the two builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKey {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    Enter,
    Esc,
    Backspace,
    Delete,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreKeyEvent {
    pub code: CoreKey,
    pub ctrl: bool,
}

impl CoreKeyEvent {
    pub fn new(code: CoreKey) -> Self {
        Self { code, ctrl: false }
    }
    pub fn with_ctrl(code: CoreKey, ctrl: bool) -> Self {
        Self { code, ctrl }
    }
}

/// Audio playback + the visualizer sample tap. Native wraps rodio; web wraps a
/// Web Audio graph fed from an `HTMLMediaElement`.
pub trait AudioBackend {
    /// Begin playing the given source. Returns a duration hint if known.
    fn play(&mut self, source: &TrackRef) -> anyhow::Result<Option<Duration>>;
    fn toggle_pause(&self);
    fn is_paused(&self) -> bool;
    /// True once the current source has finished (drives auto-advance).
    fn is_finished(&self) -> bool;
    fn stop(&mut self);
    fn set_volume(&mut self, v: f32);
    fn volume(&self) -> f32;
    /// Seek by `delta` seconds relative to the current position, clamped to
    /// `[0, total]` when `total` is known.
    fn seek_relative(&mut self, delta: f64, total: Option<Duration>) -> anyhow::Result<()>;
    fn position(&self) -> Duration;
    /// The shared sample tap the visualizer reads from.
    fn tap(&self) -> &SampleBuffer;
    /// Best-effort audio output latency (CPAL ring on native, ~0 on web).
    fn output_buffer_latency(&self) -> Duration;
    /// Web only: copy the latest analyser samples into the tap. No-op on native
    /// (the tap is filled continuously on the playback thread).
    fn pump(&mut self) {}
    /// Web only: the browser blocks audio until a user gesture. Called on the
    /// first key/click so the backend can resume its `AudioContext`. No-op on
    /// native.
    fn on_user_gesture(&mut self) {}
    /// Total duration if the backend can report it (web learns this only after
    /// the media element loads its metadata). `None` means "unknown".
    fn duration(&self) -> Option<Duration> {
        None
    }
    /// Human labels for the audio tracks in the current file, in selection
    /// order. More than one entry only for multi-audio containers (some
    /// MKV/MP4). Empty or a single entry means there is nothing to switch.
    /// Default: none (single-stream backends, web).
    fn audio_tracks(&self) -> Vec<String> {
        Vec::new()
    }
    /// Index (into [`audio_tracks`](Self::audio_tracks)) of the audio track
    /// currently playing, or `None` when the file has no enumerated tracks.
    fn active_audio_track(&self) -> Option<usize> {
        None
    }
    /// Switch to audio track `idx`, preserving playback position and pause
    /// state. No-op for an invalid index or a backend without track support.
    fn set_audio_track(&mut self, _idx: usize) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Video playback. Native decodes frames with ffmpeg and renders them in the
/// terminal (or to an SDL window); web positions a real `<video>` overlay.
pub trait VideoBackend {
    /// Open a source for video playback. Returns the pixel dimensions if the
    /// source actually has a video stream, else `None` (audio-only).
    fn open(&mut self, source: &TrackRef) -> Option<(u32, u32)>;
    /// Tear down the current video (called on track change / clear).
    fn close(&mut self);
    /// Whether a video stream is loaded (drives the A/V badge, escape menu).
    fn is_loaded(&self) -> bool;
    /// Whether there is a renderable video image *in the terminal layout* right
    /// now. On native this is false until the first frame is decoded and false
    /// while the external SDL window owns the picture; on web it tracks the
    /// `<video>` element being active.
    fn has_image(&self) -> bool;
    fn seek(&self, target: Duration);
    /// Advance to `display_pos` seconds (audio position minus A/V offset),
    /// updating the current subtitle overlay. Returns the time spent preparing
    /// the frame in seconds (native, for the A/V EWMA) or `None`.
    fn advance(&mut self, display_pos: f64, paused: bool, subtitle: Option<&str>) -> Option<f64>;
    /// Draw the current video frame into `area` (native) or reposition the
    /// overlay element to match `area` (web).
    fn render(&mut self, frame: &mut Frame, area: Rect);
    /// Whether this backend supports a dedicated external playback window
    /// (native SDL: true; web: false, so the menu row is disabled).
    fn supports_external_window(&self) -> bool {
        false
    }
    fn external_window_enabled(&self) -> bool {
        false
    }
    /// Arm/disarm (and on native, spawn/close) the dedicated playback window.
    fn set_external_window(&mut self, _enabled: bool) {}
    /// Drain keys forwarded from the external window (native only).
    fn drain_external_keys(&self) -> Vec<CoreKeyEvent> {
        Vec::new()
    }
}

/// Resolves track sources to playlists, metadata, cover art and subtitles, and
/// backs the file-browser pane. Native hits the filesystem; web reads the
/// manifest (and returns empty browser entries).
pub trait MediaLibrary {
    /// List browsable entries in `dir` (parent first, then dirs, then files).
    /// Web returns an empty list (no filesystem browser in the browser).
    fn browse(&self, dir: &Path) -> Vec<PathBuf>;
    /// Expand a playlist/folder source into tracks.
    fn load_playlist(&self, source: &TrackRef) -> anyhow::Result<Vec<Track>>;
    /// Recursively collect audio tracks under a directory (native only).
    fn scan_directory(&self, dir: &Path) -> Vec<Track>;
    fn read_metadata(&self, source: &TrackRef) -> TrackMeta;
    fn find_cover(&self, source: &TrackRef) -> Option<Vec<u8>>;
    fn load_subtitles(&self, source: &TrackRef) -> SubtitleSet;
    /// Write `tracks` to an M3U playlist file at `path`. Default: unsupported
    /// (web has no filesystem to write to).
    fn save_playlist(&self, _path: &Path, _tracks: &[Track]) -> anyhow::Result<()> {
        anyhow::bail!("saving playlists is not supported on this platform")
    }
}

/// Persisted settings storage. Native = config file; web = `localStorage`.
pub trait ConfigStore {
    fn load(&self) -> Config;
    fn save(&self, cfg: &Config);
}

/// Album-art rendering. Native decodes the bytes and paints them via
/// ratatui-image; web positions an `<img>` overlay built from the bytes.
pub trait AlbumArtRenderer {
    /// Set (or clear) the current artwork. `key` lets the renderer skip
    /// redundant re-decodes; it changes whenever the displayed art should.
    fn set_artwork(&mut self, bytes: Option<&[u8]>, key: Option<(usize, usize)>);
    fn has_art(&self) -> bool;
    /// Draw the art into `area` (native) or reposition the `<img>` overlay (web).
    fn render(&mut self, frame: &mut Frame, area: Rect);
}
