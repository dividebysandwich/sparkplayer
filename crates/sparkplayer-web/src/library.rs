//! Web implementations of [`MediaLibrary`] and [`ConfigStore`]. There is no
//! filesystem browser in the browser: tracks come from a fetched `manifest.json`
//! or from user-picked local files (handled in `lib.rs`), so most of these
//! methods are inert. Settings persist in `localStorage`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use web_sys::window;

use sparkplayer_core::backend::{ConfigStore, MediaLibrary};
use sparkplayer_core::config::Config;
use sparkplayer_core::library::{Track, TrackRef};
use sparkplayer_core::metadata::TrackMeta;
use sparkplayer_core::subtitles::SubtitleSet;

const STORAGE_KEY: &str = "sparkplayer";

/// Metadata parsed from picked local files (by locator/object-URL), shared with
/// the file-input handler in `lib.rs` which fills it.
pub type MetaMap = Rc<RefCell<HashMap<String, TrackMeta>>>;

pub struct WebLibrary {
    meta: MetaMap,
}

impl WebLibrary {
    pub fn new(meta: MetaMap) -> Self {
        Self { meta }
    }
}

impl MediaLibrary for WebLibrary {
    fn browse(&self, _dir: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    fn load_playlist(&self, _source: &TrackRef) -> anyhow::Result<Vec<Track>> {
        Ok(Vec::new())
    }

    fn scan_directory(&self, _dir: &Path) -> Vec<Track> {
        Vec::new()
    }

    fn read_metadata(&self, source: &TrackRef) -> TrackMeta {
        // Picked local files have their tags parsed up front (see `lib.rs`) and
        // stashed here by locator; manifest tracks fall back to defaults (title
        // comes from the Track's display name, duration from the media element).
        self.meta
            .borrow()
            .get(&source.locator())
            .cloned()
            .unwrap_or_default()
    }

    fn find_cover(&self, _source: &TrackRef) -> Option<Vec<u8>> {
        None
    }

    fn load_subtitles(&self, _source: &TrackRef) -> SubtitleSet {
        SubtitleSet::default()
    }
}

pub struct LocalStorageConfig;

impl LocalStorageConfig {
    fn storage() -> Option<web_sys::Storage> {
        window()?.local_storage().ok().flatten()
    }
}

impl ConfigStore for LocalStorageConfig {
    fn load(&self) -> Config {
        match Self::storage().and_then(|s| s.get_item(STORAGE_KEY).ok().flatten()) {
            Some(content) => Config::parse(&content),
            None => Config::default(),
        }
    }

    fn save(&self, cfg: &Config) {
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(STORAGE_KEY, &cfg.serialize());
        }
    }
}
