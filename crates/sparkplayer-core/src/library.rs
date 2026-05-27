use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

pub const SUPPORTED_EXTS: &[&str] = &[
    "mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma",
];
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "m4v",
];
pub const PLAYLIST_EXTS: &[&str] = &["m3u", "m3u8", "pls"];

#[derive(Debug, Clone)]
pub struct Track {
    pub path: PathBuf,
    pub display: String,
}

impl Track {
    pub fn from_path(path: PathBuf) -> Self {
        let display = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        Self { path, display }
    }
}

pub fn is_audio_file(path: &Path) -> bool {
    has_ext(path, SUPPORTED_EXTS) || has_ext(path, VIDEO_EXTS)
}

pub fn is_video_file(path: &Path) -> bool {
    has_ext(path, VIDEO_EXTS)
}

pub fn is_playlist_file(path: &Path) -> bool {
    has_ext(path, PLAYLIST_EXTS)
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// Load a set of tracks from a path. The path may be:
/// - A single audio file
/// - A directory (scanned recursively, sorted by path)
/// - A playlist file (.m3u/.m3u8/.pls)
pub fn load_tracks(path: &Path) -> Result<Vec<Track>> {
    if path.is_file() {
        if is_playlist_file(path) {
            return load_playlist(path);
        }
        if is_audio_file(path) {
            return Ok(vec![Track::from_path(path.to_path_buf())]);
        }
        anyhow::bail!("unsupported file type: {}", path.display());
    }
    if path.is_dir() {
        return Ok(scan_directory(path));
    }
    anyhow::bail!("path does not exist: {}", path.display())
}

pub fn scan_directory(dir: &Path) -> Vec<Track> {
    let mut tracks = Vec::new();
    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() && is_audio_file(entry.path()) {
            tracks.push(Track::from_path(entry.into_path()));
        }
    }
    tracks.sort_by(|a, b| a.path.cmp(&b.path));
    tracks
}

pub fn load_playlist(path: &Path) -> Result<Vec<Track>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading playlist {}", path.display()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let entries: Vec<PathBuf> = match ext.as_str() {
        "pls" => parse_pls(&content),
        _ => parse_m3u(&content),
    };

    let mut out = Vec::new();
    for raw in entries {
        let resolved = if raw.is_absolute() {
            raw
        } else {
            parent.join(raw)
        };
        if resolved.is_file() {
            out.push(Track::from_path(resolved));
        }
    }
    Ok(out)
}

fn parse_m3u(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect()
}

fn parse_pls(text: &str) -> Vec<PathBuf> {
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

/// Look for a sidecar cover image next to the song (cover.jpg, folder.png, ...).
/// Used as a fallback when the audio file has no embedded artwork.
pub fn find_local_cover(audio_path: &Path) -> Option<Vec<u8>> {
    let dir = audio_path.parent()?;
    let stems = ["cover", "folder", "front", "albumart", "album", "artwork"];
    let exts = ["jpg", "jpeg", "png", "webp"];
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let stem_l = stem.to_ascii_lowercase();
        let ext_l = ext.to_ascii_lowercase();
        if stems.contains(&stem_l.as_str()) && exts.contains(&ext_l.as_str()) {
            if let Ok(bytes) = std::fs::read(&path) {
                return Some(bytes);
            }
        }
    }
    None
}

pub fn default_music_dir() -> PathBuf {
    dirs::audio_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}
