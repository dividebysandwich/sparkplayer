//! Filesystem-backed [`MediaLibrary`]: directory scanning, playlist file
//! loading, sidecar cover discovery, and the file-browser listing. The pure
//! classification/parsing it builds on lives in `sparkplayer_core::library`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use sparkplayer_core::backend::MediaLibrary;
use sparkplayer_core::library::{self, Track, TrackRef};
use sparkplayer_core::metadata::TrackMeta;
use sparkplayer_core::subtitles::SubtitleSet;

use crate::{metadata_native, subtitles_native};

pub struct NativeLibrary;

impl MediaLibrary for NativeLibrary {
    fn browse(&self, dir: &Path) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = Vec::new();
        if let Some(parent) = dir.parent() {
            entries.push(parent.to_path_buf());
        }
        if let Ok(read) = fs::read_dir(dir) {
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
        entries
    }

    fn load_playlist(&self, source: &TrackRef) -> Result<Vec<Track>> {
        match source {
            TrackRef::Path(p) => load_playlist(p),
            TrackRef::Url(..) => Ok(Vec::new()),
        }
    }

    fn scan_directory(&self, dir: &Path) -> Vec<Track> {
        scan_directory(dir)
    }

    fn read_metadata(&self, source: &TrackRef) -> TrackMeta {
        match source {
            TrackRef::Path(p) => metadata_native::read_metadata(p).unwrap_or_default(),
            TrackRef::Url(..) => TrackMeta::default(),
        }
    }

    fn find_cover(&self, source: &TrackRef) -> Option<Vec<u8>> {
        match source {
            TrackRef::Path(p) => find_local_cover(p),
            TrackRef::Url(..) => None,
        }
    }

    fn load_subtitles(&self, source: &TrackRef) -> SubtitleSet {
        match source {
            TrackRef::Path(p) => subtitles_native::load_for_video(p),
            TrackRef::Url(..) => SubtitleSet::default(),
        }
    }

    fn save_playlist(&self, path: &Path, tracks: &[Track]) -> Result<()> {
        save_playlist(path, tracks)
    }
}

/// Write `tracks` to an M3U file: an `#EXTM3U` header followed by one absolute
/// path per filesystem-backed track. URL tracks (none on native) are skipped.
pub fn save_playlist(path: &Path, tracks: &[Track]) -> Result<()> {
    let mut out = String::from("#EXTM3U\n");
    for t in tracks {
        if let TrackRef::Path(p) = &t.source {
            out.push_str(&p.to_string_lossy());
            out.push('\n');
        }
    }
    fs::write(path, out).with_context(|| format!("writing playlist {}", path.display()))
}

/// Load a set of tracks from a path: a single file, a directory (scanned
/// recursively), or a playlist file. Used at startup from `main`.
pub fn load_tracks(path: &Path) -> Result<Vec<Track>> {
    if path.is_file() {
        if library::is_playlist_file(path) {
            return load_playlist(path);
        }
        if library::is_audio_file(path) {
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
        if entry.file_type().is_file() && library::is_audio_file(entry.path()) {
            tracks.push(Track::from_path(entry.into_path()));
        }
    }
    tracks.sort_by(|a, b| a.source.locator().cmp(&b.source.locator()));
    tracks
}

pub fn load_playlist(path: &Path) -> Result<Vec<Track>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading playlist {}", path.display()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let entries: Vec<PathBuf> = match ext.as_str() {
        "pls" => library::parse_pls(&content),
        _ => library::parse_m3u(&content),
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

/// Look for a sidecar cover image next to the song (cover.jpg, folder.png, ...).
pub fn find_local_cover(audio_path: &Path) -> Option<Vec<u8>> {
    let dir = audio_path.parent()?;
    let stems = ["cover", "folder", "front", "albumart", "album", "artwork"];
    let exts = ["jpg", "jpeg", "png", "webp"];
    let entries = fs::read_dir(dir).ok()?;
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
            if let Ok(bytes) = fs::read(&path) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_playlist_round_trips_via_parse_m3u() {
        let dir = std::env::temp_dir();
        let file = dir.join(format!("sparkplayer-test-{}.m3u", std::process::id()));
        let tracks = vec![
            Track::from_path(dir.join("one.flac")),
            Track::from_path(dir.join("two.mp3")),
        ];
        save_playlist(&file, &tracks).expect("save");

        let content = fs::read_to_string(&file).expect("read");
        assert!(content.starts_with("#EXTM3U\n"));
        let parsed = library::parse_m3u(&content);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], dir.join("one.flac"));
        assert_eq!(parsed[1], dir.join("two.mp3"));

        fs::remove_file(&file).ok();
    }
}
