//! Track model and the pure (platform-agnostic) parts of library handling:
//! the `TrackRef` source abstraction, file-type classification by extension,
//! and playlist parsing. Filesystem-backed operations (directory scanning,
//! reading playlist files, sidecar cover discovery) live in the native crate
//! behind the [`crate::backend::MediaLibrary`] trait.

use std::path::{Path, PathBuf};

pub const SUPPORTED_EXTS: &[&str] = &[
    "mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma",
];
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "m4v",
];
pub const PLAYLIST_EXTS: &[&str] = &["m3u", "m3u8", "pls"];

/// Where a track's media comes from. Native playback uses a filesystem path;
/// the browser build uses a URL (a remote http(s) URL from the manifest, or an
/// object URL minted from a user-picked local `File`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackRef {
    Path(PathBuf),
    /// `(url, name)` — `name` is the original/display name (object URLs carry no
    /// extension, so we keep the real name to classify audio vs. video).
    Url(String, String),
}

impl TrackRef {
    pub fn from_path(path: PathBuf) -> Self {
        TrackRef::Path(path)
    }

    pub fn from_url(url: impl Into<String>, name: impl Into<String>) -> Self {
        TrackRef::Url(url.into(), name.into())
    }

    /// The string the underlying player should load (a path string or a URL).
    pub fn locator(&self) -> String {
        match self {
            TrackRef::Path(p) => p.to_string_lossy().to_string(),
            TrackRef::Url(url, _) => url.clone(),
        }
    }

    /// The name used for classification and display (filename or manifest name).
    pub fn name(&self) -> String {
        match self {
            TrackRef::Path(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string()),
            TrackRef::Url(_, name) => name.clone(),
        }
    }

    /// Lowercased file extension derived from the name, if any.
    pub fn extension(&self) -> Option<String> {
        let name = self.name();
        Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub source: TrackRef,
    pub display: String,
}

impl Track {
    pub fn from_path(path: PathBuf) -> Self {
        let display = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        Self {
            source: TrackRef::Path(path),
            display,
        }
    }

    pub fn from_url(url: impl Into<String>, display: impl Into<String>) -> Self {
        let display = display.into();
        Self {
            source: TrackRef::from_url(url, display.clone()),
            display,
        }
    }
}

fn ext_in(ext: Option<&str>, exts: &[&str]) -> bool {
    ext.map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

pub fn is_audio_ext(ext: Option<&str>) -> bool {
    ext_in(ext, SUPPORTED_EXTS) || ext_in(ext, VIDEO_EXTS)
}

pub fn is_video_ext(ext: Option<&str>) -> bool {
    ext_in(ext, VIDEO_EXTS)
}

pub fn is_playlist_ext(ext: Option<&str>) -> bool {
    ext_in(ext, PLAYLIST_EXTS)
}

pub fn is_audio_file(path: &Path) -> bool {
    is_audio_ext(path.extension().and_then(|e| e.to_str()))
}

pub fn is_video_file(path: &Path) -> bool {
    is_video_ext(path.extension().and_then(|e| e.to_str()))
}

pub fn is_playlist_file(path: &Path) -> bool {
    is_playlist_ext(path.extension().and_then(|e| e.to_str()))
}

/// Classify a [`TrackRef`] as video by its extension.
pub fn is_video(source: &TrackRef) -> bool {
    is_video_ext(source.extension().as_deref())
}

/// Parse an M3U/M3U8 playlist into raw relative/absolute paths (one per line).
pub fn parse_m3u(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect()
}

/// Parse a PLS playlist into raw paths, ordered by their `FileN=` index.
pub fn parse_pls(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<(usize, PathBuf)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("File") {
            if let Some(eq) = rest.find('=') {
                let n: usize = rest[..eq].parse().unwrap_or(0);
                let path = rest[eq + 1..].trim().to_string();
                if !path.is_empty() {
                    out.push((n, PathBuf::from(path)));
                }
            }
        }
    }
    out.sort_by_key(|(n, _)| *n);
    out.into_iter().map(|(_, p)| p).collect()
}
