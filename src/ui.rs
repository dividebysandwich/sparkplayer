use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Wrap,
    canvas::{Canvas, Line as CanvasLine},
};
use ratatui_image::StatefulImage;

use crate::app::{App, FocusPane};
use crate::visualizer::VisMode;

const NEON_PINK: Color = Color::Rgb(255, 89, 194);
const NEON_CYAN: Color = Color::Rgb(0, 229, 255);
const NEON_PURPLE: Color = Color::Rgb(170, 102, 255);
const NEON_YELLOW: Color = Color::Rgb(255, 217, 102);
const NEON_GREEN: Color = Color::Rgb(102, 255, 178);
const NEON_RED: Color = Color::Rgb(255, 90, 120);
const DIM_TEXT: Color = Color::Rgb(180, 180, 200);
const BG_PANEL: Color = Color::Rgb(20, 20, 35);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(area);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(outer[0]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[0]);

    draw_playlist(frame, left[0], app);
    draw_browser(frame, left[1], app);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(16), Constraint::Min(8)])
        .split(body[1]);

    let has_art = app.album_protocol.is_some();
    if has_art {
        let top_row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
            .split(right[0]);
        draw_album_art(frame, top_row[0], app);
        draw_now_playing(frame, top_row[1], app);
    } else {
        draw_now_playing(frame, right[0], app);
    }
    draw_visualizer(frame, right[1], app);

    draw_footer(frame, outer[1], app);

    if app.show_help {
        draw_help(frame, area);
    }
}

fn draw_playlist(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == FocusPane::Playlist;
    let border = if focused { NEON_PINK } else { NEON_PURPLE };

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let playing = Some(i) == app.playing_index;
            let prefix = if playing { "▶ " } else { "  " };
            let style = if playing {
                Style::default()
                    .fg(NEON_YELLOW)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(220, 220, 240))
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(NEON_CYAN)),
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
                    Style::default().fg(NEON_PINK).add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(BG_PANEL)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(70, 35, 90))
                .fg(NEON_CYAN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("➤ ");

    frame.render_stateful_widget(list, area, &mut app.playlist_state);
}

fn draw_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == FocusPane::Browser;
    let border = if focused { NEON_PINK } else { NEON_PURPLE };
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
                Style::default().fg(NEON_CYAN)
            } else {
                Style::default().fg(Color::Rgb(220, 220, 240))
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
                        .fg(NEON_CYAN)
                        .add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(BG_PANEL)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(30, 60, 80))
                .fg(NEON_YELLOW)
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

fn draw_now_playing(frame: &mut Frame, area: Rect, app: &App) {
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
        .border_style(Style::default().fg(NEON_PURPLE))
        .title(Line::from(Span::styled(
            " Now Playing ",
            Style::default().fg(NEON_PINK).add_modifier(Modifier::BOLD),
        )))
        .title(
            Line::from(vec![
                Span::styled(
                    " ✦ ",
                    Style::default()
                        .fg(NEON_YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Spark",
                    Style::default()
                        .fg(NEON_PINK)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Player ",
                    Style::default()
                        .fg(NEON_CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("v{} ", env!("CARGO_PKG_VERSION")),
                    Style::default().fg(DIM_TEXT),
                ),
            ])
            .alignment(Alignment::Right),
        )
        .padding(Padding::new(1, 1, 0, 0))
        .style(Style::default().bg(BG_PANEL));
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
                Style::default().fg(NEON_PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                title,
                Style::default()
                    .fg(NEON_YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " ARTIST  ",
                Style::default().fg(NEON_PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(artist, Style::default().fg(NEON_PINK)),
        ]),
        Line::from(vec![
            Span::styled(
                "  ALBUM  ",
                Style::default().fg(NEON_PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(album, Style::default().fg(NEON_CYAN)),
        ]),
        Line::from(vec![
            Span::styled(
                "  INFO   ",
                Style::default().fg(NEON_PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format_format_line(app), Style::default().fg(DIM_TEXT)),
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
        .gauge_style(Style::default().fg(NEON_PINK).bg(Color::Rgb(50, 30, 70)))
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
    let line = Line::from(vec![
        Span::styled(
            mode_label,
            Style::default()
                .fg(Color::Black)
                .bg(NEON_GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" Repeat: {} ", app.repeat.label()),
            Style::default()
                .fg(Color::Black)
                .bg(NEON_YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" Shuffle: {} ", if app.shuffle { "On" } else { "Off" }),
            Style::default()
                .fg(Color::Black)
                .bg(NEON_PINK)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), inner_layout[4]);

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
                .fg(NEON_CYAN)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{:>3}%", pct),
            Style::default()
                .fg(if volume > 1.0 { NEON_RED } else { NEON_YELLOW })
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
                cell.set_bg(BG_PANEL);
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
                cell.set_bg(BG_PANEL);
            } else {
                cell.set_char('░');
                cell.set_fg(Color::Rgb(60, 40, 80));
                cell.set_bg(BG_PANEL);
            }
        }
    }
}

fn volume_color(row: usize, h: usize) -> Color {
    // green low → yellow → pink → red at the very top (over-100%).
    let t = if h == 0 { 0.0 } else { row as f32 / h as f32 };
    if t < 0.45 {
        lerp(NEON_GREEN, NEON_YELLOW, t / 0.45)
    } else if t < 0.75 {
        lerp(NEON_YELLOW, NEON_PINK, (t - 0.45) / 0.30)
    } else {
        lerp(NEON_PINK, NEON_RED, (t - 0.75) / 0.25)
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

fn draw_album_art(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PURPLE))
        .title(Line::from(Span::styled(
            " Album Art ",
            Style::default()
                .fg(NEON_YELLOW)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // Read the terminal's actual cell pixel size from the picker so we can
    // fit any image aspect ratio correctly, not just square covers.
    let (font_w, font_h) = app
        .picker
        .as_ref()
        .map(|p| p.font_size())
        .unwrap_or((8, 16));
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

fn draw_visualizer(frame: &mut Frame, area: Rect, app: &mut App) {
    let mode = app.visualizer.mode;
    let active = app.playing_index.is_some() && !app.player.is_paused();
    let title = format!(" Visualizer — {} ", mode.label());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PURPLE))
        .title(Line::from(Span::styled(
            title,
            Style::default()
                .fg(NEON_CYAN)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 4 || inner.height < 3 {
        return;
    }

    match mode {
        VisMode::Spectrum => draw_spectrum(frame, inner, app, active),
        VisMode::Waveform => draw_waveform(frame, inner, app, active),
        VisMode::ScrollingWaveform => draw_scrolling_waveform(frame, inner, app, active),
        VisMode::Spectrogram => draw_spectrogram(frame, inner, app, active),
        VisMode::Lissajous => draw_lissajous(frame, inner, app, active),
        VisMode::Spectrum3D => draw_spectrum_3d(frame, inner, app, active),
    }
}

fn draw_spectrum(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let bar_width: u16 = 2;
    let bars = (area.width / bar_width).max(1) as usize;
    let sr = app.player.tap.sample_rate();
    let mags = app.visualizer.spectrum(&app.player.tap, bars, sr, active);
    let h = area.height as usize;
    let buf = frame.buffer_mut();
    for (i, m) in mags.iter().enumerate() {
        let x = area.x + (i as u16) * bar_width;
        if x >= area.x + area.width {
            break;
        }
        let bar_h_f = (m * h as f32 * 8.0).max(0.0);
        let full_cells = (bar_h_f / 8.0) as usize;
        let frac_eighths = (bar_h_f as usize) % 8;
        for row in 0..h {
            let y = area.y + area.height - 1 - row as u16;
            let color = bar_color(row, h);
            if row < full_cells {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char('█');
                    cell.set_fg(color);
                    cell.set_bg(BG_PANEL);
                }
            } else if row == full_cells && frac_eighths > 0 {
                let ch = match frac_eighths {
                    1 => '▁',
                    2 => '▂',
                    3 => '▃',
                    4 => '▄',
                    5 => '▅',
                    6 => '▆',
                    7 => '▇',
                    _ => '█',
                };
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                    cell.set_bg(BG_PANEL);
                }
            }
        }
    }
}

fn bar_color(row: usize, h: usize) -> Color {
    if h == 0 {
        return NEON_GREEN;
    }
    let t = row as f32 / h as f32;
    if t < 0.4 {
        lerp(NEON_GREEN, NEON_YELLOW, t / 0.4)
    } else if t < 0.75 {
        lerp(NEON_YELLOW, NEON_PINK, (t - 0.4) / 0.35)
    } else {
        lerp(NEON_PINK, NEON_PURPLE, (t - 0.75) / 0.25)
    }
}

fn lerp(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let t = t.clamp(0.0, 1.0);
    let r = (ar as f32 + (br as f32 - ar as f32) * t) as u8;
    let g = (ag as f32 + (bg as f32 - ag as f32) * t) as u8;
    let b = (ab as f32 + (bb as f32 - ab as f32) * t) as u8;
    Color::Rgb(r, g, b)
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

fn draw_waveform(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    let points = app.visualizer.waveform(&app.player.tap, w, active);
    draw_amplitude_strip(frame, area, &points, w, h);
}

fn draw_amplitude_strip(frame: &mut Frame, area: Rect, points: &[f32], w: usize, h: usize) {
    if h == 0 || w == 0 {
        return;
    }
    let mid = h / 2;
    let buf = frame.buffer_mut();
    for x in 0..w {
        if let Some(cell) = buf.cell_mut((area.x + x as u16, area.y + mid as u16)) {
            cell.set_char('·');
            cell.set_fg(Color::Rgb(60, 60, 90));
            cell.set_bg(BG_PANEL);
        }
    }
    for (i, p) in points.iter().enumerate() {
        let amp = (*p * mid as f32) as usize;
        let amp = amp.min(mid);
        for d in 0..=amp {
            let color = lerp(NEON_CYAN, NEON_PINK, d as f32 / mid.max(1) as f32);
            let yu = mid.saturating_sub(d);
            let yd = (mid + d).min(h - 1);
            if let Some(cell) = buf.cell_mut((area.x + i as u16, area.y + yu as u16)) {
                cell.set_char(if d == amp { '▀' } else { '█' });
                cell.set_fg(color);
                cell.set_bg(BG_PANEL);
            }
            if let Some(cell) = buf.cell_mut((area.x + i as u16, area.y + yd as u16)) {
                cell.set_char(if d == amp { '▄' } else { '█' });
                cell.set_fg(color);
                cell.set_bg(BG_PANEL);
            }
        }
    }
}

/// Scrolling waveform rendered via a Braille-marker Canvas, giving 2× horizontal
/// and 4× vertical sub-cell precision for much finer peak detail.
fn draw_scrolling_waveform(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    // Two columns of braille dots per terminal cell -> oversample by 2.
    let dots = w.saturating_mul(2).max(2);
    let points = app.visualizer.scrolling_waveform(&app.player.tap, dots, active);
    // Slight headroom so peaks at 1.0 don't clip into the top border.
    let y_max = 1.05f64;
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(BG_PANEL)
        .x_bounds([0.0, dots as f64])
        .y_bounds([-y_max, y_max])
        .paint(move |ctx| {
            // Faint zero-line.
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 0.0,
                x2: dots as f64,
                y2: 0.0,
                color: Color::Rgb(60, 60, 90),
            });
            // Vertical peak line per dot-column, colored by amplitude.
            for (i, p) in points.iter().enumerate() {
                let pv = *p as f64;
                if pv < 1e-4 {
                    continue;
                }
                let color = lerp(NEON_CYAN, NEON_PINK, *p);
                ctx.draw(&CanvasLine {
                    x1: i as f64,
                    y1: -pv,
                    x2: i as f64,
                    y2: pv,
                    color,
                });
            }
        });
    frame.render_widget(canvas, area);
}

fn draw_spectrogram(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    let sr = app.player.tap.sample_rate();
    let cols = app.visualizer.spectrogram(&app.player.tap, w, h, sr, active);
    let buf = frame.buffer_mut();
    let n_cols = cols.len();
    let start_x = (area.width as usize).saturating_sub(n_cols);
    for (xi, col) in cols.iter().enumerate() {
        let x = area.x + (start_x + xi) as u16;
        if x >= area.x + area.width {
            break;
        }
        for (yi, mag) in col.iter().enumerate() {
            if yi >= h {
                break;
            }
            let y = area.y + area.height - 1 - yi as u16;
            let color = heatmap(*mag);
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('█');
                cell.set_fg(color);
                cell.set_bg(BG_PANEL);
            }
        }
    }
}

/// Stereo X/Y oscillogram (vectorscope). Left channel drives X, right channel
/// drives Y. Mono signals trace a diagonal; in-phase stereo widens the cloud
/// vertically, out-of-phase stretches it horizontally.
fn draw_lissajous(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    let pts = app.visualizer.lissajous(&app.player.tap, 2048, active);
    // Keep the plot square in pixel terms: cells are ~2:1, so the X bounds
    // are twice the Y bounds and we letterbox via the Canvas bounds.
    let cell_aspect = 2.0f64;
    let h_unit = 1.05f64;
    let w_unit = h_unit * (w as f64 / h as f64) / cell_aspect;
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(BG_PANEL)
        .x_bounds([-w_unit, w_unit])
        .y_bounds([-h_unit, h_unit])
        .paint(move |ctx| {
            // Reference frame: faint axes + the unit square.
            let axis = Color::Rgb(50, 50, 80);
            ctx.draw(&CanvasLine {
                x1: -1.0,
                y1: 0.0,
                x2: 1.0,
                y2: 0.0,
                color: axis,
            });
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: -1.0,
                x2: 0.0,
                y2: 1.0,
                color: axis,
            });
            let frame_col = Color::Rgb(40, 30, 70);
            ctx.draw(&CanvasLine {
                x1: -1.0,
                y1: -1.0,
                x2: 1.0,
                y2: -1.0,
                color: frame_col,
            });
            ctx.draw(&CanvasLine {
                x1: -1.0,
                y1: 1.0,
                x2: 1.0,
                y2: 1.0,
                color: frame_col,
            });
            ctx.draw(&CanvasLine {
                x1: -1.0,
                y1: -1.0,
                x2: -1.0,
                y2: 1.0,
                color: frame_col,
            });
            ctx.draw(&CanvasLine {
                x1: 1.0,
                y1: -1.0,
                x2: 1.0,
                y2: 1.0,
                color: frame_col,
            });

            // Connect consecutive (L, R) pairs to draw a continuous oscilloscope
            // trace. Color by radial magnitude so quieter passages stay cool and
            // peaks blaze pink.
            for w in pts.windows(2) {
                let (x1, y1) = (w[0].0 as f64, w[0].1 as f64);
                let (x2, y2) = (w[1].0 as f64, w[1].1 as f64);
                let r = ((x1 * x1 + y1 * y1).sqrt()).min(1.0) as f32;
                let color = lerp(NEON_CYAN, NEON_PINK, r);
                ctx.draw(&CanvasLine {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                });
            }
        });
    frame.render_widget(canvas, area);
}

/// Pseudo-3D spectrum waterfall. Many recent FFT rows stacked from front
/// (newest, near the bottom-left) to back (oldest, up and to the right),
/// with each silhouette colored by bin amplitude on a blue → yellow → red
/// ramp and dimmed as it recedes for depth cueing.
fn draw_spectrum_3d(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    // Full-resolution bins across the braille subpixel grid.
    let bins = w.saturating_mul(2).max(8);
    // Pack a dense stack of history rows — the reference look depends on lots
    // of overlapping silhouettes.
    let depth_rows = (h.saturating_mul(2)).clamp(18, 40);
    let sr = app.player.tap.sample_rate();
    let rows = app.visualizer.spectrum_3d(&app.player.tap, bins, sr, depth_rows, active);
    if rows.is_empty() {
        return;
    }
    let n_rows = rows.len();

    // The canvas is wider than one row so older rows can park to the right.
    let x_units = bins as f64;
    let shift_max = x_units * 0.22;
    let total_x = x_units + shift_max;
    let y_top = 0.62; // how far up the back row sits
    let amp_scale = 0.45; // height of a full-scale peak above its row's baseline

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(BG_PANEL)
        .x_bounds([0.0, total_x])
        .y_bounds([0.0, 1.0])
        .paint(move |ctx| {
            // Oldest first so newer rows visually overdraw the older ones.
            for (i, row) in rows.iter().enumerate() {
                let depth = if n_rows > 1 {
                    i as f64 / (n_rows - 1) as f64
                } else {
                    1.0
                };
                let y_baseline = (1.0 - depth) * y_top;
                let x_shift = (1.0 - depth) * shift_max;
                // Back rows are dimmer; the front row blazes at full brightness.
                let brightness = (0.30 + 0.70 * depth) as f32;

                let m = row.len();
                if m < 2 {
                    continue;
                }
                for bin in 0..m - 1 {
                    let m1 = row[bin].clamp(0.0, 1.0);
                    let m2 = row[bin + 1].clamp(0.0, 1.0);
                    let x1 = x_shift + bin as f64;
                    let x2 = x_shift + (bin + 1) as f64;
                    let y1 = y_baseline + m1 as f64 * amp_scale;
                    let y2 = y_baseline + m2 as f64 * amp_scale;
                    let avg = (m1 + m2) * 0.5;
                    let base = spectrum_3d_color(avg);
                    let color = scale_color(base, brightness);
                    ctx.draw(&CanvasLine {
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                    });
                }
            }
        });
    frame.render_widget(canvas, area);
}

/// Blue → cyan → yellow → orange → red ramp keyed to bin amplitude.
fn spectrum_3d_color(t: f32) -> Color {
    let stops: [(f32, (u8, u8, u8)); 5] = [
        (0.00, (20, 50, 170)),
        (0.30, (50, 190, 230)),
        (0.55, (255, 230, 80)),
        (0.80, (255, 130, 30)),
        (1.00, (220, 40, 40)),
    ];
    let t = t.clamp(0.0, 1.0);
    for i in 1..stops.len() {
        if t <= stops[i].0 {
            let (lo, hi) = (stops[i - 1].0, stops[i].0);
            let k = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
            let (r1, g1, b1) = stops[i - 1].1;
            let (r2, g2, b2) = stops[i].1;
            let r = (r1 as f32 + (r2 as f32 - r1 as f32) * k) as u8;
            let g = (g1 as f32 + (g2 as f32 - g1 as f32) * k) as u8;
            let b = (b1 as f32 + (b2 as f32 - b1 as f32) * k) as u8;
            return Color::Rgb(r, g, b);
        }
    }
    let (r, g, b) = stops[stops.len() - 1].1;
    Color::Rgb(r, g, b)
}

fn scale_color(c: Color, factor: f32) -> Color {
    let factor = factor.clamp(0.0, 1.5);
    let (r, g, b) = rgb(c);
    let r = (r as f32 * factor).clamp(0.0, 255.0) as u8;
    let g = (g as f32 * factor).clamp(0.0, 255.0) as u8;
    let b = (b as f32 * factor).clamp(0.0, 255.0) as u8;
    Color::Rgb(r, g, b)
}

fn heatmap(t: f32) -> Color {
    let stops: [(f32, (u8, u8, u8)); 6] = [
        (0.00, (10, 8, 25)),
        (0.20, (60, 15, 90)),
        (0.40, (140, 30, 110)),
        (0.60, (220, 60, 90)),
        (0.80, (250, 140, 60)),
        (1.00, (255, 230, 120)),
    ];
    let t = t.clamp(0.0, 1.0);
    for i in 1..stops.len() {
        if t <= stops[i].0 {
            let (lo, hi) = (stops[i - 1].0, stops[i].0);
            let k = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
            let (r1, g1, b1) = stops[i - 1].1;
            let (r2, g2, b2) = stops[i].1;
            let r = (r1 as f32 + (r2 as f32 - r1 as f32) * k) as u8;
            let g = (g1 as f32 + (g2 as f32 - g1 as f32) * k) as u8;
            let b = (b1 as f32 + (b2 as f32 - b1 as f32) * k) as u8;
            return Color::Rgb(r, g, b);
        }
    }
    let (r, g, b) = stops[stops.len() - 1].1;
    Color::Rgb(r, g, b)
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PURPLE))
        .style(Style::default().bg(BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    // Status line (top).
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " ◆ ",
                Style::default().fg(NEON_YELLOW),
            ),
            Span::styled(app.status.clone(), Style::default().fg(NEON_GREEN)),
        ])),
        rows[0],
    );

    let key = |k: &str| {
        Span::styled(
            format!(" {k} "),
            Style::default()
                .fg(Color::Black)
                .bg(NEON_CYAN)
                .add_modifier(Modifier::BOLD),
        )
    };
    let lbl = |s: &str| Span::styled(format!(" {s}  "), Style::default().fg(DIM_TEXT));

    let controls = Line::from(vec![
        key("Space"),
        lbl("Play/Pause"),
        key("n"),
        lbl("Next"),
        key("p"),
        lbl("Prev"),
        key("v"),
        lbl("Visualizer"),
        key("r"),
        lbl("Repeat"),
        key("s"),
        lbl("Shuffle"),
        key("a/A"),
        lbl("Queue"),
        key("C"),
        lbl("Clear"),
        key("←→"),
        lbl("Seek 10s"),
        key("+/-"),
        lbl("Volume"),
        key("Tab"),
        lbl("Focus"),
        key("?"),
        lbl("Help"),
        key("q"),
        lbl("Quit"),
    ]);
    frame.render_widget(Paragraph::new(controls), rows[1]);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let w = area.width.min(66);
    let h = area.height.min(30);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect::new(x, y, w, h);
    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PINK).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            " ✦ SparkPlayer Help ",
            Style::default()
                .fg(NEON_YELLOW)
                .add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(Color::Rgb(15, 10, 30)));

    let body = vec![
        Line::from(Span::styled(
            "Playback",
            Style::default().fg(NEON_CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Space          play / pause"),
        Line::from("  n / p          next / previous track"),
        Line::from("  ← / →          seek -10s / +10s"),
        Line::from("  Ctrl+← / Ctrl+→  seek -30s / +30s"),
        Line::from("  + / = / -      volume up / up / down"),
        Line::from("  Enter          play selection"),
        Line::from(""),
        Line::from(Span::styled(
            "Navigation",
            Style::default().fg(NEON_CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓          move selection"),
        Line::from("  PgUp / PgDn    page selection"),
        Line::from("  Home / End     jump to first / last"),
        Line::from("  Tab            switch focus (playlist ↔ browser)"),
        Line::from(""),
        Line::from(Span::styled(
            "Modes",
            Style::default().fg(NEON_CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from("  v              cycle visualizer:"),
        Line::from("                  FFT bars → waveform → scrolling →"),
        Line::from("                  spectrogram → stereo X/Y → spectrum 3D"),
        Line::from("  r              cycle repeat (off / all / one)"),
        Line::from("  s              shuffle remaining tracks"),
        Line::from(""),
        Line::from(Span::styled(
            "Playlist",
            Style::default().fg(NEON_CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from("  a              queue the highlighted browser item"),
        Line::from("  Shift+A        queue every audio file under the current dir"),
        Line::from("  Shift+C        clear the playlist (stops playback)"),
        Line::from(""),
        Line::from("  ? or h         this help    •    q or Esc    quit"),
    ];

    frame.render_widget(Paragraph::new(body).block(block), rect);
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
