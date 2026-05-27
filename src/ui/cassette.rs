//! The cassette-tape and VHS visualizers, plus the small text-into-buffer
//! label helpers they rely on. The case, label and screws are drawn directly
//! into the cell buffer for crisp box-art, while each spindle/reel is a
//! Canvas+Braille widget so the spokes step in sub-cell increments.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};

use crate::app::App;

use super::palette::{cyan, pink, yellow};

/// VHS cassette variant of the cassette visualizer, used when the current
/// track is a video. VHS tapes are wider and thinner than compact cassettes,
/// with two big translucent reels behind a clear window and a tape door slot
/// along the bottom edge. Same rotating-spindle technique as the audio
/// cassette.
pub(super) fn draw_vhs(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
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
pub(super) fn draw_cassette(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
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
    let case_trim = cyan();
    let label_bg = Color::Rgb(245, 230, 195);
    let label_text = Color::Rgb(35, 25, 60);
    let label_meta = Color::Rgb(170, 90, 70);
    let label_border = pink();
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
                circle(ctx, hub_r, yellow());
                let spokes = 6;
                for s in 0..spokes {
                    let a = phase_d + (s as f64 / spokes as f64) * std::f64::consts::TAU;
                    ctx.draw(&CanvasLine {
                        x1: -a.cos() * hub_r * 0.2,
                        y1: -a.sin() * hub_r * 0.2,
                        x2: a.cos() * 0.72,
                        y2: a.sin() * 0.72,
                        color: cyan(),
                    });
                }
                // Center peg.
                circle(ctx, 0.10, Color::Rgb(40, 30, 60));
                circle(ctx, 0.05, pink());
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
