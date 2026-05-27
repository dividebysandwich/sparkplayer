//! Platform-agnostic core of SparkPlayer.
//!
//! This crate holds everything that is identical between the native terminal
//! build and the browser (WASM/Ratzilla) build: the [`app::App`] state machine
//! and keymap, the [`ui`] rendering, the [`visualizer`], themes, and the pure
//! parsing/model code. Anything that touches a real device — audio output,
//! video decoding, the filesystem, image decoding — is reached only through the
//! traits in [`backend`], which each platform crate implements.

pub mod app;
pub mod audio_tap;
pub mod backend;
pub mod config;
pub mod library;
pub mod metadata;
pub mod subtitles;
pub mod theme;
pub mod ui;
pub mod visualizer;

pub use app::App;
pub use audio_tap::SampleBuffer;
pub use backend::{
    AlbumArtRenderer, AudioBackend, ConfigStore, CoreKey, CoreKeyEvent, MediaLibrary, VideoBackend,
};
pub use config::Config;
pub use library::{Track, TrackRef};
pub use metadata::TrackMeta;
pub use subtitles::{SubtitleCue, SubtitleSet, SubtitleTrack};

// Re-export the exact ratatui the core was built against so the platform crates
// don't accidentally depend on a different version.
pub use ratatui;
