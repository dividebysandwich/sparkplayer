//! Filesystem metadata extraction via `lofty`. Produces the platform-agnostic
//! [`TrackMeta`] consumed by the core UI.

use std::path::Path;

use anyhow::Result;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, PictureType};
use lofty::probe::Probe;
use lofty::tag::Accessor;

use sparkplayer_core::metadata::TrackMeta;

pub fn read_metadata(path: &Path) -> Result<TrackMeta> {
    let tagged = Probe::open(path)?.read()?;
    let props = tagged.properties();
    let duration = Some(props.duration());
    let sample_rate = props.sample_rate();
    let channels = props.channels();
    let bitrate = props.audio_bitrate();

    let mut meta = TrackMeta {
        duration,
        sample_rate,
        channels,
        bitrate,
        ..Default::default()
    };

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        meta.title = tag.title().map(|s| s.to_string());
        meta.artist = tag.artist().map(|s| s.to_string());
        meta.album = tag.album().map(|s| s.to_string());
        meta.year = tag.date().map(|d| d.year as u32);
        meta.track_no = tag.track();
    }

    // Sweep every tag for embedded pictures and pick the highest-quality one.
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
                meta.artwork_mime = Some(mime_name(pic.mime_type()).to_string());
            }
        }
    }

    Ok(meta)
}

fn mime_name(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Png) => "image/png",
        Some(MimeType::Jpeg) => "image/jpeg",
        Some(MimeType::Gif) => "image/gif",
        Some(MimeType::Bmp) => "image/bmp",
        Some(MimeType::Tiff) => "image/tiff",
        _ => "application/octet-stream",
    }
}
