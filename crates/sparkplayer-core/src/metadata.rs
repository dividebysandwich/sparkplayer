//! Track metadata model. The struct is platform-agnostic; the native crate
//! fills it via `lofty` from a file, while the web crate fills the textual
//! fields from the manifest.

use std::time::Duration;

#[derive(Debug, Default, Clone)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub track_no: Option<u32>,
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bitrate: Option<u32>,
    pub artwork: Option<Vec<u8>>,
    pub artwork_mime: Option<String>,
}
