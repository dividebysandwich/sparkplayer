//! Subtitle data model and the pure parsers (SRT/VTT/ASS, language-name
//! mapping). Sources that require platform facilities — embedded extraction
//! via ffmpeg, sidecar directory scans, background threads — live in the
//! native crate. The web crate fetches a `.vtt` URL and feeds it to
//! [`parse_vtt`]. Both build a [`SubtitleSet`] through [`SubtitleSet::extend`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct SubtitleCue {
    pub start_secs: f64,
    pub end_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    pub label: String,
    pub language: Option<String>,
    pub cues: Vec<SubtitleCue>,
}

#[derive(Debug, Default)]
struct SubtitleInner {
    tracks: Mutex<Vec<SubtitleTrack>>,
    cancelled: AtomicBool,
}

/// Thread-safe handle to a (possibly still-loading) set of subtitle tracks.
/// Producers append tracks via [`SubtitleSet::extend`]; readers can query
/// whatever is available at any moment without blocking playback.
#[derive(Debug, Default, Clone)]
pub struct SubtitleSet {
    inner: Arc<SubtitleInner>,
}

impl SubtitleSet {
    pub fn track_count(&self) -> usize {
        self.inner.tracks.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn track_label(&self, idx: usize) -> Option<String> {
        let guard = self.inner.tracks.lock().ok()?;
        guard.get(idx).map(|t| t.label.clone())
    }

    /// Append tracks with at least one cue. Used by both platforms' loaders.
    pub fn extend(&self, tracks: impl IntoIterator<Item = SubtitleTrack>) {
        if let Ok(mut guard) = self.inner.tracks.lock() {
            guard.extend(tracks.into_iter().filter(|t| !t.cues.is_empty()));
        }
    }

    /// Locate a track by language hint, returning its index. Matches the
    /// requested string (case-insensitive) against the ISO language tag first
    /// and the human label second. Empty input returns None.
    pub fn find_track_by_language(&self, query: &str) -> Option<usize> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return None;
        }
        let guard = self.inner.tracks.lock().ok()?;
        for (i, t) in guard.iter().enumerate() {
            if let Some(lang) = t.language.as_ref() {
                if lang.to_ascii_lowercase() == q {
                    return Some(i);
                }
            }
        }
        for (i, t) in guard.iter().enumerate() {
            if t.label.to_ascii_lowercase() == q {
                return Some(i);
            }
        }
        for (i, t) in guard.iter().enumerate() {
            if t.label.to_ascii_lowercase().contains(&q) {
                return Some(i);
            }
        }
        None
    }

    pub fn cue_at(&self, track_idx: usize, secs: f64) -> Option<String> {
        let guard = self.inner.tracks.lock().ok()?;
        let track = guard.get(track_idx)?;
        if track.cues.is_empty() {
            return None;
        }
        let i = track.cues.partition_point(|c| c.start_secs <= secs);
        if i == 0 {
            return None;
        }
        let cue = &track.cues[i - 1];
        if cue.end_secs >= secs {
            Some(cue.text.clone())
        } else {
            None
        }
    }

    /// Signal any background loader to stop as soon as it can.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }
}

/// Map ISO 639-1 / 639-2 / 639-3 codes to English language names. Falls back
/// to returning the code as-is for codes we don't know.
pub fn language_display_name(code: &str) -> String {
    let key = code.trim().to_ascii_lowercase();
    let name = match key.as_str() {
        "en" | "eng" => "English",
        "de" | "ger" | "deu" => "German",
        "fr" | "fre" | "fra" => "French",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "nl" | "dut" | "nld" => "Dutch",
        "sv" | "swe" => "Swedish",
        "no" | "nor" => "Norwegian",
        "da" | "dan" => "Danish",
        "fi" | "fin" => "Finnish",
        "is" | "ice" | "isl" => "Icelandic",
        "pl" | "pol" => "Polish",
        "cs" | "cze" | "ces" => "Czech",
        "sk" | "slo" | "slk" => "Slovak",
        "hu" | "hun" => "Hungarian",
        "ro" | "rum" | "ron" => "Romanian",
        "ru" | "rus" => "Russian",
        "uk" | "ukr" => "Ukrainian",
        "bg" | "bul" => "Bulgarian",
        "sr" | "srp" => "Serbian",
        "hr" | "hrv" => "Croatian",
        "sl" | "slv" => "Slovenian",
        "el" | "gre" | "ell" => "Greek",
        "tr" | "tur" => "Turkish",
        "he" | "heb" => "Hebrew",
        "ar" | "ara" => "Arabic",
        "fa" | "per" | "fas" => "Persian",
        "hi" | "hin" => "Hindi",
        "bn" | "ben" => "Bengali",
        "ur" | "urd" => "Urdu",
        "ta" | "tam" => "Tamil",
        "te" | "tel" => "Telugu",
        "th" | "tha" => "Thai",
        "vi" | "vie" => "Vietnamese",
        "id" | "ind" => "Indonesian",
        "ms" | "may" | "msa" => "Malay",
        "fil" | "tgl" => "Filipino",
        "zh" | "chi" | "zho" => "Chinese",
        "ja" | "jpn" => "Japanese",
        "ko" | "kor" => "Korean",
        "lat" | "la" => "Latin",
        _ => return code.to_string(),
    };
    name.to_string()
}

/// Decode subtitle bytes to text, stripping a UTF-8 BOM and tolerating invalid
/// sequences.
pub fn decode_text(bytes: &[u8]) -> String {
    let trimmed = if bytes.starts_with(b"\xEF\xBB\xBF") {
        &bytes[3..]
    } else {
        bytes
    };
    match std::str::from_utf8(trimmed) {
        Ok(s) => s.to_string(),
        Err(_) => String::from_utf8_lossy(trimmed).into_owned(),
    }
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    let (main, ms_str) = match s.rsplit_once(|c| c == ',' || c == '.') {
        Some((a, b)) => (a, b),
        None => (s, "0"),
    };
    let ms: f64 = ms_str.parse().ok()?;
    let parts: Vec<&str> = main.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec + ms / 1000.0)
}

pub fn parse_srt(text: &str) -> Vec<SubtitleCue> {
    parse_srt_or_vtt(text, false)
}

pub fn parse_vtt(text: &str) -> Vec<SubtitleCue> {
    parse_srt_or_vtt(text, true)
}

fn parse_srt_or_vtt(text: &str, is_vtt: bool) -> Vec<SubtitleCue> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    for line in normalized.split('\n').chain(std::iter::once("")) {
        if line.is_empty() {
            if !block.is_empty() {
                if let Some(cue) = parse_block(&block, is_vtt) {
                    out.push(cue);
                }
                block.clear();
            }
            continue;
        }
        block.push(line);
    }
    out
}

fn parse_block(block: &[&str], is_vtt: bool) -> Option<SubtitleCue> {
    let mut i = 0;
    if is_vtt {
        let head = block[0].trim();
        if head.starts_with("WEBVTT") || head == "NOTE" || head.starts_with("NOTE ")
            || head == "STYLE" || head == "REGION"
        {
            return None;
        }
    }
    if !block[i].contains("-->") {
        i += 1;
        if i >= block.len() {
            return None;
        }
    }
    if !block[i].contains("-->") {
        return None;
    }
    let timeline = block[i];
    let (start_s, rest) = timeline.split_once("-->")?;
    let end_s = rest.trim().split_whitespace().next()?;
    let start_secs = parse_timestamp(start_s)?;
    let end_secs = parse_timestamp(end_s)?;
    i += 1;
    if i >= block.len() {
        return None;
    }
    let payload = block[i..].join("\n");
    let text = clean_html(&payload).trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(SubtitleCue {
        start_secs,
        end_secs,
        text,
    })
}

/// Strip simple `<...>` HTML tags. Used for SRT/VTT and to clean up Text-rect
/// subtitle payloads which may contain `<i>` etc.
pub fn clean_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Parse an ASS dialogue payload (as emitted in `AVSubtitleRect.ass`) and
/// return the plain visible text. Handles both the modern format
/// `ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text`
/// (8 commas before the text) and the older "Dialogue: ..." line format.
pub fn parse_ass_dialogue(s: &str) -> String {
    let trimmed = s.trim();
    let mut lines = Vec::new();
    for raw in trimmed.split('\n') {
        let line = raw.trim().trim_start_matches('\r');
        if line.is_empty() {
            continue;
        }
        let payload = if let Some(rest) = line.strip_prefix("Dialogue:") {
            nth_comma_tail(rest.trim_start(), 9)
        } else {
            nth_comma_tail(line, 8)
        };
        if let Some(text) = payload {
            lines.push(strip_ass_overrides(text));
        }
    }
    lines.join("\n")
}

fn nth_comma_tail(s: &str, n: usize) -> Option<&str> {
    let mut count = 0;
    for (i, ch) in s.char_indices() {
        if ch == ',' {
            count += 1;
            if count == n {
                return Some(&s[i + 1..]);
            }
        }
    }
    None
}

fn strip_ass_overrides(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_brace = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => in_brace = true,
            '}' if in_brace => in_brace = false,
            _ if in_brace => {}
            '\\' => match chars.peek() {
                Some('N') | Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('h') => {
                    chars.next();
                    out.push(' ');
                }
                Some(_) | None => out.push(ch),
            },
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_basic() {
        let text = "1\n00:00:01,000 --> 00:00:02,500\nHello world\n\n2\n00:00:03,000 --> 00:00:04,000\n<i>Second</i> line\n";
        let cues = parse_srt(text);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_secs, 1.0);
        assert_eq!(cues[0].end_secs, 2.5);
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!(cues[1].text, "Second line");
    }

    #[test]
    fn vtt_basic() {
        let text = "WEBVTT\n\nNOTE this is a note\n\n00:00:01.000 --> 00:00:02.000 align:start\nFirst\n\ncue-id\n00:00:03.000 --> 00:00:04.000\nSecond";
        let cues = parse_vtt(text);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "First");
        assert_eq!(cues[1].text, "Second");
    }

    #[test]
    fn cue_at_binary_search() {
        let set = SubtitleSet::default();
        set.extend(vec![SubtitleTrack {
            label: "t".into(),
            language: None,
            cues: vec![
                SubtitleCue { start_secs: 1.0, end_secs: 2.0, text: "a".into() },
                SubtitleCue { start_secs: 3.0, end_secs: 4.0, text: "b".into() },
            ],
        }]);
        assert_eq!(set.cue_at(0, 0.5), None);
        assert_eq!(set.cue_at(0, 1.5).as_deref(), Some("a"));
        assert_eq!(set.cue_at(0, 2.5), None);
        assert_eq!(set.cue_at(0, 3.5).as_deref(), Some("b"));
    }

    #[test]
    fn ass_strip() {
        let s = "0,0,Default,,0,0,0,,{\\an8}Hello\\Nworld";
        assert_eq!(parse_ass_dialogue(s), "Hello\nworld");
    }
}
