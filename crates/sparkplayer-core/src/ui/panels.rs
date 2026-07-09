//! The main layout panels: playlist, file browser, "Now Playing" metadata
//! block with its volume column, album art, and the keybinding footer.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap};

use crate::app::{App, FocusPane, InputMode, MouseControl};

use super::palette::{cyan, dim, green, lerp, panel_bg, pink, purple, red, text, yellow};

pub(super) fn draw_playlist(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == FocusPane::Playlist;
    let border = if focused { pink() } else { purple() };

    let vis = app.visible_indices(FocusPane::Playlist);
    let items: Vec<ListItem> = vis
        .iter()
        .map(|&i| {
            let t = &app.tracks[i];
            let playing = Some(i) == app.playing_index;
            let prefix = if playing { "▶ " } else { "  " };
            let fav = if app.is_favorite(&t.source) { "★ " } else { "" };
            let style = if playing {
                Style::default()
                    .fg(yellow())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(text())
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(cyan())),
                Span::styled(fav, Style::default().fg(yellow())),
                Span::styled(t.display.clone(), style),
            ]))
        })
        .collect();

    // The ListState selection indexes the *rendered* (filtered) list, so map the
    // full-list cursor to its position among the visible rows.
    let sel_pos = vis.iter().position(|&i| i == app.selected);
    app.playlist_state.select(if items.is_empty() { None } else { sel_pos.or(Some(0)) });

    let playlist_filter = app.filter_pane == FocusPane::Playlist && !app.filter_query.is_empty();
    let title = if app.input_mode == InputMode::SavePlaylist {
        format!(" 💾 Save as: {}▏ ", app.input_buffer)
    } else if playlist_filter {
        format!(
            " ♪ Playlist ({}/{})  /{}{} ",
            vis.len(),
            app.tracks.len(),
            app.filter_query,
            if app.input_mode == InputMode::Filter { "▏" } else { "" }
        )
    } else {
        format!(" ♪ Playlist ({}) ", app.tracks.len())
    };
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

    // Record the inner (bordered) row area for click hit-testing.
    app.playlist_hit = bordered_inner(area);
    frame.render_stateful_widget(list, area, &mut app.playlist_state);
}

/// The content rect inside a full (`Borders::ALL`) block: `area` inset by one
/// cell on every side. `None` when the area is too small to have an interior.
fn bordered_inner(area: Rect) -> Option<Rect> {
    if area.width < 3 || area.height < 3 {
        return None;
    }
    Some(Rect::new(
        area.x + 1,
        area.y + 1,
        area.width - 2,
        area.height - 2,
    ))
}

pub(super) fn draw_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    // Record the panel rect so the web build can float the file-picker buttons
    // over it (the browser pane is empty in the browser).
    app.last_browser_rect = Some(area);
    let focused = app.focus == FocusPane::Browser;
    let border = if focused { pink() } else { purple() };
    let cwd = app.browser_dir.display().to_string();
    let has_parent = app.browser_dir.parent().is_some();

    let vis = app.visible_indices(FocusPane::Browser);
    let items: Vec<ListItem> = vis
        .iter()
        .map(|&i| {
            let p = &app.browser_entries[i];
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

    let sel_pos = vis.iter().position(|&i| i == app.browser_selected);
    app.browser_state.select(if items.is_empty() { None } else { sel_pos.or(Some(0)) });

    let browser_filter = app.filter_pane == FocusPane::Browser && !app.filter_query.is_empty();
    let title = if browser_filter {
        format!(
            " 📂 {}  /{}{} ",
            truncate_path(&cwd, area.width as usize / 2),
            app.filter_query,
            if app.input_mode == InputMode::Filter { "▏" } else { "" }
        )
    } else {
        format!(" 📂 {} ", truncate_path(&cwd, area.width as usize))
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .title(Line::from(Span::styled(
                    title,
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

    app.browser_hit = bordered_inner(area);
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

/// Flow the badges into centered lines that fit `max_width`, and compute the
/// on-screen rect of each badge that carries a `MouseControl` for click
/// hit-testing. The rects account for the centering offset (matching ratatui's
/// `Alignment::Center`), so the caller must render the lines centered.
fn layout_badges<'a>(
    badges: Vec<(Span<'a>, Option<MouseControl>)>,
    max_width: usize,
    origin: Rect,
) -> (Vec<Line<'a>>, Vec<(Rect, MouseControl)>) {
    const SEP: &str = "  ";
    let sep_w = SEP.len();
    // First group the badges (with their width + action) into fitting rows.
    type Placed<'a> = (Span<'a>, usize, Option<MouseControl>);
    let mut rows: Vec<Vec<Placed<'a>>> = vec![Vec::new()];
    let mut current_w = 0usize;
    for (badge, action) in badges {
        let w = badge.width();
        let empty = rows.last().unwrap().is_empty();
        let needed = if empty { w } else { current_w + sep_w + w };
        if needed > max_width && !empty {
            rows.push(Vec::new());
            current_w = 0;
        }
        let row = rows.last_mut().unwrap();
        if !row.is_empty() {
            row.push((Span::raw(SEP), sep_w, None));
            current_w += sep_w;
        }
        current_w += w;
        row.push((badge, w, action));
    }

    // Emit each row centered, placing hit rects at the centered x position.
    let mut lines: Vec<Line<'a>> = Vec::new();
    let mut hits: Vec<(Rect, MouseControl)> = Vec::new();
    let avail = origin.width as usize;
    let mut y = origin.y;
    for row in rows {
        if row.is_empty() {
            continue;
        }
        let line_w: usize = row.iter().map(|(_, w, _)| *w).sum();
        // Matches ratatui's centered alignment: floor((avail - line_w) / 2).
        let center_off = avail.saturating_sub(line_w) / 2;
        let mut x = origin.x as usize + center_off;
        let mut spans: Vec<Span> = Vec::with_capacity(row.len());
        let in_bounds = y < origin.y.saturating_add(origin.height);
        for (span, w, action) in row {
            if let (true, Some(action)) = (in_bounds, action) {
                hits.push((Rect::new(x as u16, y, w as u16, 1), action));
            }
            x += w;
            spans.push(span);
        }
        lines.push(Line::from(spans));
        y = y.saturating_add(1);
    }
    (lines, hits)
}

pub(super) fn draw_now_playing(frame: &mut Frame, area: Rect, app: &mut App) {
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
            Constraint::Length(4), // 0: metadata block
            Constraint::Length(1), // 1: spacer above gauge
            Constraint::Length(2), // 2: progress gauge (2 rows tall)
            Constraint::Length(1), // 3: spacer
            Constraint::Length(1), // 4: transport buttons
            Constraint::Length(1), // 5: spacer
            Constraint::Min(0),    // 6: status badges
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
    let total = if dur.is_zero() {
        String::from("--:--")
    } else {
        fmt_time(dur)
    };
    // Record the seek bar (first row of the gauge) for click/drag scrubbing.
    app.progress_hit = Some(Rect::new(
        inner_layout[2].x,
        inner_layout[2].y,
        inner_layout[2].width,
        1,
    ));
    draw_progress_bar(
        frame,
        inner_layout[2],
        ratio,
        &fmt_time(pos),
        &total,
        app.clock_secs as f32,
    );

    // Cassette-deck transport row: rewind · stop · play/pause · fast-forward.
    let mut control_hits = draw_transport(frame, inner_layout[4], app);

    let badge = |text: String, bg: Color| {
        Span::styled(
            text,
            Style::default()
                .fg(Color::Black)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )
    };
    // Status badges, below the transport row. Each carries the action it fires.
    let mut badges: Vec<(Span, Option<MouseControl>)> = vec![
        (
            badge(format!(" Repeat: {} ", app.repeat.label()), yellow()),
            Some(MouseControl::CycleRepeat),
        ),
        (
            badge(
                format!(" Shuffle: {} ", if app.shuffle { "On" } else { "Off" }),
                pink(),
            ),
            Some(MouseControl::ToggleShuffle),
        ),
    ];
    if let Some(src) = &app.playing_track {
        let star = if app.is_favorite(src) { "★ " } else { "" };
        badges.push((
            badge(format!(" {}Plays: {} ", star, app.play_count(src)), yellow()),
            Some(MouseControl::ToggleFavorite),
        ));
    }
    if app.video.is_loaded() {
        let mode = if app.auto_av_offset { "auto" } else { "manual" };
        badges.push((
            badge(
                format!(" A/V: {:+.0} ms ({}) ", app.av_offset_secs * 1000.0, mode),
                cyan(),
            ),
            None,
        ));
        if app.subtitles.track_count() > 0 {
            let sub_state = match app.active_subtitle_track {
                Some(i) => app
                    .subtitles
                    .track_label(i)
                    .unwrap_or_else(|| format!("Track {}", i + 1)),
                None => "Off".to_string(),
            };
            badges.push((
                badge(format!(" Subs: {} ", sub_state), purple()),
                Some(MouseControl::CycleSubtitle),
            ));
        }
    }
    let (badge_lines, badge_hits) =
        layout_badges(badges, inner_layout[6].width as usize, inner_layout[6]);
    control_hits.extend(badge_hits);
    app.control_hits = control_hits;
    frame.render_widget(
        Paragraph::new(badge_lines).alignment(Alignment::Center),
        inner_layout[6],
    );

    draw_volume_column(frame, cols[1], app);
}

/// Render the centered cassette-deck transport row into `area` and return each
/// button's clickable rect. Buttons are fixed-width so they read as chunky,
/// evenly-sized keys; the middle one toggles between play (▶) and pause (⏸).
/// Rewind/fast-forward use ASCII arrows.
fn draw_transport(frame: &mut Frame, area: Rect, app: &App) -> Vec<(Rect, MouseControl)> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let playing = app.playing_index.is_some() && !app.audio.is_paused();
    let play_sym = if playing { "⏸" } else { "▶" };
    let buttons: [(&str, MouseControl, Color); 4] = [
        ("<<", MouseControl::SeekBack, cyan()),
        ("■", MouseControl::Stop, red()),
        (play_sym, MouseControl::PlayPause, green()),
        (">>", MouseControl::SeekForward, cyan()),
    ];
    // Fixed button width; centers each glyph in a wide, easy-to-click key.
    const BTN_W: u16 = 8;
    const GAP: u16 = 2;
    let style = |bg: Color| {
        Style::default()
            .fg(Color::Black)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };

    let total: u16 = BTN_W * buttons.len() as u16 + GAP * (buttons.len() as u16 - 1);
    let start_x = area.x + area.width.saturating_sub(total) / 2;

    let mut out: Vec<Span> = vec![Span::raw(" ".repeat((start_x - area.x) as usize))];
    let mut hits: Vec<(Rect, MouseControl)> = Vec::new();
    let mut x = start_x;
    for (i, (sym, action, bg)) in buttons.iter().enumerate() {
        // Center the glyph within BTN_W cells.
        let symw = Span::raw(*sym).width() as u16;
        let pad = BTN_W.saturating_sub(symw);
        let left = pad / 2;
        let label = format!(
            "{}{}{}",
            " ".repeat(left as usize),
            sym,
            " ".repeat((pad - left) as usize)
        );
        hits.push((Rect::new(x, area.y, BTN_W, 1), *action));
        out.push(Span::styled(label, style(*bg)));
        x += BTN_W;
        if i + 1 < buttons.len() {
            out.push(Span::raw(" ".repeat(GAP as usize)));
            x += GAP;
        }
    }
    frame.render_widget(Paragraph::new(Line::from(out)), area);
    hits
}

fn draw_volume_column(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.height < 3 || area.width == 0 {
        return;
    }
    let volume = app.audio.volume();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    // Record the bar area for click/drag volume control.
    app.volume_hit = Some(rows[2]);

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
    // Record the rect for the web `<img>` overlay, then let the backend paint
    // the art (native: ratatui-image; web: no-op, the overlay floats above).
    app.last_art_rect = Some(inner);
    app.art.render(frame, inner);
}

/// Fill the whole `area` with the track artwork for the fullscreen album-art
/// display modes. The image is drawn through the backend renderer (graphically
/// when the terminal supports it, ASCII halfblocks otherwise), centered and
/// aspect-fit within `area`. When the track has no artwork a centered notice is
/// shown instead.
pub(super) fn draw_fullscreen_art(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Paint the backdrop so any letterbox margins around the image are clean.
    frame.render_widget(
        Block::default().style(Style::default().bg(panel_bg())),
        area,
    );

    // Reserve a single unobtrusive row at the very bottom for the time bar when
    // something is playing and there is height to spare; the art fills the rest.
    let show_bar = app.playing_index.is_some() && area.height >= 4;
    let (art_area, bar_area) = if show_bar {
        let h = area.height - 1;
        (
            Rect::new(area.x, area.y, area.width, h),
            Some(Rect::new(area.x, area.y + h, area.width, 1)),
        )
    } else {
        (area, None)
    };

    if !app.art.has_art() {
        let notice = Paragraph::new("No album art")
            .style(Style::default().fg(dim()))
            .alignment(Alignment::Center);
        let y = art_area.y + art_area.height / 2;
        let line = Rect::new(art_area.x, y, art_area.width, 1);
        frame.render_widget(notice, line);
        if let Some(bar) = bar_area {
            draw_art_time_bar(frame, bar, app);
        }
        return;
    }

    // Record the rect for the web `<img>` overlay, then let the backend paint
    // the art (native: ratatui-image; web: no-op, the overlay floats above).
    app.last_art_rect = Some(art_area);
    app.art.render(frame, art_area);
    if let Some(bar) = bar_area {
        draw_art_time_bar(frame, bar, app);
    }
}

/// A compact single-row progress bar with the elapsed / total time on either
/// side, drawn along the bottom of the fullscreen album art. Deliberately thin
/// and dim so it stays out of the way of the artwork.
fn draw_art_time_bar(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Small horizontal inset so the bar doesn't run flush into the edges.
    let inset = if area.width >= 8 { 2 } else { 0 };
    let row = Rect::new(
        area.x + inset,
        area.y,
        area.width.saturating_sub(inset * 2),
        1,
    );
    // Clickable/scrubbable seek bar in fullscreen album art too.
    app.progress_hit = Some(row);
    let pos = app.position();
    let dur = app.current_duration.unwrap_or(Duration::ZERO);
    let ratio = if dur.as_secs_f64() > 0.0 {
        (pos.as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let left = fmt_time(pos);
    let right = if dur.is_zero() {
        String::from("--:--")
    } else {
        fmt_time(dur)
    };

    let w = row.width as usize;
    let labels = left.chars().count() + right.chars().count() + 2; // a space each side
    if w <= labels {
        // Too narrow for a groove — just show "elapsed / total".
        let line = Line::from(Span::styled(
            format!("{left} / {right}"),
            Style::default().fg(dim()),
        ));
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), row);
        return;
    }
    let bar_w = w - labels;
    let fill = (ratio * bar_w as f64).round() as usize;
    let filled: String = "━".repeat(fill);
    let rest: String = "─".repeat(bar_w - fill);

    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(text())),
        Span::raw(" "),
        Span::styled(filled, Style::default().fg(cyan())),
        Span::styled(rest, Style::default().fg(dim())),
        Span::raw(" "),
        Span::styled(right, Style::default().fg(text())),
    ]);
    frame.render_widget(Paragraph::new(line), row);
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
    let entries: [(&str, &str); 17] = [
        ("?", "Help"),
        ("←→", "Seek 10s"),
        ("Tab", "Focus"),
        ("Space", "Play/Pause"),
        ("+/-", "Volume"),
        ("g", "Search"),
        ("F", "Favorite"),
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

/// Custom playback bar: a solid groove with a bright fill whose hue gently
/// flows between the theme's primary and accent (a "color wave"), plus a
/// comet-tail glow trailing the playhead. Sub-cell precision at the fill edge
/// via left-eighth blocks. The bottom row carries elapsed / percent / total.
/// `now` is the platform wall-clock (seconds) driving the shimmer animation —
/// supplied by the caller because `std::time::Instant` is unavailable on wasm.
fn draw_progress_bar(frame: &mut Frame, area: Rect, ratio: f64, pos: &str, total: &str, now: f32) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    // Reserve the last row for the time read-out when we have the height for it.
    let bar_rows = if area.height >= 2 { area.height - 1 } else { area.height };

    let t = now;
    let track = Color::Rgb(38, 26, 56); // dim groove the fill rides in

    // Sub-cell fill: `full` columns are solid, the next column is a partial
    // left-block sized to `frac`.
    let filled_f = (ratio.clamp(0.0, 1.0) as f32) * w as f32;
    let full = filled_f.floor() as usize;
    let frac = filled_f - full as f32;
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

    let buf = frame.buffer_mut();
    for x in 0..w {
        // Color wave: mostly the primary hue, with a band of accent flowing
        // left-to-right over time. Kept narrow (45%) so it stays subtle.
        let wave = 0.5 + 0.5 * ((x as f32) * 0.22 - t * 2.4).sin();
        let base = lerp(pink(), cyan(), wave * 0.45);
        // Comet glow: cells just behind the playhead brighten toward highlight.
        let behind = filled_f - (x as f32 + 0.5);
        let glow = if behind >= 0.0 {
            (1.0 - behind / 4.0).clamp(0.0, 1.0) * 0.7
        } else {
            0.0
        };
        let fill = lerp(base, yellow(), glow);

        let (ch, fg, bg) = if x < full {
            ('█', fill, panel_bg())
        } else if x == full && frac > 0.0 {
            let idx = (frac * 8.0).round().clamp(1.0, 7.0) as usize;
            (EIGHTHS[idx], fill, track)
        } else {
            ('█', track, panel_bg())
        };

        for row in 0..bar_rows {
            if let Some(cell) = buf.cell_mut((area.x + x as u16, area.y + row)) {
                cell.set_char(ch);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
    }

    if area.height < 2 {
        return;
    }
    // Time read-out beneath the bar: elapsed left, percent centered, total right.
    let pct = format!("{}%", (ratio.clamp(0.0, 1.0) * 100.0).round() as u32);
    let line = Line::from(vec![
        Span::styled(pos.to_string(), Style::default().fg(text())),
        Span::styled(pct, Style::default().fg(dim())),
        Span::styled(total.to_string(), Style::default().fg(text())),
    ]);
    let time_area = Rect::new(area.x, area.y + bar_rows, area.width, 1);
    render_three_up(frame, time_area, &line);
}

/// Lay out exactly three spans across `area`: first flush left, second
/// centered, third flush right. Falls back to plain left alignment if the
/// area is too narrow to separate them.
fn render_three_up(frame: &mut Frame, area: Rect, line: &Line) {
    let spans = &line.spans;
    let w = area.width as usize;
    let len: Vec<usize> = spans.iter().map(|s| s.content.chars().count()).collect();
    let total_len: usize = len.iter().sum();
    if spans.len() != 3 || total_len >= w {
        frame.render_widget(Paragraph::new(line.clone()), area);
        return;
    }
    let mut out: Vec<Span> = Vec::with_capacity(5);
    out.push(spans[0].clone());
    // Spaces before the centered span so its midpoint lands at the bar center.
    let center_start = (w.saturating_sub(len[1])) / 2;
    let left_pad = center_start.saturating_sub(len[0]);
    out.push(Span::raw(" ".repeat(left_pad)));
    out.push(spans[1].clone());
    let used = len[0] + left_pad + len[1];
    let right_pad = w.saturating_sub(used + len[2]);
    out.push(Span::raw(" ".repeat(right_pad)));
    out.push(spans[2].clone());
    frame.render_widget(Paragraph::new(Line::from(out)), area);
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
