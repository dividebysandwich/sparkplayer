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
    last_stopped: bool,
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
            last_stopped: false,
            cover_path: None,
        })
    }

    /// Drain control requests the OS has queued since the last call.
    pub fn poll(&mut self) -> Vec<MediaCommand> {
        self.rx.try_iter().collect()
    }

    /// Push the current track metadata and playback state to the OS. Metadata is
    /// only re-sent when the track changes; playback state + progress every call.
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
        if app.playing_index.is_some() && self.last_meta.as_ref() != Some(&key) {
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

        let stopped = app.playing_index.is_none();
        let paused = app.audio.is_paused();
        let progress = Some(MediaPosition(app.position()));
        if stopped {
            if !self.last_stopped {
                let _ = self.controls.set_playback(MediaPlayback::Stopped);
                self.last_stopped = true;
            }
            return;
        }
        self.last_stopped = false;
        let playback = if paused {
            MediaPlayback::Paused { progress }
        } else {
            MediaPlayback::Playing { progress }
        };
        let _ = self.controls.set_playback(playback);
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
