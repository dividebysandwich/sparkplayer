//! Parse a picked local file's bytes into [`TrackMeta`] with `lofty`. This gives
//! the browser build the same title/artist/album and embedded cover art the
//! native build reads from disk — just from an in-memory buffer instead.

use std::io::Cursor;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::Accessor;

use sparkplayer_core::metadata::TrackMeta;

pub fn parse_metadata(bytes: &[u8]) -> TrackMeta {
    let mut meta = TrackMeta::default();
    let probe = match Probe::new(Cursor::new(bytes)).guess_file_type() {
        Ok(p) => p,
        Err(_) => return meta,
    };
    let tagged = match probe.read() {
        Ok(t) => t,
        Err(_) => return meta,
    };

    let props = tagged.properties();
    meta.duration = Some(props.duration());
    meta.sample_rate = props.sample_rate();
    meta.channels = props.channels();
    meta.bitrate = props.audio_bitrate();

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        meta.title = tag.title().map(|s| s.to_string());
        meta.artist = tag.artist().map(|s| s.to_string());
        meta.album = tag.album().map(|s| s.to_string());
        meta.year = tag.date().map(|d| d.year as u32);
        meta.track_no = tag.track();
    }

    let mut best_score = -1i32;
    for tag in tagged.tags() {
        for pic in tag.pictures() {
            if pic.data().is_empty() {
                continue;
            }
            let score = match pic.pic_type() {
                PictureType::CoverFront => 100,
                PictureType::Other => 60,
                PictureType::Icon | PictureType::OtherIcon => 30,
                _ => 20,
            };
            if score > best_score {
                best_score = score;
                meta.artwork = Some(pic.data().to_vec());
            }
        }
    }

    meta
}
