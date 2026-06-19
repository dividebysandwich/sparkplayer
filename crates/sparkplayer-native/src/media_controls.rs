//! OS media integration via [`souvlaki`]: exposes SparkPlayer to the desktop's
//! now-playing widget and hardware media keys (Linux MPRIS over D-Bus, macOS
//! `MPNowPlayingInfoCenter`, Windows SMTC). Control events the OS sends back
//! (play/pause, next, seek, …) are funneled through an `mpsc` channel and
//! drained by the run loop, mirroring how the SDL external window forwards keys.
//!
//! This is best-effort: if the platform backend can't initialize (e.g. no D-Bus
//! session), `MediaOs::new` returns `Err` and the caller simply runs without it.
//!
//! macOS caveat: full lock-screen now-playing is most reliable from a bundled
//! `.app`; media-key control works from a plain binary but the run loop must be
//! pumped. We target Linux MPRIS as the primary path and treat macOS as
//! best-effort.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

use sparkplayer_core::App;

/// A control request originating from the OS (media key, desktop widget, …).
#[derive(Debug, Clone)]
pub enum MediaCommand {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    Stop,
    SeekForward,
    SeekBack,
    SetPosition(Duration),
    Quit,
}

/// Identity of the currently-published track; used to avoid re-pushing metadata
/// every frame. Cheap to compute and compare.
type MetaKey = (Option<usize>, String, String);

pub struct MediaOs {
    controls: MediaControls,
    rx: Receiver<MediaCommand>,
    last_meta: Option<MetaKey>,
    /// Last playback state pushed: `None` = stopped, `Some(true/false)` =
    /// paused/playing. Used to push only on change (souvlaki's service thread
    /// processes events on a channel — flooding it every frame backs it up and
    /// stalls shutdown, which joins that thread).
    last_state: Option<Option<bool>>,
    /// `clock_secs` at the last push, for a coarse position refresh.
    last_push_secs: f64,
    cover_path: Option<PathBuf>,
}

impl MediaOs {
    pub fn new() -> Result<Self, souvlaki::Error> {
        let config = PlatformConfig {
            dbus_name: "sparkplayer",
            display_name: "SparkPlayer",
            // Windows wants the console/window handle here; harmless elsewhere.
            hwnd: None,
        };
        let mut controls = MediaControls::new(config)?;

        let (tx, rx): (Sender<MediaCommand>, Receiver<MediaCommand>) = mpsc::channel();
        controls.attach(move |event: MediaControlEvent| {
            let cmd = match event {
                MediaControlEvent::Toggle => Some(MediaCommand::Toggle),
                MediaControlEvent::Play => Some(MediaCommand::Play),
                MediaControlEvent::Pause => Some(MediaCommand::Pause),
                MediaControlEvent::Next => Some(MediaCommand::Next),
                MediaControlEvent::Previous => Some(MediaCommand::Prev),
                MediaControlEvent::Stop => Some(MediaCommand::Stop),
                MediaControlEvent::Seek(SeekDirection::Forward) => Some(MediaCommand::SeekForward),
                MediaControlEvent::Seek(SeekDirection::Backward) => Some(MediaCommand::SeekBack),
                MediaControlEvent::SeekBy(SeekDirection::Forward, _) => {
                    Some(MediaCommand::SeekForward)
                }
                MediaControlEvent::SeekBy(SeekDirection::Backward, _) => {
                    Some(MediaCommand::SeekBack)
                }
                MediaControlEvent::SetPosition(MediaPosition(pos)) => {
                    Some(MediaCommand::SetPosition(pos))
                }
                MediaControlEvent::Quit => Some(MediaCommand::Quit),
                _ => None,
            };
            if let Some(cmd) = cmd {
                let _ = tx.send(cmd);
            }
        })?;

        Ok(Self {
            controls,
            rx,
            last_meta: None,
            last_state: None,
            last_push_secs: 0.0,
            cover_path: None,
        })
    }

    /// Drain control requests the OS has queued since the last call.
    pub fn poll(&mut self) -> Vec<MediaCommand> {
        self.rx.try_iter().collect()
    }

    /// Push the current track metadata and playback state to the OS.
    ///
    /// Both are sent sparingly — metadata only when the track changes, playback
    /// only when the play/pause/stop state changes plus a coarse ~1s position
    /// refresh. souvlaki's service thread drains these from a channel and
    /// shutdown joins it, so pushing every frame would back the channel up and
    /// stall (or hang) exit.
    pub fn sync(&mut self, app: &App) {
        let meta = &app.current_meta;
        let title = meta.title.clone().unwrap_or_else(|| {
            app.playing_index
                .and_then(|i| app.tracks.get(i))
                .map(|t| t.display.clone())
                .unwrap_or_default()
        });
        let artist = meta.artist.clone().unwrap_or_default();

        let key: MetaKey = (app.playing_index, title.clone(), artist.clone());
        let meta_changed = app.playing_index.is_some() && self.last_meta.as_ref() != Some(&key);
        if meta_changed {
            let cover_url = self.write_cover(meta.artwork.as_deref());
            let album = meta.album.clone().unwrap_or_default();
            let _ = self.controls.set_metadata(MediaMetadata {
                title: Some(title.as_str()),
                artist: (!artist.is_empty()).then_some(artist.as_str()),
                album: (!album.is_empty()).then_some(album.as_str()),
                cover_url: cover_url.as_deref(),
                duration: app.current_duration,
            });
            self.last_meta = Some(key);
        }

        let state = if app.playing_index.is_none() {
            None
        } else {
            Some(app.audio.is_paused())
        };
        let now = app.clock_secs;
        let state_changed = self.last_state != Some(state);
        // Refresh position roughly once a second while actively playing.
        let periodic = matches!(state, Some(false)) && (now - self.last_push_secs) >= 1.0;
        if !(meta_changed || state_changed || periodic) {
            return;
        }
        let progress = Some(MediaPosition(app.position()));
        let playback = match state {
            None => MediaPlayback::Stopped,
            Some(true) => MediaPlayback::Paused { progress },
            Some(false) => MediaPlayback::Playing { progress },
        };
        let _ = self.controls.set_playback(playback);
        self.last_state = Some(state);
        self.last_push_secs = now;
    }

    /// Write artwork bytes to a stable temp file and return a `file://` URL, so
    /// the desktop widget can show the cover. Returns `None` when there's no art.
    fn write_cover(&mut self, bytes: Option<&[u8]>) -> Option<String> {
        let bytes = bytes?;
        let path = self
            .cover_path
            .get_or_insert_with(|| {
                std::env::temp_dir().join(format!("sparkplayer-cover-{}.img", std::process::id()))
            })
            .clone();
        fs::write(&path, bytes).ok()?;
        Some(format!("file://{}", path.to_string_lossy()))
    }
}
