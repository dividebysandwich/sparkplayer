//! The main layout panels: playlist, file browser, "Now Playing" metadata
//! block with its volume column, album art, and the keybinding footer.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Padding, Paragraph, Wrap};
use ratatui_image::StatefulImage;

use crate::app::{App, FocusPane};

use super::palette::{cyan, dim, green, lerp, panel_bg, pink, purple, red, text, yellow};

pub(super) fn draw_playlist(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == FocusPane::Playlist;
    let border = if focused { pink() } else { purple() };

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let playing = Some(i) == app.playing_index;
            let prefix = if playing { "▶ " } else { "  " };
            let style = if playing {
                Style::default()
                    .fg(yellow())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(text())
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(cyan())),
                Span::styled(t.display.clone(), style),
            ]))
        })
        .collect();

    let title = format!(" ♪ Playlist ({}) ", app.tracks.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .title(Line::from(Span::styled(
                    title,
                    Style::default().fg(pink()).add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(panel_bg())),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(70, 35, 90))
                .fg(cyan())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("➤ ");

    frame.render_stateful_widget(list, area, &mut app.playlist_state);
}

pub(super) fn draw_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == FocusPane::Browser;
    let border = if focused { pink() } else { purple() };
    let cwd = app.browser_dir.display().to_string();
    let has_parent = app.browser_dir.parent().is_some();

    let items: Vec<ListItem> = app
        .browser_entries
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let parent = i == 0 && has_parent;
            let label = if parent {
                String::from("⤴ ..")
            } else if p.is_dir() {
                format!(
                    "📁 {}",
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                )
            } else {
                format!(
                    "🎵 {}",
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                )
            };
            let style = if p.is_dir() || parent {
                Style::default().fg(cyan())
            } else {
                Style::default().fg(text())
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .title(Line::from(Span::styled(
                    format!(" 📂 {} ", truncate_path(&cwd, area.width as usize)),
                    Style::default()
                        .fg(cyan())
                        .add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(panel_bg())),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(30, 60, 80))
                .fg(yellow())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("➤ ");

    frame.render_stateful_widget(list, area, &mut app.browser_state);
}

fn truncate_path(p: &str, max: usize) -> String {
    let budget = max.saturating_sub(6);
    if p.len() <= budget || budget == 0 {
        return p.to_string();
    }
    let tail = &p[p.len() - budget + 1..];
    format!("…{tail}")
}

fn layout_badges(badges: Vec<Span<'_>>, max_width: usize) -> Vec<Line<'_>> {
    const SEP: &str = "  ";
    let sep_w = SEP.len();
    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut current_w: usize = 0;
    for badge in badges {
        let w = badge.width();
        let needed = if current.is_empty() {
            w
        } else {
            current_w + sep_w + w
        };
        if needed > max_width && !current.is_empty() {
            lines.push(Line::from(std::mem::take(&mut current)));
            lines.push(Line::from(""));
            current_w = 0;
        }
        if !current.is_empty() {
            current.push(Span::raw(SEP));
            current_w += sep_w;
        }
        current_w += w;
        current.push(badge);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

pub(super) fn draw_now_playing(frame: &mut Frame, area: Rect, app: &App) {
    let meta = &app.current_meta;
    let title = meta.title.clone().unwrap_or_else(|| {
        app.playing_index
            .and_then(|i| app.tracks.get(i))
            .map(|t| t.display.clone())
            .unwrap_or_else(|| "—".to_string())
    });
    let artist = meta.artist.clone().unwrap_or_else(|| "—".to_string());
    let album = meta.album.clone().unwrap_or_else(|| "—".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(purple()))
        .title(Line::from(Span::styled(
            " Now Playing ",
            Style::default().fg(pink()).add_modifier(Modifier::BOLD),
        )))
        .title(
            Line::from(vec![
                Span::styled(
                    " ✦ ",
                    Style::default()
                        .fg(yellow())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Spark",
                    Style::default()
                        .fg(pink())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Player ",
                    Style::default()
                        .fg(cyan())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("v{} ", env!("CARGO_PKG_VERSION")),
                    Style::default().fg(dim()),
                ),
            ])
            .alignment(Alignment::Right),
        )
        .padding(Padding::new(1, 1, 0, 0))
        .style(Style::default().bg(panel_bg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Reserve a narrow column on the right for the vertical volume meter.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(7)])
        .split(inner);

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "  TITLE  ",
                Style::default().fg(purple()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                title,
                Style::default()
                    .fg(yellow())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " ARTIST  ",
                Style::default().fg(purple()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(artist, Style::default().fg(pink())),
        ]),
        Line::from(vec![
            Span::styled(
                "  ALBUM  ",
                Style::default().fg(purple()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(album, Style::default().fg(cyan())),
        ]),
        Line::from(vec![
            Span::styled(
                "  INFO   ",
                Style::default().fg(purple()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format_format_line(app), Style::default().fg(dim())),
        ]),
    ];

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // metadata block
            Constraint::Length(1), // spacer above gauge
            Constraint::Length(2), // progress gauge (2 rows tall)
            Constraint::Length(2), // padding between gauge and control badges
            Constraint::Min(0),    // badges
        ])
        .split(cols[0]);

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        inner_layout[0],
    );

    let pos = app.position();
    let dur = app.current_duration.unwrap_or(Duration::ZERO);
    let ratio = if dur.as_secs_f64() > 0.0 {
        (pos.as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = format!(
        " {} / {} ",
        fmt_time(pos),
        if dur.is_zero() {
            String::from("--:--")
        } else {
            fmt_time(dur)
        }
    );

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(pink()).bg(Color::Rgb(50, 30, 70)))
        .ratio(ratio)
        .label(Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, inner_layout[2]);

    let mode_label = if app.player.is_paused() {
        " ⏸ Paused "
    } else if app.playing_index.is_some() {
        " ▶ Playing "
    } else {
        " ⏹ Stopped "
    };
    let mut badges: Vec<Span> = vec![
        Span::styled(
            mode_label,
            Style::default()
                .fg(Color::Black)
                .bg(green())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" Repeat: {} ", app.repeat.label()),
            Style::default()
                .fg(Color::Black)
                .bg(yellow())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" Shuffle: {} ", if app.shuffle { "On" } else { "Off" }),
            Style::default()
                .fg(Color::Black)
                .bg(pink())
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if app.video.is_some() {
        let mode = if app.auto_av_offset { "auto" } else { "manual" };
        badges.push(Span::styled(
            format!(
                " A/V: {:+.0} ms ({}) ",
                app.av_offset_secs * 1000.0,
                mode
            ),
            Style::default()
                .fg(Color::Black)
                .bg(cyan())
                .add_modifier(Modifier::BOLD),
        ));
        if app.subtitles.track_count() > 0 {
            let sub_state = match app.active_subtitle_track {
                Some(i) => app
                    .subtitles
                    .track_label(i)
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                None => "Off".to_string(),
            };
            badges.push(Span::styled(
                format!(" Subs: {} ", sub_state),
                Style::default()
                    .fg(Color::Black)
                    .bg(purple())
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(layout_badges(badges, inner_layout[4].width as usize)),
        inner_layout[4],
    );

    draw_volume_column(frame, cols[1], app.player.volume());
}

fn draw_volume_column(frame: &mut Frame, area: Rect, volume: f32) {
    if area.height < 3 || area.width == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    let pct = (volume * 100.0).round() as i32;
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "VOL",
            Style::default()
                .fg(cyan())
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{:>3}%", pct),
            Style::default()
                .fg(if volume > 1.0 { red() } else { yellow() })
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        rows[1],
    );

    draw_vertical_bar(frame, rows[2], volume);
}

fn draw_vertical_bar(frame: &mut Frame, area: Rect, volume: f32) {
    let h = area.height as usize;
    let w = area.width as usize;
    if h == 0 || w == 0 {
        return;
    }
    // Map 0.0..1.5 (audio range) to 0..h cells with eighth-block precision.
    let level_f = (volume.clamp(0.0, 1.5) / 1.5) * h as f32 * 8.0;
    let full_cells = (level_f / 8.0) as usize;
    let frac = (level_f as usize) % 8;

    let bar_w = w.min(3).max(1);
    let x_start = area.x + ((w - bar_w) / 2) as u16;
    let buf = frame.buffer_mut();

    for col in 0..bar_w {
        let x = x_start + col as u16;
        for row in 0..h {
            let y = area.y + area.height - 1 - row as u16;
            let color = volume_color(row, h);
            let cell = buf.cell_mut((x, y));
            let Some(cell) = cell else { continue };
            if row < full_cells {
                cell.set_char('█');
                cell.set_fg(color);
                cell.set_bg(panel_bg());
            } else if row == full_cells && frac > 0 {
                let ch = match frac {
                    1 => '▁',
                    2 => '▂',
                    3 => '▃',
                    4 => '▄',
                    5 => '▅',
                    6 => '▆',
                    7 => '▇',
                    _ => '█',
                };
                cell.set_char(ch);
                cell.set_fg(color);
                cell.set_bg(panel_bg());
            } else {
                cell.set_char('░');
                cell.set_fg(Color::Rgb(60, 40, 80));
                cell.set_bg(panel_bg());
            }
        }
    }
}

fn volume_color(row: usize, h: usize) -> Color {
    // green low → yellow → pink → red at the very top (over-100%).
    let t = if h == 0 { 0.0 } else { row as f32 / h as f32 };
    if t < 0.45 {
        lerp(green(), yellow(), t / 0.45)
    } else if t < 0.75 {
        lerp(yellow(), pink(), (t - 0.45) / 0.30)
    } else {
        lerp(pink(), red(), (t - 0.75) / 0.25)
    }
}

fn format_format_line(app: &App) -> String {
    let m = &app.current_meta;
    let mut parts: Vec<String> = Vec::new();
    if let Some(sr) = m.sample_rate {
        parts.push(format!("{} Hz", sr));
    }
    if let Some(ch) = m.channels {
        parts.push(format!("{} ch", ch));
    }
    if let Some(br) = m.bitrate {
        parts.push(format!("{} kbps", br));
    }
    if let Some(year) = m.year {
        parts.push(format!("{}", year));
    }
    if parts.is_empty() {
        "—".to_string()
    } else {
        parts.join("  •  ")
    }
}

pub(super) fn draw_album_art(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(purple()))
        .title(Line::from(Span::styled(
            " Album Art ",
            Style::default()
                .fg(yellow())
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(panel_bg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // Read the terminal's actual cell pixel size from the picker so we can
    // fit any image aspect ratio correctly, not just square covers.
    let font_size = app
        .picker
        .as_ref()
        .map(|p| p.font_size())
        .unwrap_or(ratatui_image::FontSize::new(8, 16));
    let font_w = font_size.width;
    let font_h = font_size.height;
    let dims = app.album_dims;

    let Some(proto) = app.album_protocol.as_mut() else {
        return;
    };
    let (iw, ih) = dims.unwrap_or((1, 1));
    let iw = iw.max(1);
    let ih = ih.max(1);

    let avail_w_px = inner.width as u32 * font_w.max(1) as u32;
    let avail_h_px = inner.height as u32 * font_h.max(1) as u32;

    // Scale the image to fit the available panel while preserving aspect.
    let scale = (avail_w_px as f64 / iw as f64).min(avail_h_px as f64 / ih as f64);
    let fit_w_px = (iw as f64 * scale).round() as u32;
    let fit_h_px = (ih as f64 * scale).round() as u32;

    // Round up to whole cells so the image doesn't get truncated at the edges.
    let cells_w = ((fit_w_px + font_w as u32 - 1) / font_w.max(1) as u32)
        .max(1)
        .min(inner.width as u32) as u16;
    let cells_h = ((fit_h_px + font_h as u32 - 1) / font_h.max(1) as u32)
        .max(1)
        .min(inner.height as u32) as u16;

    let x = inner.x + (inner.width - cells_w) / 2;
    let y = inner.y + (inner.height - cells_h) / 2;
    let img_area = Rect::new(x, y, cells_w, cells_h);
    frame.render_stateful_widget(StatefulImage::default(), img_area, proto);
}

pub(super) fn draw_footer(frame: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(purple()))
        .style(Style::default().bg(panel_bg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let key = |k: &str| {
        Span::styled(
            format!(" {k} "),
            Style::default()
                .fg(Color::Black)
                .bg(cyan())
                .add_modifier(Modifier::BOLD),
        )
    };
    let lbl = |s: &str| Span::styled(format!(" {s}  "), Style::default().fg(dim()));

    // Priority-ordered: most important first. We drop entries from the end
    // when the bar wouldn't fit the available width.
    let entries: [(&str, &str); 15] = [
        ("?", "Help"),
        ("←→", "Seek 10s"),
        ("Tab", "Focus"),
        ("Space", "Play/Pause"),
        ("+/-", "Volume"),
        ("v", "Visualizer"),
        ("t", "Theme"),
        ("f", "Fullscreen"),
        ("n", "Next"),
        ("p", "Prev"),
        ("a/A", "Queue"),
        ("C", "Clear"),
        ("r", "Repeat"),
        ("s", "Shuffle"),
        ("q", "Quit"),
    ];

    let avail = inner.width as usize;
    let mut spans: Vec<Span> = Vec::with_capacity(entries.len() * 2);
    let mut used = 0usize;
    for (k, l) in entries {
        // key span is " {k} " → k.chars().count() + 2
        // lbl span is " {l}  " → l.chars().count() + 3
        let cost = k.chars().count() + l.chars().count() + 5;
        if used + cost > avail {
            break;
        }
        spans.push(key(k));
        spans.push(lbl(l));
        used += cost;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn fmt_time(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m >= 60 {
        format!("{}:{:02}:{:02}", m / 60, m % 60, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}
