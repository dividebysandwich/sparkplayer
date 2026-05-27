//! Modal overlays drawn on top of the main layout: the help popup and the
//! escape (settings) menu, with their small text-layout helpers.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

use crate::app::{App, EscapeMenuKind};

use super::palette::{cyan, dim, pink, purple, text, yellow};

pub(super) fn draw_help(frame: &mut Frame, area: Rect) {
    let w = area.width.min(66);
    let h = area.height.min(30);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect::new(x, y, w, h);
    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pink()).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            " ✦ SparkPlayer Help ",
            Style::default()
                .fg(yellow())
                .add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(Color::Rgb(15, 10, 30)));

    let body = vec![
        Line::from(Span::styled(
            "Playback",
            Style::default().fg(cyan()).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Space          play / pause"),
        Line::from("  n / p          next / previous track"),
        Line::from("  ← / →          seek -10s / +10s"),
        Line::from("  Ctrl+← / Ctrl+→  seek -30s / +30s"),
        Line::from("  + / = / -      volume up / up / down"),
        Line::from("  [ / ]          A/V sync offset -25ms / +25ms (video)"),
        Line::from("  c              cycle subtitle track (video)"),
        Line::from("  Enter          play selection"),
        Line::from(""),
        Line::from(Span::styled(
            "Navigation",
            Style::default().fg(cyan()).add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ / ↓          move selection"),
        Line::from("  PgUp / PgDn    page selection"),
        Line::from("  Home / End     jump to first / last"),
        Line::from("  Tab            switch focus (playlist ↔ browser)"),
        Line::from(""),
        Line::from(Span::styled(
            "Modes",
            Style::default().fg(cyan()).add_modifier(Modifier::BOLD),
        )),
        Line::from("  v              cycle visualizer:"),
        Line::from("                  FFT bars → waveform → scrolling →"),
        Line::from("                  spectrogram → stereo X/Y →"),
        Line::from("                  spectrum 3D → cassette tape"),
        Line::from("  t              cycle color theme"),
        Line::from("  f              cycle display: normal → fullscreen → video window"),
        Line::from("  r              cycle repeat (off / all / one)"),
        Line::from("  s              shuffle remaining tracks"),
        Line::from(""),
        Line::from(Span::styled(
            "Playlist",
            Style::default().fg(cyan()).add_modifier(Modifier::BOLD),
        )),
        Line::from("  a              queue the highlighted browser item"),
        Line::from("  Shift+A        queue every audio file under the current dir"),
        Line::from("  Shift+C        clear the playlist (stops playback)"),
        Line::from(""),
        Line::from("  Esc            open menu (volume, subtitle, A/V, …)"),
        Line::from("  ? or h         this help    •    q    quit"),
    ];

    frame.render_widget(Paragraph::new(body).block(block), rect);
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
            EscapeMenuKind::Subtitle
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
