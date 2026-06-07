//! Modal overlays drawn on top of the main layout: the help popup and the
//! escape (settings) menu, with their small text-layout helpers.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use crate::app::{App, EscapeMenuKind};

use super::palette::{cyan, dim, pink, purple, text, yellow};

pub(super) fn draw_help(frame: &mut Frame, area: Rect, app: &mut App) {
    let w = area.width.min(70);
    let h = area.height.min(34);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);
    frame.render_widget(Clear, rect);

    let body = help_lines();

    // Inner content height: minus the top/bottom borders (2) and the top/bottom
    // padding (2). Clamp the scroll offset so the last page can't scroll past
    // the end (this is also where `End` / u16::MAX resolves to the real bottom).
    let visible = h.saturating_sub(4);
    let max_scroll = (body.len() as u16).saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let scroll = app.help_scroll;

    let more_above = scroll > 0;
    let more_below = scroll < max_scroll;
    let hint = match (more_above, more_below) {
        (false, false) => " ↑↓/PgUp/PgDn scroll • Esc close ".to_string(),
        _ => format!(
            " {}↑↓/PgUp/PgDn scroll{} • Esc close ",
            if more_above { "▲ " } else { "" },
            if more_below { " ▼" } else { "" },
        ),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pink()).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            " ✦ SparkPlayer Help ",
            Style::default()
                .fg(yellow())
                .add_modifier(Modifier::BOLD),
        )))
        .title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(dim()),
        )))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(Color::Rgb(15, 10, 30)));

    frame.render_widget(
        Paragraph::new(body).block(block).scroll((scroll, 0)),
        rect,
    );
}

fn section(title: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        title,
        Style::default().fg(cyan()).add_modifier(Modifier::BOLD),
    ))
}

/// The full help text. Kept well within the overlay's inner width (~62 cols)
/// so lines never wrap and the scroll offset maps one-to-one to source lines.
fn help_lines() -> Vec<Line<'static>> {
    vec![
        section("Welcome"),
        Line::from("  SparkPlayer is an easy, fun terminal music player for"),
        Line::from("  your on-disk library. It opens to your music folder and"),
        Line::from("  plays just about anything — no library import needed."),
        Line::from(""),
        section("Getting around"),
        Line::from("  The left side has your playlist (top) and a file"),
        Line::from("  browser (bottom). Press Tab to move focus between them."),
        Line::from("  In the browser, walk into folders with Enter and queue"),
        Line::from("  what you find. The right side shows now-playing info,"),
        Line::from("  album art or video, and the visualizer."),
        Line::from(""),
        section("Playback"),
        Line::from("  Space          play / pause"),
        Line::from("  n / p          next / previous track"),
        Line::from("  ← / →          seek -10s / +10s"),
        Line::from("  Ctrl+← / →     seek -30s / +30s"),
        Line::from("  + / = / -      volume up / up / down"),
        Line::from("  [ / ]          A/V sync offset -25ms / +25ms (video)"),
        Line::from("  b              cycle audio track (video)"),
        Line::from("  c              cycle subtitle track (video)"),
        Line::from(""),
        section("Navigation"),
        Line::from("  ↑ / ↓          move selection"),
        Line::from("  PgUp / PgDn    page selection"),
        Line::from("  Home / End     jump to first / last"),
        Line::from("  Tab            switch focus (playlist ↔ browser)"),
        Line::from("  /              filter the focused list (Esc clears)"),
        Line::from(""),
        section("Managing the playlist"),
        Line::from("  Enter          browser: open folder / load playlist /"),
        Line::from("                 play file   •   playlist: play selection"),
        Line::from("  a              queue the highlighted browser item"),
        Line::from("  Shift+A        queue every audio file under the dir"),
        Line::from("                 (recursive)"),
        Line::from("  d / Delete     remove the highlighted playlist track"),
        Line::from("  Ctrl+↑ / ↓     move the highlighted track up / down"),
        Line::from("  w              save the playlist to an .m3u file"),
        Line::from("  Shift+C        clear the playlist (stops playback)"),
        Line::from("  s              shuffle the remaining tracks"),
        Line::from("  r              cycle repeat (off / all / one)"),
        Line::from(""),
        section("Look & feel"),
        Line::from("  v              cycle visualizer:"),
        Line::from("                  FFT bars → mirror bars → radial →"),
        Line::from("                  waveform → scrolling → spectrogram →"),
        Line::from("                  stereo X/Y → VU meters → spectrum 3D →"),
        Line::from("                  plasma → cassette tape"),
        Line::from("  t              cycle color theme"),
        Line::from("  f              cycle display: normal → fullscreen →"),
        Line::from("                 video window"),
        Line::from(""),
        section("Other"),
        Line::from("  Esc            open menu (volume, subtitle, A/V, …)"),
        Line::from("  ? or h         this help    •    q    quit"),
        Line::from(""),
        section("Supported formats"),
        Line::from("  Audio     mp3  wav  ogg  flac  m4a  aac  opus  wma"),
        Line::from("  Video     mp4  mkv  avi  mov  webm  m4v"),
        Line::from("  Playlist  m3u  m3u8  pls"),
        Line::from("  Album art is read from embedded tags, or a cover /"),
        Line::from("  folder / front image sitting next to the track."),
        Line::from(""),
        section("Video"),
        Line::from("  Video plays right in the album-art pane, synced to the"),
        Line::from("  audio. Terminals with a graphics protocol (Kitty,"),
        Line::from("  Sixel, iTerm2) show true images; others use colored"),
        Line::from("  halfblocks. Press f to fill the window."),
    ]
}

pub(super) fn draw_escape_menu(frame: &mut Frame, area: Rect, app: &App) {
    let items = app.escape_menu_items();
    let logo_lines: &[&str] = &[
        "░█▀▀░█▀█░█▀█░█▀▄░█░█░█▀█░█░░░█▀█░█░█░█▀▀░█▀▄",
        "░▀▀█░█▀▀░█▀█░█▀▄░█▀▄░█▀▀░█░░░█▀█░░█░░█▀▀░█▀▄",
        "░▀▀▀░▀░░░▀░▀░▀░▀░▀░▀░▀░░░▀▀▀░▀░▀░░▀░░▀▀▀░▀░▀",
    ];
    let logo_w = logo_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0) as u16;
    // Width: borders (2) + horizontal padding (4) + content. The hint line is
    // ~47 chars, so the content column needs at least that. Allow extra
    // margin so adjustment values (e.g. long subtitle labels) don't crowd
    // the right edge.
    let body_w = logo_w.max(54) + 8;
    let body_h = (3 + 1 + items.len() as u16 + 1 + 1 + 2 + 2).min(area.height);
    let w = body_w.min(area.width);
    let h = body_h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);
    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pink()).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            " Menu ",
            Style::default()
                .fg(yellow())
                .add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(Color::Rgb(15, 10, 30)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(logo_lines.len() as u16),
            Constraint::Length(1),
            Constraint::Min(items.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // Logo — each row uses a different theme color for a colorful look.
    let logo_colors = [pink(), cyan(), purple()];
    let logo_para: Vec<Line> = logo_lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            Line::from(Span::styled(
                *l,
                Style::default()
                    .fg(logo_colors[i % logo_colors.len()])
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(logo_para).alignment(Alignment::Center),
        layout[0],
    );

    // Menu rows.
    let body_w = layout[2].width as usize;
    let label_col = 14usize.min(body_w / 2);
    let value_col = body_w.saturating_sub(label_col + 4);
    let mut rows: Vec<Line> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let selected = i == app.escape_menu_selected;
        if item.kind == EscapeMenuKind::Separator {
            let dash: String = "─".repeat(body_w.saturating_sub(2));
            rows.push(Line::from(Span::styled(
                dash,
                Style::default().fg(dim()),
            )));
            continue;
        }
        let prefix = if selected { " ➤ " } else { "   " };
        let label_style = if !item.enabled {
            Style::default().fg(dim())
        } else if selected {
            Style::default()
                .fg(yellow())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(cyan())
        };
        let value_style = if !item.enabled {
            Style::default().fg(dim())
        } else if selected {
            Style::default().fg(pink()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(text())
        };

        let label_padded = pad_right(item.label, label_col);
        let value = if item.value.is_empty() {
            String::new()
        } else if item.kind == EscapeMenuKind::Volume {
            // Render a small horizontal slider next to the percentage.
            let bar = volume_bar(app.audio.volume(), value_col.saturating_sub(7));
            format!("{} {}", bar, item.value)
        } else if matches!(
            item.kind,
            EscapeMenuKind::AudioTrack
                | EscapeMenuKind::Subtitle
                | EscapeMenuKind::Visualizer
                | EscapeMenuKind::Theme
        ) && item.enabled
        {
            format!("‹ {} ›", item.value)
        } else {
            item.value.clone()
        };

        rows.push(Line::from(vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(if selected { pink() } else { Color::Reset }),
            ),
            Span::styled(label_padded, label_style),
            Span::styled(value, value_style),
        ]));
    }
    frame.render_widget(Paragraph::new(rows), layout[2]);

    let hint = Line::from(vec![
        Span::styled("↑↓ ", Style::default().fg(cyan())),
        Span::styled("navigate  ", Style::default().fg(dim())),
        Span::styled("←→ ", Style::default().fg(cyan())),
        Span::styled("adjust  ", Style::default().fg(dim())),
        Span::styled("Enter ", Style::default().fg(cyan())),
        Span::styled("select  ", Style::default().fg(dim())),
        Span::styled("Esc ", Style::default().fg(cyan())),
        Span::styled("close", Style::default().fg(dim())),
    ]);
    frame.render_widget(
        Paragraph::new(hint).alignment(Alignment::Center),
        layout[4],
    );
}

fn pad_right(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count >= width {
        s.to_string()
    } else {
        let mut out = String::with_capacity(width);
        out.push_str(s);
        for _ in 0..(width - count) {
            out.push(' ');
        }
        out
    }
}

fn volume_bar(volume: f32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let frac = (volume.clamp(0.0, 1.5) / 1.5).min(1.0);
    let filled = (frac * width as f32).round() as usize;
    let mut s = String::with_capacity(width + 2);
    s.push('[');
    for i in 0..width {
        s.push(if i < filled { '█' } else { '░' });
    }
    s.push(']');
    s
}
