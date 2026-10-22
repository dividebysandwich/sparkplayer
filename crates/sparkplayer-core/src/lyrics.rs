//! Lyrics model and pure LRC parser. No I/O: the native crate supplies the raw
//! text (sidecar `.lrc` or embedded tag) through `TrackMeta::lyrics`.

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    /// Parse LRC text. Returns `None` if it contains no timestamped lines
    /// (e.g. plain unsynced lyrics).
    ///
    /// Handles multiple timestamps per line (`[00:10.00][01:30.00]text`),
    /// the `[offset:+/-ms]` header (positive = lyrics appear sooner, per the
    /// LRC convention), and strips enhanced-LRC word stamps (`<00:12.30>`).
    pub fn parse_lrc(text: &str) -> Option<Lyrics> {
        let mut lines: Vec<LyricLine> = Vec::new();
        let mut offset_secs = 0.0;

        for raw in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
            let mut rest = raw.trim();
            let mut stamps: Vec<f64> = Vec::new();
            while let Some(inner) = rest.strip_prefix('[') {
                let Some(end) = inner.find(']') else { break };
                let tag = &inner[..end];
                if let Some(t) = parse_stamp(tag) {
                    stamps.push(t);
                } else if let Some(v) = tag.strip_prefix("offset:") {
                    if let Ok(ms) = v.trim().parse::<f64>() {
                        offset_secs = ms / 1000.0;
                    }
                } else {
                    // Unknown tag ([ar:..], [Chorus], ...): stop; whatever is
                    // left is lyric text (or ignored if there were no stamps).
                    break;
                }
                rest = inner[end + 1..].trim_start();
            }
            if stamps.is_empty() {
                continue;
            }
            let text = strip_word_stamps(rest).trim().to_string();
            for t in stamps {
                lines.push(LyricLine { time_secs: t, text: text.clone() });
            }
        }

        if lines.is_empty() {
            return None;
        }
        for l in &mut lines {
            l.time_secs = (l.time_secs - offset_secs).max(0.0);
        }
        lines.sort_by(|a, b| a.time_secs.partial_cmp(&b.time_secs).unwrap_or(std::cmp::Ordering::Equal));
        Some(Lyrics { lines })
    }

    /// Index of the line being sung at `secs`, or `None` before the first line.
    pub fn current_index(&self, secs: f64) -> Option<usize> {
        let i = self.lines.partition_point(|l| l.time_secs <= secs);
        i.checked_sub(1)
    }

    /// (previous, current, next) lines at `secs`. Before the first line the
    /// result is `(None, None, first)`.
    pub fn context(&self, secs: f64) -> (Option<&LyricLine>, Option<&LyricLine>, Option<&LyricLine>) {
        match self.current_index(secs) {
            None => (None, None, self.lines.first()),
            Some(i) => (
                i.checked_sub(1).and_then(|p| self.lines.get(p)),
                        self.lines.get(i),
                        self.lines.get(i + 1),
            ),
        }
    }
}

/// `mm:ss`, `mm:ss.xx`, `mm:ss,xxx`. Rejects `ar:...`, `ti:...` etc.
fn parse_stamp(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let s: f64 = s.trim().replace(',', ".").parse().ok()?;
    Some(m as f64 * 60.0 + s)
}

fn strip_word_stamps(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('>') {
            Some(j) if parse_stamp(&after[..j]).is_some() => rest = &after[j + 1..],
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_and_multi_stamp() {
        let l = Lyrics::parse_lrc("[ar:Someone]\n[00:01.00]one\n[00:03.50][00:09.00]two\n").unwrap();
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[1].time_secs, 3.5);
        assert_eq!(l.lines[2].text, "two");
    }

    #[test]
    fn offset_and_lookup() {
        let l = Lyrics::parse_lrc("[offset:+500]\n[00:02.00]a\n[00:04.00]b").unwrap();
        assert_eq!(l.lines[0].time_secs, 1.5);
        assert_eq!(l.current_index(1.0), None);
        assert_eq!(l.current_index(1.5), Some(0));
        assert_eq!(l.current_index(10.0), Some(1));
    }

    #[test]
    fn unsynced_is_none_and_word_stamps_stripped() {
        assert!(Lyrics::parse_lrc("just words\nmore words").is_none());
        let l = Lyrics::parse_lrc("[00:01.00]<00:01.00>Hel<00:01.40>lo").unwrap();
        assert_eq!(l.lines[0].text, "Hello");
    }
}
