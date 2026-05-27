//! Native subtitle loading: synchronous sidecar (.srt/.vtt) discovery plus
//! background embedded-track extraction via ffmpeg. Both feed cues into a
//! `sparkplayer_core::SubtitleSet` through its pure parsers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use ffmpeg_next as ffmpeg;
use ffmpeg::codec::Id;
use ffmpeg::format::stream::Disposition;
use ffmpeg::media::Type;

use sparkplayer_core::subtitles::{
    self, SubtitleCue, SubtitleSet, SubtitleTrack,
};

pub fn load_for_video(video_path: &Path) -> SubtitleSet {
    let set = SubtitleSet::default();
    // Sidecar parsing is fast — surface those tracks synchronously.
    let sidecars = discover_sidecars(video_path);
    if !sidecars.is_empty() {
        set.extend(sidecars);
    }

    // Embedded extraction can read a lot of the container — punt to a thread.
    let path = video_path.to_path_buf();
    let set_t = set.clone();
    let _ = thread::Builder::new()
        .name("sparkplayer-subs".into())
        .spawn(move || {
            if set_t.is_cancelled() {
                return;
            }
            let cancelled = AtomicBool::new(false);
            let tracks = extract_embedded(&path, &set_t, &cancelled).unwrap_or_default();
            if set_t.is_cancelled() {
                return;
            }
            if !tracks.is_empty() {
                set_t.extend(tracks);
            }
        });

    set
}

fn extract_embedded(
    video_path: &Path,
    set: &SubtitleSet,
    cancelled: &AtomicBool,
) -> Option<Vec<SubtitleTrack>> {
    ffmpeg::init().ok();
    let mut ictx = ffmpeg::format::input(&video_path.to_path_buf()).ok()?;

    struct Pending {
        index: usize,
        label: String,
        language: Option<String>,
        time_base_num: f64,
        time_base_den: f64,
        decoder: ffmpeg::codec::decoder::Subtitle,
        cues: Vec<SubtitleCue>,
    }

    let mut pendings: Vec<Pending> = Vec::new();
    for stream in ictx.streams() {
        let params = stream.parameters();
        if params.medium() != Type::Subtitle {
            continue;
        }
        let codec_id = params.id();
        if !is_text_codec(codec_id) {
            continue;
        }
        let Ok(ctx) = ffmpeg::codec::context::Context::from_parameters(params) else {
            continue;
        };
        let Ok(decoder) = ctx.decoder().subtitle() else {
            continue;
        };
        let meta = stream.metadata();
        let title = meta
            .get("title")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let language = meta
            .get("language")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "und");
        let n = pendings.len() + 1;
        let label = build_label(language.as_deref(), title.as_deref(), stream.disposition(), n);
        let tb = stream.time_base();
        pendings.push(Pending {
            index: stream.index(),
            label,
            language,
            time_base_num: tb.numerator() as f64,
            time_base_den: tb.denominator() as f64,
            decoder,
            cues: Vec::new(),
        });
    }

    if pendings.is_empty() {
        return Some(Vec::new());
    }

    // Tell ffmpeg to drop everything that isn't one of our subtitle streams.
    let kept: std::collections::HashSet<usize> = pendings.iter().map(|p| p.index).collect();
    let stream_count = ictx.nb_streams() as usize;
    for i in 0..stream_count {
        if let Some(mut sm) = ictx.stream_mut(i) {
            if !kept.contains(&i) {
                unsafe {
                    (*sm.as_mut_ptr()).discard = ffmpeg::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }
    }

    for (stream, packet) in ictx.packets() {
        if cancelled.load(Ordering::Relaxed) || set.is_cancelled() {
            return None;
        }
        let idx = stream.index();
        let Some(pending) = pendings.iter_mut().find(|p| p.index == idx) else {
            continue;
        };
        let pts = packet.pts().unwrap_or(0);
        let packet_dur = packet.duration();
        let mut sub = ffmpeg::Subtitle::new();
        match pending.decoder.decode(&packet, &mut sub) {
            Ok(true) => {}
            _ => continue,
        }
        let base_secs = pts as f64 * pending.time_base_num / pending.time_base_den;
        let start_off_ms = sub.start() as f64;
        let end_off_ms = sub.end() as f64;
        let mut start_secs = base_secs + start_off_ms / 1000.0;
        let mut end_secs = base_secs + end_off_ms / 1000.0;
        if end_secs <= start_secs {
            let dur = if packet_dur > 0 {
                packet_dur as f64 * pending.time_base_num / pending.time_base_den
            } else {
                2.0
            };
            end_secs = start_secs + dur;
        }
        if start_secs < 0.0 {
            start_secs = 0.0;
        }

        for rect in sub.rects() {
            let text = match rect {
                ffmpeg::codec::subtitle::Rect::Text(t) => subtitles::clean_html(t.get()),
                ffmpeg::codec::subtitle::Rect::Ass(a) => subtitles::parse_ass_dialogue(a.get()),
                _ => continue,
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            pending.cues.push(SubtitleCue {
                start_secs,
                end_secs,
                text,
            });
        }
    }

    let mut tracks: Vec<SubtitleTrack> = pendings
        .into_iter()
        .map(|p| {
            let mut cues = p.cues;
            cues.sort_by(|a, b| {
                a.start_secs
                    .partial_cmp(&b.start_secs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            SubtitleTrack {
                label: p.label,
                language: p.language,
                cues,
            }
        })
        .collect();
    tracks.retain(|t| !t.cues.is_empty());
    Some(tracks)
}

fn build_label(
    language: Option<&str>,
    title: Option<&str>,
    disposition: Disposition,
    n: usize,
) -> String {
    let lang_name = language.map(subtitles::language_display_name);
    let mut qualifiers: Vec<&str> = Vec::new();
    if disposition.contains(Disposition::FORCED) {
        qualifiers.push("forced");
    }
    if disposition.contains(Disposition::HEARING_IMPAIRED) {
        qualifiers.push("SDH");
    }
    if disposition.contains(Disposition::COMMENT) {
        qualifiers.push("commentary");
    }
    if let Some(t) = title {
        let lower = t.to_ascii_lowercase();
        if !qualifiers.contains(&"SDH") && (lower.contains("sdh") || lower.contains("hearing")) {
            qualifiers.push("SDH");
        }
        if !qualifiers.contains(&"forced") && lower.contains("forced") {
            qualifiers.push("forced");
        }
        if !qualifiers.contains(&"commentary") && lower.contains("comment") {
            qualifiers.push("commentary");
        }
    }
    let qual_suffix = if qualifiers.is_empty() {
        String::new()
    } else {
        format!(" ({})", qualifiers.join(", "))
    };
    match lang_name {
        Some(name) => format!("{name}{qual_suffix}"),
        None => {
            let title_is_meaningful = title
                .map(|t| t.contains(' ') || t.chars().any(|c| c.is_ascii_lowercase()))
                .unwrap_or(false);
            match title {
                Some(t) if title_is_meaningful => format!("{t}{qual_suffix}"),
                _ => format!("Track {n}{qual_suffix}"),
            }
        }
    }
}

fn is_text_codec(id: Id) -> bool {
    matches!(id, Id::SUBRIP | Id::ASS | Id::SSA | Id::MOV_TEXT | Id::WEBVTT)
}

fn discover_sidecars(video_path: &Path) -> Vec<SubtitleTrack> {
    let mut out = Vec::new();
    let Some(dir) = video_path.parent() else {
        return out;
    };
    let Some(video_stem) = video_path.file_stem().and_then(|s| s.to_str()) else {
        return out;
    };
    let video_stem_l = video_stem.to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut candidates: Vec<(PathBuf, String, String)> = Vec::new();
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
        let ext_l = ext.to_ascii_lowercase();
        if ext_l != "srt" && ext_l != "vtt" {
            continue;
        }
        let stem_l = stem.to_ascii_lowercase();
        let lang_suffix = if stem_l == video_stem_l {
            String::new()
        } else if let Some(rest) = stem_l.strip_prefix(&format!("{video_stem_l}.")) {
            rest.to_string()
        } else {
            continue;
        };
        candidates.push((path, lang_suffix, ext_l));
    }
    candidates.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));

    for (path, lang, ext) in candidates {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let text = subtitles::decode_text(&bytes);
        let cues = match ext.as_str() {
            "srt" => subtitles::parse_srt(&text),
            "vtt" => subtitles::parse_vtt(&text),
            _ => continue,
        };
        if cues.is_empty() {
            continue;
        }
        let label = if lang.is_empty() {
            format!("sidecar ({ext})")
        } else {
            format!("{lang} (sidecar)")
        };
        let language = if lang.is_empty() { None } else { Some(lang) };
        out.push(SubtitleTrack {
            label,
            language,
            cues,
        });
    }
    out
}
