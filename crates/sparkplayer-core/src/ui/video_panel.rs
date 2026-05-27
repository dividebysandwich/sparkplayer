//! The video panel: frames the picture area and renders the active subtitle
//! cue (or a transient announcement) in a fixed-height strip below it. The
//! actual picture is drawn by the `VideoBackend` (native: a scaled
//! ratatui-image; web: a `<video>` overlay positioned at the recorded rect).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;

use super::palette::{panel_bg, purple, yellow};

pub(super) fn draw_video(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(purple()))
        .title(Line::from(Span::styled(
            " Video ",
            Style::default().fg(yellow()).add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(panel_bg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Reserve a fixed-height strip below the video whenever a subtitle track is
    // active, so the video frame doesn't shift as cues come and go.
    const SUB_STRIP_ROWS: u16 = 2;
    let announcement = app.subtitle_announcement();
    let subs_active = app.active_subtitle_track.is_some() && app.subtitles.track_count() > 0;
    let need_strip = subs_active || announcement.is_some();
    let (video_area, sub_area) = if need_strip && inner.height > SUB_STRIP_ROWS + 1 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(SUB_STRIP_ROWS)])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    // Prefer a real cue over the announcement when both are present.
    let strip_text: Option<String> = app.current_subtitle_text.clone().or(announcement);
    let sub_lines: Vec<String> = match (sub_area, strip_text.as_deref()) {
        (Some(area), Some(text)) => wrap_subtitle(text, area.width as usize, SUB_STRIP_ROWS as usize),
        _ => Vec::new(),
    };

    // Record the picture rect for the web overlay, then let the backend paint
    // it (native) or no-op (web).
    app.last_video_rect = Some(video_area);
    app.video.render(frame, video_area);

    if let Some(sub_area) = sub_area {
        let lines: Vec<Line> = sub_lines
            .into_iter()
            .map(|s| {
                Line::from(Span::styled(
                    s,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            sub_area,
        );
    }
}

/// Greedy word-wrap a subtitle string to `width` columns, capped at `max_lines`.
/// Honors existing `\n` line breaks. The last line is truncated with `…` when
/// content overflows `max_lines`.
fn wrap_subtitle(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for segment in text.split('\n') {
        if out.len() >= max_lines {
            break;
        }
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let mut current = String::new();
        for word in segment.split_whitespace() {
            if current.is_empty() {
                if word.chars().count() <= width {
                    current.push_str(word);
                } else {
                    let mut chars = word.chars();
                    loop {
                        let chunk: String = chars.by_ref().take(width).collect();
                        if chunk.is_empty() {
                            break;
                        }
                        if out.len() < max_lines {
                            out.push(chunk);
                        }
                    }
                    current.clear();
                }
            } else if current.chars().count() + 1 + word.chars().count() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                if out.len() >= max_lines {
                    break;
                }
                current.push_str(word);
            }
        }
        if !current.is_empty() && out.len() < max_lines {
            out.push(current);
        }
    }
    if out.len() > max_lines {
        out.truncate(max_lines);
    }
    let total_text_words = text.split_whitespace().count();
    let used_words: usize = out.iter().map(|l| l.split_whitespace().count()).sum();
    if used_words < total_text_words {
        if let Some(last) = out.last_mut() {
            while last.chars().count() + 1 > width {
                last.pop();
            }
            last.push('…');
        }
    }
    out
}
