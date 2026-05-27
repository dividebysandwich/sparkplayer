//! The video panel: scales the decoded frame to fit while preserving aspect,
//! and renders the active subtitle cue (or a transient announcement) in a
//! fixed-height strip below it.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui_image::{Resize, StatefulImage};

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

    // Reserve a fixed-height strip below the video whenever a subtitle track
    // is active, so the video frame doesn't shift up/down as cues come and go.
    // The strip is sized for the maximum cue height (2 lines); empty rows are
    // simply left blank between cues.
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
    let strip_text: Option<String> = app
        .current_subtitle_text
        .clone()
        .or(announcement);
    let sub_lines: Vec<String> = match (sub_area, strip_text.as_deref()) {
        (Some(area), Some(text)) => wrap_subtitle(text, area.width as usize, SUB_STRIP_ROWS as usize),
        _ => Vec::new(),
    };

    let font_size = app
        .picker
        .as_ref()
        .map(|p| p.font_size())
        .unwrap_or(ratatui_image::FontSize::new(8, 16));
    let font_w = font_size.width;
    let font_h = font_size.height;
    let dims = app.video_dims;

    if let Some(proto) = app.video_protocol.as_mut() {
        let (iw, ih) = dims.unwrap_or((1, 1));
        let iw = iw.max(1);
        let ih = ih.max(1);

        let avail_w_px = video_area.width as u32 * font_w.max(1) as u32;
        let avail_h_px = video_area.height as u32 * font_h.max(1) as u32;

        let scale = (avail_w_px as f64 / iw as f64).min(avail_h_px as f64 / ih as f64);
        let fit_w_px = (iw as f64 * scale).round() as u32;
        let fit_h_px = (ih as f64 * scale).round() as u32;

        let cells_w = ((fit_w_px + font_w as u32 - 1) / font_w.max(1) as u32)
            .max(1)
            .min(video_area.width as u32) as u16;
        let cells_h = ((fit_h_px + font_h as u32 - 1) / font_h.max(1) as u32)
            .max(1)
            .min(video_area.height as u32) as u16;

        let x = video_area.x + (video_area.width - cells_w) / 2;
        let y = video_area.y + (video_area.height - cells_h) / 2;
        let img_area = Rect::new(x, y, cells_w, cells_h);
        frame.render_stateful_widget(
            StatefulImage::default().resize(Resize::Scale(None)),
            img_area,
            proto,
        );
    }

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
                    // Word longer than width — break inside the word.
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
    // If we hit the cap and there is still more text we didn't fit, indicate
    // truncation on the last line.
    let total_text_words = text.split_whitespace().count();
    let used_words: usize = out
        .iter()
        .map(|l| l.split_whitespace().count())
        .sum();
    if used_words < total_text_words {
        if let Some(last) = out.last_mut() {
            // Ensure room for the ellipsis.
            while last.chars().count() + 1 > width {
                last.pop();
            }
            last.push('…');
        }
    }
    out
}
