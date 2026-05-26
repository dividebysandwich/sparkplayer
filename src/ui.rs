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
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    if app.fullscreen_vis {
        if app.video_protocol.is_some() {
            draw_video(frame, outer[0], app);
        } else {
            draw_visualizer(frame, outer[0], app);
        }
        draw_footer(frame, outer[1], app);
        if app.show_help {
            draw_help(frame, area);
        }
        return;
    }

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

    let has_video = app.video_protocol.is_some();

    if has_video {
        // Video gets the larger half of the right column; metadata + visualizer
        // share the bottom.
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(16)])
            .split(body[1]);
        draw_video(frame, right[0], app);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(right[1]);
        draw_now_playing(frame, bottom[0], app);
        draw_visualizer(frame, bottom[1], app);
    } else {
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
    }

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
    let mut badges = vec![
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
    ];
    if app.video.is_some() {
        let mode = if app.auto_av_offset { "auto" } else { "manual" };
        badges.push(Span::raw("  "));
        badges.push(Span::styled(
            format!(
                " A/V: {:+.0} ms ({}) ",
                app.av_offset_secs * 1000.0,
                mode
            ),
            Style::default()
                .fg(Color::Black)
                .bg(NEON_CYAN)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(badges)), inner_layout[4]);

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

fn draw_video(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PURPLE))
        .title(Line::from(Span::styled(
            " Video ",
            Style::default().fg(NEON_YELLOW).add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let font_size = app
        .picker
        .as_ref()
        .map(|p| p.font_size())
        .unwrap_or(ratatui_image::FontSize::new(8, 16));
    let font_w = font_size.width;
    let font_h = font_size.height;
    let dims = app.video_dims;

    let Some(proto) = app.video_protocol.as_mut() else {
        return;
    };
    let (iw, ih) = dims.unwrap_or((1, 1));
    let iw = iw.max(1);
    let ih = ih.max(1);

    let avail_w_px = inner.width as u32 * font_w.max(1) as u32;
    let avail_h_px = inner.height as u32 * font_h.max(1) as u32;

    let scale = (avail_w_px as f64 / iw as f64).min(avail_h_px as f64 / ih as f64);
    let fit_w_px = (iw as f64 * scale).round() as u32;
    let fit_h_px = (ih as f64 * scale).round() as u32;

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
        VisMode::Cassette => {
            if app.video.is_some() {
                draw_vhs(frame, inner, app, active);
            } else {
                draw_cassette(frame, inner, app, active);
            }
        }
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

/// VHS cassette variant of the cassette visualizer, used when the current
/// track is a video. VHS tapes are wider and thinner than compact cassettes,
/// with two big translucent reels behind a clear window and a tape door slot
/// along the bottom edge. Same rotating-spindle technique as the audio
/// cassette.
fn draw_vhs(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let phase = app.visualizer.cassette_phase(&app.player.tap, active);
    let title = app
        .current_meta
        .title
        .clone()
        .or_else(|| {
            app.playing_index
                .and_then(|i| app.tracks.get(i))
                .map(|t| t.display.clone())
        })
        .unwrap_or_else(|| "Untitled".to_string());
    let artist = app
        .current_meta
        .artist
        .clone()
        .unwrap_or_else(|| "Unknown Studio".to_string());

    if area.width < 32 || area.height < 10 {
        return;
    }
    // VHS aspect ~188:104 mm = 1.81:1. With ~2:1 cell aspect, target cells
    // are ~3.6:1 (wide:tall). Slightly squatter than compact cassette.
    let target_w: u16 = 64;
    let w = target_w.min(area.width);
    let target_h = ((w as u32 * 28 + 50) / 100) as u16;
    let h = target_h.clamp(10, area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let body = Rect::new(x, y, w, h);

    let case_color = Color::Rgb(18, 18, 22);
    let case_trim = Color::Rgb(80, 80, 100);
    let label_bg = Color::Rgb(240, 235, 220);
    let label_text = Color::Rgb(25, 25, 50);
    let label_meta = Color::Rgb(160, 50, 50);
    let label_border = Color::Rgb(60, 60, 90);
    let screw_color = Color::Rgb(80, 75, 100);
    let window_frame = Color::Rgb(140, 140, 160);
    let reel_outer = Color::Rgb(230, 230, 240);
    let reel_inner = Color::Rgb(60, 50, 40);
    let door_color = Color::Rgb(8, 8, 12);

    let x0 = body.x;
    let y0 = body.y;
    let x1 = body.x + body.width - 1;
    let y1 = body.y + body.height - 1;

    // Label sits in the upper third; window with reels fills the middle; a
    // tape-door slot runs along the bottom edge.
    let lbl_pad_x = 4u16;
    let lbl_top = y0 + 1;
    let lbl_h = ((h - 4) / 3).max(2).min(4);
    let lx0 = x0 + lbl_pad_x;
    let lx1 = x1 - lbl_pad_x;
    let ly0 = lbl_top;
    let ly1 = ly0 + lbl_h;

    let door_h = 2u16.min(h.saturating_sub(8));
    let door_top = y1.saturating_sub(door_h);

    let win_top = ly1 + 1;
    let win_bot = door_top.saturating_sub(1);
    let win_pad_x = 2u16;
    let wx0 = x0 + win_pad_x;
    let wx1 = x1 - win_pad_x;

    // ----- structural pass: case, label, window frame, screws, door -----
    {
        let buf = frame.buffer_mut();

        // Body fill.
        for yi in body.y..body.y + body.height {
            for xi in body.x..body.x + body.width {
                if let Some(cell) = buf.cell_mut((xi, yi)) {
                    cell.set_char(' ');
                    cell.set_bg(case_color);
                    cell.set_fg(case_trim);
                }
            }
        }

        // Square outer border — VHS shells are blocky, not rounded.
        let corners = [
            (x0, y0, '┌'),
            (x1, y0, '┐'),
            (x0, y1, '└'),
            (x1, y1, '┘'),
        ];
        for (cx, cy, ch) in corners {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_char(ch);
                cell.set_fg(case_trim);
                cell.set_bg(case_color);
            }
        }
        for xi in (x0 + 1)..x1 {
            for &cy in &[y0, y1] {
                if let Some(cell) = buf.cell_mut((xi, cy)) {
                    cell.set_char('─');
                    cell.set_fg(case_trim);
                    cell.set_bg(case_color);
                }
            }
        }
        for yi in (y0 + 1)..y1 {
            for &cx in &[x0, x1] {
                if let Some(cell) = buf.cell_mut((cx, yi)) {
                    cell.set_char('│');
                    cell.set_fg(case_trim);
                    cell.set_bg(case_color);
                }
            }
        }

        // Label paper.
        for yi in ly0..=ly1 {
            for xi in lx0..=lx1 {
                if let Some(cell) = buf.cell_mut((xi, yi)) {
                    cell.set_char(' ');
                    cell.set_bg(label_bg);
                    cell.set_fg(label_text);
                }
            }
        }
        let lbl_corners = [
            (lx0, ly0, '┌'),
            (lx1, ly0, '┐'),
            (lx0, ly1, '└'),
            (lx1, ly1, '┘'),
        ];
        for (cx, cy, ch) in lbl_corners {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_char(ch);
                cell.set_fg(label_border);
                cell.set_bg(label_bg);
            }
        }
        for xi in (lx0 + 1)..lx1 {
            for &cy in &[ly0, ly1] {
                if let Some(cell) = buf.cell_mut((xi, cy)) {
                    cell.set_char('─');
                    cell.set_fg(label_border);
                    cell.set_bg(label_bg);
                }
            }
        }
        for yi in (ly0 + 1)..ly1 {
            for &cx in &[lx0, lx1] {
                if let Some(cell) = buf.cell_mut((cx, yi)) {
                    cell.set_char('│');
                    cell.set_fg(label_border);
                    cell.set_bg(label_bg);
                }
            }
        }

        // Label text. VHS labels typically read "VHS · SP · 120 min" or similar.
        let inner_w = (lx1 - lx0).saturating_sub(2) as usize;
        let inner_x = lx0 + 1;
        if ly1 > ly0 + 1 {
            let header = " ▌ VHS · SP · T-120 ▐ ";
            write_label_centered(
                buf, inner_x, ly0 + 1, inner_w, header, label_meta, label_bg, true,
            );
        }
        if ly1 > ly0 + 2 {
            let title_line = format!("▶ {}", truncate_chars(&title, inner_w.saturating_sub(2)));
            write_label_centered(
                buf,
                inner_x,
                ly0 + 2,
                inner_w,
                &title_line,
                label_text,
                label_bg,
                true,
            );
        }
        if ly1 > ly0 + 3 {
            let artist_line = truncate_chars(&artist, inner_w);
            write_label_centered(
                buf,
                inner_x,
                ly0 + 3,
                inner_w,
                &artist_line,
                label_meta,
                label_bg,
                false,
            );
        }

        // Window frame around the reels.
        if win_bot > win_top + 1 && wx1 > wx0 + 1 {
            for xi in (wx0 + 1)..wx1 {
                if let Some(cell) = buf.cell_mut((xi, win_top)) {
                    cell.set_char('─');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
                if let Some(cell) = buf.cell_mut((xi, win_bot)) {
                    cell.set_char('─');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
            for yi in (win_top + 1)..win_bot {
                if let Some(cell) = buf.cell_mut((wx0, yi)) {
                    cell.set_char('│');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
                if let Some(cell) = buf.cell_mut((wx1, yi)) {
                    cell.set_char('│');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
            for (cx, cy, ch) in [
                (wx0, win_top, '┌'),
                (wx1, win_top, '┐'),
                (wx0, win_bot, '└'),
                (wx1, win_bot, '┘'),
            ] {
                if let Some(cell) = buf.cell_mut((cx, cy)) {
                    cell.set_char(ch);
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
        }

        // Tape door — a darker recessed slot across the bottom edge.
        if door_h > 0 {
            for yi in door_top..y1 {
                for xi in (x0 + 2)..x1.saturating_sub(1) {
                    if let Some(cell) = buf.cell_mut((xi, yi)) {
                        cell.set_char('▀');
                        cell.set_fg(door_color);
                        cell.set_bg(Color::Rgb(40, 35, 50));
                    }
                }
            }
            // A thin tape line peeking out of the door.
            for xi in (x0 + 4)..x1.saturating_sub(3) {
                if let Some(cell) = buf.cell_mut((xi, door_top)) {
                    cell.set_char('▁');
                    cell.set_fg(Color::Rgb(150, 110, 70));
                    cell.set_bg(case_color);
                }
            }
        }

        // Four corner screws.
        let screw_positions = [
            (x0 + 2, y0 + 1),
            (x1 - 2, y0 + 1),
            (x0 + 2, y1 - 1),
            (x1 - 2, y1 - 1),
        ];
        for (sx, sy) in screw_positions {
            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_char('◉');
                cell.set_fg(screw_color);
                cell.set_bg(case_color);
            }
        }
    }

    // ----- canvas pass: two large reels -----
    let inner_w = wx1.saturating_sub(wx0).saturating_sub(1);
    let inner_h = win_bot.saturating_sub(win_top).saturating_sub(1);
    let inner_x = wx0 + 1;
    let inner_y = win_top + 1;

    if inner_w < 16 || inner_h < 3 {
        return;
    }

    // VHS reels are much bigger than cassette spindles — they nearly touch.
    let spindle_h: u16 = inner_h;
    let spindle_w: u16 = (inner_w / 2 - 1).max(7);
    let spindle_y = inner_y;

    let quarter = inner_w / 4;
    let three_q = (inner_w * 3) / 4;
    let half_sw = spindle_w / 2;
    let left_x = inner_x + quarter.saturating_sub(half_sw);
    let right_x = inner_x + three_q.saturating_sub(half_sw);

    // Tape spanning between the two reels (inside the window).
    let tape_y = spindle_y + spindle_h / 2;
    let tape_start = left_x + spindle_w;
    let tape_end = right_x;
    if tape_end > tape_start && tape_y < inner_y + inner_h {
        let tape_rect = Rect::new(tape_start, tape_y, tape_end - tape_start, 1);
        let phase_d = phase as f64;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .background_color(case_color)
            .x_bounds([0.0, 1.0])
            .y_bounds([-1.0, 1.0])
            .paint(move |ctx| {
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 0.0,
                    color: Color::Rgb(120, 90, 60),
                });
                let n = 32;
                let drift = (phase_d * 0.06).rem_euclid(1.0 / n as f64);
                for k in 0..n {
                    let t = k as f64 / n as f64 + drift;
                    ctx.draw(&CanvasLine {
                        x1: t,
                        y1: -0.4,
                        x2: t + 0.005,
                        y2: 0.4,
                        color: Color::Rgb(200, 160, 110),
                    });
                }
            });
        frame.render_widget(canvas, tape_rect);
    }

    for &spindle_x in &[left_x, right_x] {
        let rect = Rect::new(spindle_x, spindle_y, spindle_w, spindle_h);
        let phase_d = phase as f64;
        let h_bound = 1.15f64;
        let w_bound = h_bound * (spindle_w as f64 / spindle_h as f64) / 2.0;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .background_color(case_color)
            .x_bounds([-w_bound, w_bound])
            .y_bounds([-h_bound, h_bound])
            .paint(move |ctx| {
                let seg = 80;
                let circle = |ctx: &mut ratatui::widgets::canvas::Context,
                              r: f64,
                              color: Color| {
                    for k in 0..seg {
                        let a1 = (k as f64 / seg as f64) * std::f64::consts::TAU;
                        let a2 = ((k + 1) as f64 / seg as f64) * std::f64::consts::TAU;
                        ctx.draw(&CanvasLine {
                            x1: a1.cos() * r,
                            y1: a1.sin() * r,
                            x2: a2.cos() * r,
                            y2: a2.sin() * r,
                            color,
                        });
                    }
                };
                // Translucent reel disc.
                circle(ctx, 0.98, reel_outer);
                circle(ctx, 0.90, Color::Rgb(190, 190, 200));
                // Wound tape underneath the disc.
                circle(ctx, 0.82, reel_inner);
                circle(ctx, 0.72, Color::Rgb(80, 60, 50));

                // Hub: VHS reels have a six-tooth gear in the middle.
                let hub_r = 0.34;
                circle(ctx, hub_r, Color::Rgb(40, 40, 50));
                let teeth = 6;
                for t in 0..teeth {
                    let a = phase_d + (t as f64 / teeth as f64) * std::f64::consts::TAU;
                    let r_in = hub_r * 0.6;
                    let r_out = hub_r * 1.05;
                    ctx.draw(&CanvasLine {
                        x1: a.cos() * r_in,
                        y1: a.sin() * r_in,
                        x2: a.cos() * r_out,
                        y2: a.sin() * r_out,
                        color: Color::Rgb(220, 220, 230),
                    });
                }
                // Spoke lines across the reel disc — these are what your eye
                // actually tracks for rotation cues.
                let spokes = 5;
                for s in 0..spokes {
                    let a = phase_d * 0.9 + (s as f64 / spokes as f64) * std::f64::consts::TAU;
                    ctx.draw(&CanvasLine {
                        x1: -a.cos() * 0.20,
                        y1: -a.sin() * 0.20,
                        x2: a.cos() * 0.86,
                        y2: a.sin() * 0.86,
                        color: Color::Rgb(160, 160, 175),
                    });
                }
                // Center peg.
                circle(ctx, 0.10, Color::Rgb(25, 25, 35));
            });
        frame.render_widget(canvas, rect);
    }
}

/// Detailed ASCII cassette tape with two rotating spindles. The case, label
/// and screws are drawn directly into the cell buffer for crisp box-art,
/// while each spindle is a Canvas+Braille widget so the spokes step in
/// quarter-cell subpixel increments instead of jumping a whole cell at a time.
fn draw_cassette(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let phase = app.visualizer.cassette_phase(&app.player.tap, active);
    let title = app
        .current_meta
        .title
        .clone()
        .or_else(|| {
            app.playing_index
                .and_then(|i| app.tracks.get(i))
                .map(|t| t.display.clone())
        })
        .unwrap_or_else(|| "Untitled".to_string());
    let artist = app
        .current_meta
        .artist
        .clone()
        .unwrap_or_else(|| "Unknown Artist".to_string());

    // Cassettes are roughly 5:3 in mm; with cell aspect ~1:2, target ~10:3 in
    // cells. Bail out cleanly when the panel is too small to read; clamp would
    // otherwise panic if min > max.
    if area.width < 30 || area.height < 10 {
        return;
    }
    let target_w: u16 = 64;
    let w = target_w.min(area.width);
    let target_h = ((w as u32 * 3 + 5) / 10) as u16;
    let h = target_h.clamp(10, area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let body = Rect::new(x, y, w, h);

    let case_color = Color::Rgb(35, 28, 55);
    let case_trim = NEON_CYAN;
    let label_bg = Color::Rgb(245, 230, 195);
    let label_text = Color::Rgb(35, 25, 60);
    let label_meta = Color::Rgb(170, 90, 70);
    let label_border = NEON_PINK;
    let screw_color = Color::Rgb(110, 95, 130);
    let tape_color = Color::Rgb(150, 110, 70);
    let window_frame = Color::Rgb(120, 110, 150);

    let x0 = body.x;
    let y0 = body.y;
    let x1 = body.x + body.width - 1;
    let y1 = body.y + body.height - 1;

    // Lay out structural areas before we draw — the spindle window sits in
    // the lower half, the paper label in the upper half.
    let lbl_pad_x = 3u16;
    let lbl_top = y0 + 1;
    let lbl_h = ((h - 2) / 2).max(3).min(6);
    let lx0 = x0 + lbl_pad_x;
    let lx1 = x1 - lbl_pad_x;
    let ly0 = lbl_top;
    let ly1 = ly0 + lbl_h;

    let win_top = ly1 + 1;
    let win_bot = y1 - 1;
    let win_h = win_bot.saturating_sub(win_top);
    let win_pad_x = 2u16;
    let wx0 = x0 + win_pad_x;
    let wx1 = x1 - win_pad_x;

    // ----- direct-buffer pass: case, label, window frame, screws -----
    {
        let buf = frame.buffer_mut();

        // Body fill (case color).
        for yi in body.y..body.y + body.height {
            for xi in body.x..body.x + body.width {
                if let Some(cell) = buf.cell_mut((xi, yi)) {
                    cell.set_char(' ');
                    cell.set_bg(case_color);
                    cell.set_fg(case_trim);
                }
            }
        }

        // Rounded outer border.
        let corners = [
            (x0, y0, '╭'),
            (x1, y0, '╮'),
            (x0, y1, '╰'),
            (x1, y1, '╯'),
        ];
        for (cx, cy, ch) in corners {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_char(ch);
                cell.set_fg(case_trim);
                cell.set_bg(case_color);
            }
        }
        for xi in (x0 + 1)..x1 {
            for &cy in &[y0, y1] {
                if let Some(cell) = buf.cell_mut((xi, cy)) {
                    cell.set_char('─');
                    cell.set_fg(case_trim);
                    cell.set_bg(case_color);
                }
            }
        }
        for yi in (y0 + 1)..y1 {
            for &cx in &[x0, x1] {
                if let Some(cell) = buf.cell_mut((cx, yi)) {
                    cell.set_char('│');
                    cell.set_fg(case_trim);
                    cell.set_bg(case_color);
                }
            }
        }

        // Label paper.
        for yi in ly0..=ly1 {
            for xi in lx0..=lx1 {
                if let Some(cell) = buf.cell_mut((xi, yi)) {
                    cell.set_char(' ');
                    cell.set_bg(label_bg);
                    cell.set_fg(label_text);
                }
            }
        }
        let lbl_corners = [
            (lx0, ly0, '┌'),
            (lx1, ly0, '┐'),
            (lx0, ly1, '└'),
            (lx1, ly1, '┘'),
        ];
        for (cx, cy, ch) in lbl_corners {
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_char(ch);
                cell.set_fg(label_border);
                cell.set_bg(label_bg);
            }
        }
        for xi in (lx0 + 1)..lx1 {
            for &cy in &[ly0, ly1] {
                if let Some(cell) = buf.cell_mut((xi, cy)) {
                    cell.set_char('─');
                    cell.set_fg(label_border);
                    cell.set_bg(label_bg);
                }
            }
        }
        for yi in (ly0 + 1)..ly1 {
            for &cx in &[lx0, lx1] {
                if let Some(cell) = buf.cell_mut((cx, yi)) {
                    cell.set_char('│');
                    cell.set_fg(label_border);
                    cell.set_bg(label_bg);
                }
            }
        }

        // Label text rows.
        let inner_w = (lx1 - lx0).saturating_sub(2) as usize;
        let inner_x = lx0 + 1;
        let side_line = " ✦ SIDE A · TYPE II · 60 MIN ✦ ";
        write_label_centered(buf, inner_x, ly0 + 1, inner_w, side_line, label_meta, label_bg, true);
        let title_line = format!(" ♬ {}", truncate_chars(&title, inner_w.saturating_sub(3)));
        write_label(buf, inner_x, ly0 + 2, inner_w, &title_line, label_text, label_bg, true);
        if ly0 + 3 < ly1 {
            let artist_line = format!("   — {}", truncate_chars(&artist, inner_w.saturating_sub(5)));
            write_label(
                buf,
                inner_x,
                ly0 + 3,
                inner_w,
                &artist_line,
                label_meta,
                label_bg,
                false,
            );
        }
        if ly0 + 4 < ly1 {
            // Faint guideline to suggest a write-on area.
            let dotted: String = std::iter::repeat('·').take(inner_w).collect();
            write_label(
                buf,
                inner_x,
                ly0 + 4,
                inner_w,
                &dotted,
                Color::Rgb(190, 175, 145),
                label_bg,
                false,
            );
        }

        // Window frame around the spindles.
        if win_h >= 3 && wx1 > wx0 {
            for xi in (wx0 + 1)..wx1 {
                if let Some(cell) = buf.cell_mut((xi, win_top)) {
                    cell.set_char('─');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
                if let Some(cell) = buf.cell_mut((xi, win_bot)) {
                    cell.set_char('─');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
            for yi in (win_top + 1)..win_bot {
                if let Some(cell) = buf.cell_mut((wx0, yi)) {
                    cell.set_char('│');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
                if let Some(cell) = buf.cell_mut((wx1, yi)) {
                    cell.set_char('│');
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
            for (cx, cy, ch) in [
                (wx0, win_top, '┌'),
                (wx1, win_top, '┐'),
                (wx0, win_bot, '└'),
                (wx1, win_bot, '┘'),
            ] {
                if let Some(cell) = buf.cell_mut((cx, cy)) {
                    cell.set_char(ch);
                    cell.set_fg(window_frame);
                    cell.set_bg(case_color);
                }
            }
        }

        // Four corner screws — the unmistakable cassette tell.
        let screw_positions = [
            (x0 + 2, y0 + 1),
            (x1 - 2, y0 + 1),
            (x0 + 2, y1 - 1),
            (x1 - 2, y1 - 1),
        ];
        for (sx, sy) in screw_positions {
            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_char('◉');
                cell.set_fg(screw_color);
                cell.set_bg(case_color);
            }
        }

        // Bottom sprocket / pressure pad holes between the two reels.
        let mid_x = (x0 + x1) / 2;
        if y1 >= 2 {
            let pad_y = y1 - 1;
            for off in [-4i32, -2, 0, 2, 4] {
                let px = (mid_x as i32 + off) as u16;
                if px > x0 && px < x1 {
                    if let Some(cell) = buf.cell_mut((px, pad_y)) {
                        cell.set_char('◦');
                        cell.set_fg(Color::Rgb(160, 150, 180));
                        cell.set_bg(case_color);
                    }
                }
            }
        }
    }

    // ----- canvas pass: rotating spindles + connecting tape -----
    // wx0/wx1 and win_top/win_bot are the *border* coordinates of the window
    // box, so inner cell counts subtract one border on each side.
    let inner_w = wx1.saturating_sub(wx0).saturating_sub(1);
    let inner_h = win_bot.saturating_sub(win_top).saturating_sub(1);
    let inner_x = wx0 + 1;
    let inner_y = win_top + 1;

    // Need room for two non-overlapping spindles plus inter-reel tape, and
    // spindle_w <= inner_w / 2 so the quarter-offset placement never wraps.
    if inner_w < 16 || inner_h < 3 {
        return;
    }

    let spindle_w: u16 = 13.min(inner_w / 2 - 1).max(7);
    let spindle_h: u16 = 6.min(inner_h).max(3);
    let spindle_y = inner_y + (inner_h.saturating_sub(spindle_h)) / 2;

    // Place each spindle a quarter of the way in. saturating_sub guards against
    // any future tuning that might push spindle_w past inner_w/2.
    let quarter = inner_w / 4;
    let three_q = (inner_w * 3) / 4;
    let half_sw = spindle_w / 2;
    let left_x = inner_x + quarter.saturating_sub(half_sw);
    let right_x = inner_x + three_q.saturating_sub(half_sw);

    let tape_y = spindle_y + spindle_h / 2;
    let tape_start = left_x + spindle_w;
    let tape_end = right_x;
    if tape_end > tape_start {
        let tape_rect = Rect::new(tape_start, tape_y, tape_end - tape_start, 1);
        let phase_d = phase as f64;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .background_color(case_color)
            .x_bounds([0.0, 1.0])
            .y_bounds([-1.0, 1.0])
            .paint(move |ctx| {
                // Two parallel tape rails.
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: -0.5,
                    x2: 1.0,
                    y2: -0.5,
                    color: tape_color,
                });
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.5,
                    x2: 1.0,
                    y2: 0.5,
                    color: tape_color,
                });
                // Marching speckles to suggest tape motion. Phase advances
                // continuously so each frame nudges the dots a fractional pixel.
                let n = 24;
                let drift = (phase_d * 0.05).rem_euclid(1.0 / n as f64);
                for k in 0..n {
                    let t = k as f64 / n as f64 + drift;
                    ctx.draw(&CanvasLine {
                        x1: t,
                        y1: -0.25,
                        x2: t + 0.005,
                        y2: 0.25,
                        color: Color::Rgb(220, 180, 120),
                    });
                }
            });
        frame.render_widget(canvas, tape_rect);
    }

    // Render each spindle. Both turn in the same direction — tape spools
    // from the supply reel to the take-up reel.
    for &spindle_x in &[left_x, right_x] {
        let rect = Rect::new(spindle_x, spindle_y, spindle_w, spindle_h);
        let phase_d = phase as f64;
        // Aspect-correct bounds: terminal cells are ~2:1 (h:w), so widen the
        // x range to make the spool render as a true circle on the braille grid.
        let h_bound = 1.15f64;
        let w_bound = h_bound * (spindle_w as f64 / spindle_h as f64) / 2.0;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .background_color(case_color)
            .x_bounds([-w_bound, w_bound])
            .y_bounds([-h_bound, h_bound])
            .paint(move |ctx| {
                let seg = 72;
                let circle = |ctx: &mut ratatui::widgets::canvas::Context,
                              r: f64,
                              color: Color| {
                    for k in 0..seg {
                        let a1 = (k as f64 / seg as f64) * std::f64::consts::TAU;
                        let a2 = ((k + 1) as f64 / seg as f64) * std::f64::consts::TAU;
                        ctx.draw(&CanvasLine {
                            x1: a1.cos() * r,
                            y1: a1.sin() * r,
                            x2: a2.cos() * r,
                            y2: a2.sin() * r,
                            color,
                        });
                    }
                };
                // Spool of wound tape.
                circle(ctx, 0.98, Color::Rgb(200, 200, 220));
                circle(ctx, 0.88, Color::Rgb(180, 140, 80));
                circle(ctx, 0.78, Color::Rgb(160, 120, 70));
                // Tape-edge ticks: catch the eye and make rotation legible.
                let ticks = 18;
                for t in 0..ticks {
                    let a = phase_d + (t as f64 / ticks as f64) * std::f64::consts::TAU;
                    ctx.draw(&CanvasLine {
                        x1: a.cos() * 0.78,
                        y1: a.sin() * 0.78,
                        x2: a.cos() * 0.90,
                        y2: a.sin() * 0.90,
                        color: Color::Rgb(110, 80, 50),
                    });
                }
                // Hub gear: spoked wheel that grabs the spool — this is the
                // rotation the user actually reads.
                let hub_r = 0.30;
                circle(ctx, hub_r, NEON_YELLOW);
                let spokes = 6;
                for s in 0..spokes {
                    let a = phase_d + (s as f64 / spokes as f64) * std::f64::consts::TAU;
                    ctx.draw(&CanvasLine {
                        x1: -a.cos() * hub_r * 0.2,
                        y1: -a.sin() * hub_r * 0.2,
                        x2: a.cos() * 0.72,
                        y2: a.sin() * 0.72,
                        color: NEON_CYAN,
                    });
                }
                // Center peg.
                circle(ctx, 0.10, Color::Rgb(40, 30, 60));
                circle(ctx, 0.05, NEON_PINK);
            });
        frame.render_widget(canvas, rect);
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn write_label(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    max_w: usize,
    text: &str,
    fg: Color,
    bg: Color,
    bold: bool,
) {
    let mut col = x;
    let mut written = 0usize;
    for ch in text.chars() {
        if written >= max_w {
            break;
        }
        if let Some(cell) = buf.cell_mut((col, y)) {
            let mut style = Style::default().fg(fg).bg(bg);
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            cell.set_style(style);
            cell.set_char(ch);
        }
        col += 1;
        written += 1;
    }
}

fn write_label_centered(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    max_w: usize,
    text: &str,
    fg: Color,
    bg: Color,
    bold: bool,
) {
    let len = text.chars().count().min(max_w);
    let pad = (max_w.saturating_sub(len)) / 2;
    write_label(buf, x + pad as u16, y, max_w - pad, text, fg, bg, bold);
}

fn draw_footer(frame: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(NEON_PURPLE))
        .style(Style::default().bg(BG_PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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

    // Priority-ordered: most important first. We drop entries from the end
    // when the bar wouldn't fit the available width.
    let entries: [(&str, &str); 14] = [
        ("?", "Help"),
        ("←→", "Seek 10s"),
        ("Space", "Play/Pause"),
        ("+/-", "Volume"),
        ("v", "Visualizer"),
        ("f", "Fullscreen"),
        ("n", "Next"),
        ("p", "Prev"),
        ("a/A", "Queue"),
        ("C", "Clear"),
        ("r", "Repeat"),
        ("s", "Shuffle"),
        ("Tab", "Focus"),
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
        Line::from("  [ / ]          A/V sync offset -25ms / +25ms (video)"),
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
        Line::from("                  spectrogram → stereo X/Y →"),
        Line::from("                  spectrum 3D → cassette tape"),
        Line::from("  f              toggle fullscreen visualizer"),
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
