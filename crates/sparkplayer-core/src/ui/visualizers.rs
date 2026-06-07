//! Audio visualizers: the dispatcher plus the spectrum, waveform, scrolling
//! waveform, spectrogram, Lissajous, and pseudo-3D spectrum renderers, with
//! their associated color ramps. The cassette/VHS visualizer lives in
//! [`super::cassette`].

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders,
    canvas::{Canvas, Line as CanvasLine},
};

use crate::app::App;
use crate::visualizer::VisMode;

use super::cassette::{draw_cassette, draw_vhs};
use super::palette::{cyan, green, lerp, panel_bg, pink, purple, red, rgb, yellow};

pub(super) fn draw_visualizer(frame: &mut Frame, area: Rect, app: &mut App) {
    let mode = app.visualizer.mode;
    let active = app.playing_index.is_some() && !app.audio.is_paused();
    let title = format!(" Visualizer — {} ", mode.label());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(purple()))
        .title(Line::from(Span::styled(
            title,
            Style::default()
                .fg(cyan())
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(panel_bg()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 4 || inner.height < 3 {
        return;
    }

    match mode {
        VisMode::Spectrum => draw_spectrum(frame, inner, app, active),
        VisMode::MirrorBars => draw_mirror_bars(frame, inner, app, active),
        VisMode::Radial => draw_radial(frame, inner, app, active),
        VisMode::Waveform => draw_waveform(frame, inner, app, active),
        VisMode::ScrollingWaveform => draw_scrolling_waveform(frame, inner, app, active),
        VisMode::Spectrogram => draw_spectrogram(frame, inner, app, active),
        VisMode::Lissajous => draw_lissajous(frame, inner, app, active),
        VisMode::Vu => draw_vu(frame, inner, app, active),
        VisMode::Spectrum3D => draw_spectrum_3d(frame, inner, app, active),
        VisMode::Plasma => draw_plasma(frame, inner, app, active),
        VisMode::Cassette => {
            if app.video.is_loaded() {
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
    let sr = app.audio.tap().sample_rate();
    let mags = app.visualizer.spectrum(app.audio.tap(), bars, sr, active);
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
                    // Solid part of the bar: fill via background so the web
                    // canvas has no inter-row gap.
                    cell.set_char(' ');
                    cell.set_bg(color);
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
                    cell.set_bg(panel_bg());
                }
            }
        }
    }
}

fn bar_color(row: usize, h: usize) -> Color {
    if h == 0 {
        return green();
    }
    let t = row as f32 / h as f32;
    if t < 0.4 {
        lerp(green(), yellow(), t / 0.4)
    } else if t < 0.75 {
        lerp(yellow(), pink(), (t - 0.4) / 0.35)
    } else {
        lerp(pink(), purple(), (t - 0.75) / 0.25)
    }
}

/// Mirrored spectrum: bars rise above a centerline (with falling peak-hold
/// caps) and a dimmed reflection drops below it, like bars on a glossy floor.
fn draw_mirror_bars(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let bar_width: u16 = 2;
    let bars = (area.width / bar_width).max(1) as usize;
    let sr = app.audio.tap().sample_rate();
    let (mags, peaks) = app.visualizer.mirror_bars(app.audio.tap(), bars, sr, active);
    let h = area.height as usize;
    let up_rows = (h / 2).max(1); // rows above the centerline
    let down_rows = h - up_rows; // reflection rows below it
    let mid_y = area.y + up_rows as u16; // first reflection row
    let buf = frame.buffer_mut();
    for (i, &m) in mags.iter().enumerate() {
        let x0 = area.x + (i as u16) * bar_width;
        if x0 >= area.x + area.width {
            break;
        }
        let cols = bar_width.min(area.x + area.width - x0);

        // Upward bar with eighth-block tip (r = 0 sits just above the center).
        let up_f = (m * up_rows as f32 * 8.0).max(0.0);
        let up_full = (up_f / 8.0) as usize;
        let up_frac = (up_f as usize) % 8;
        for r in 0..up_rows {
            let y = mid_y - 1 - r as u16;
            let color = bar_color(r, up_rows);
            if r < up_full {
                fill_cells(buf, x0, cols, y, ' ', color, color);
            } else if r == up_full && up_frac > 0 {
                fill_cells(buf, x0, cols, y, eighth_up(up_frac), color, panel_bg());
            }
        }

        // Falling peak-hold cap floating above the bar.
        let cap = (peaks[i] * up_rows as f32).round() as usize;
        if cap >= 1 {
            let r = (cap - 1).min(up_rows - 1);
            let y = mid_y - 1 - r as u16;
            fill_cells(buf, x0, cols, y, ' ', yellow(), yellow());
        }

        // Dimmed reflection below the centerline (full cells only).
        let dn = (m * down_rows as f32).round() as usize;
        for r in 0..dn.min(down_rows) {
            let y = mid_y + r as u16;
            let color = lerp(bar_color(r, down_rows.max(1)), panel_bg(), 0.55);
            fill_cells(buf, x0, cols, y, ' ', color, color);
        }
    }
}

/// Paint `cols` horizontally-adjacent cells at row `y` with the same glyph/colors.
fn fill_cells(
    buf: &mut ratatui::buffer::Buffer,
    x0: u16,
    cols: u16,
    y: u16,
    ch: char,
    fg: Color,
    bg: Color,
) {
    for c in 0..cols {
        if let Some(cell) = buf.cell_mut((x0 + c, y)) {
            cell.set_char(ch);
            cell.set_fg(fg);
            cell.set_bg(bg);
        }
    }
}

/// Lower eighth-block for a partial bar tip filled from the bottom (1..=7).
fn eighth_up(frac: usize) -> char {
    match frac {
        1 => '▁',
        2 => '▂',
        3 => '▃',
        4 => '▄',
        5 => '▅',
        6 => '▆',
        7 => '▇',
        _ => '█',
    }
}

/// Radial spectrum: the FFT bins wrapped symmetrically around a circle, each a
/// spoke whose length tracks its band, with an inner ring that breathes with
/// the overall energy.
fn draw_radial(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    let nbars = (h * 3).clamp(16, 64);
    let sr = app.audio.tap().sample_rate();
    let mags = app.visualizer.spectrum(app.audio.tap(), nbars, sr, active);
    let energy: f32 = if mags.is_empty() {
        0.0
    } else {
        mags.iter().sum::<f32>() / mags.len() as f32
    };

    // Keep the circle round: cells are ~2:1, so widen the X bounds like the
    // Lissajous view does.
    let cell_aspect = 2.0f64;
    let h_unit = 1.1f64;
    let w_unit = h_unit * (w as f64 / h as f64) / cell_aspect;
    let r0 = 0.34 * (1.0 + 0.18 * energy as f64); // inner radius, breathing
    let span = 0.62;

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(panel_bg())
        .x_bounds([-w_unit, w_unit])
        .y_bounds([-h_unit, h_unit])
        .paint(move |ctx| {
            // Inner ring.
            let ring = lerp(purple(), cyan(), energy);
            let seg = 64;
            for k in 0..seg {
                let a1 = (k as f64 / seg as f64) * std::f64::consts::TAU;
                let a2 = ((k + 1) as f64 / seg as f64) * std::f64::consts::TAU;
                ctx.draw(&CanvasLine {
                    x1: a1.cos() * r0,
                    y1: a1.sin() * r0,
                    x2: a2.cos() * r0,
                    y2: a2.sin() * r0,
                    color: ring,
                });
            }
            // Spokes, mirrored left/right so the figure is symmetric.
            let n = nbars * 2;
            for a in 0..n {
                let idx = if a < nbars { a } else { n - 1 - a };
                let m = mags[idx];
                let angle = (a as f64 / n as f64) * std::f64::consts::TAU;
                let r1 = r0;
                let r2 = r0 + m as f64 * span;
                let color = lerp(cyan(), pink(), m);
                ctx.draw(&CanvasLine {
                    x1: angle.cos() * r1,
                    y1: angle.sin() * r1,
                    x2: angle.cos() * r2,
                    y2: angle.sin() * r2,
                    color,
                });
            }
        });
    frame.render_widget(canvas, area);
}

/// Stereo VU meters: L/R RMS level bars with green→yellow→red zones, a falling
/// peak-hold tick, and a centered −1…+1 phase-correlation bar below.
fn draw_vu(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let lv = app.visualizer.levels(app.audio.tap(), active);
    let w = area.width as usize;
    let h = area.height;
    if w == 0 || h == 0 {
        return;
    }
    // Split the height: two level bars on top, the correlation bar on the last
    // row when there's room.
    let corr_rows: u16 = if h >= 4 { 1 } else { 0 };
    let meter_area = h - corr_rows;
    let bar_h = (meter_area / 2).max(1);

    draw_meter(frame, area.x, area.y, bar_h, area.width, lv.rms[0], lv.peak[0]);
    draw_meter(
        frame,
        area.x,
        area.y + bar_h,
        bar_h,
        area.width,
        lv.rms[1],
        lv.peak[1],
    );

    if corr_rows == 1 {
        draw_correlation(frame, area.x, area.y + area.height - 1, area.width, lv.correlation);
    }
}

fn vu_zone_color(t: f32) -> Color {
    if t < 0.6 {
        lerp(green(), yellow(), t / 0.6)
    } else if t < 0.85 {
        lerp(yellow(), pink(), (t - 0.6) / 0.25)
    } else {
        lerp(pink(), red(), (t - 0.85) / 0.15)
    }
}

fn draw_meter(frame: &mut Frame, x0: u16, y0: u16, rows: u16, width: u16, value: f32, peak: f32) {
    let w = width as usize;
    if w == 0 || rows == 0 {
        return;
    }
    let fill = value.clamp(0.0, 1.0) * w as f32;
    let full = fill.floor() as usize;
    let frac = (fill - full as f32) * 8.0;
    let peak_col = ((peak.clamp(0.0, 1.0) * w as f32) as usize).min(w.saturating_sub(1));
    let buf = frame.buffer_mut();
    for col in 0..w {
        let t = col as f32 / w as f32;
        let zone = vu_zone_color(t);
        let (ch, fg, bg) = if col < full {
            (' ', zone, zone)
        } else if col == full && frac >= 1.0 {
            (eighth_left(frac as usize), zone, panel_bg())
        } else if col == peak_col && peak > 0.0 {
            // Floating peak-hold tick beyond the filled region.
            ('│', yellow(), panel_bg())
        } else {
            ('·', Color::Rgb(50, 45, 70), panel_bg())
        };
        let x = x0 + col as u16;
        for r in 0..rows {
            if let Some(cell) = buf.cell_mut((x, y0 + r)) {
                cell.set_char(ch);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
    }
}

/// Left eighth-block for a partial meter tip filled from the left (1..=7).
fn eighth_left(frac: usize) -> char {
    match frac {
        1 => '▏',
        2 => '▎',
        3 => '▍',
        4 => '▌',
        5 => '▋',
        6 => '▊',
        7 => '▉',
        _ => '█',
    }
}

fn draw_correlation(frame: &mut Frame, x0: u16, y: u16, width: u16, corr: f32) {
    let w = width as usize;
    if w == 0 {
        return;
    }
    let mid = w / 2;
    let marker = (((corr.clamp(-1.0, 1.0) * 0.5 + 0.5) * (w - 1) as f32).round() as usize).min(w - 1);
    // Positive correlation (mono-compatible) reads cyan; negative (out of
    // phase) reads red.
    let marker_color = if corr >= 0.0 {
        lerp(dim_axis(), cyan(), corr)
    } else {
        lerp(dim_axis(), red(), -corr)
    };
    let buf = frame.buffer_mut();
    for col in 0..w {
        let (ch, fg) = if col == marker {
            ('●', marker_color)
        } else if col == mid {
            ('┊', Color::Rgb(70, 65, 95))
        } else {
            ('─', Color::Rgb(45, 40, 65))
        };
        if let Some(cell) = buf.cell_mut((x0 + col as u16, y)) {
            cell.set_char(ch);
            cell.set_fg(fg);
            cell.set_bg(panel_bg());
        }
    }
}

fn dim_axis() -> Color {
    Color::Rgb(70, 65, 95)
}

/// Audio-reactive plasma field: layered sines drifting over time, brightened by
/// bass and shimmered by treble, mapped through the spectrogram heat ramp.
fn draw_plasma(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    let sr = app.audio.tap().sample_rate();
    let (phase, bands) = app.visualizer.plasma_state(app.audio.tap(), sr, active);
    let (bass, _mid, treble) = (bands[0], bands[1], bands[2]);
    let p = phase;
    let buf = frame.buffer_mut();
    for yy in 0..h {
        let fy = yy as f32;
        for xx in 0..w {
            let fx = xx as f32;
            let v = (fx * 0.20 + p).sin()
                + (fy * 0.26 + p * 0.8).sin()
                + ((fx + fy) * 0.15 + p * 1.3).sin()
                + ((fx * fx + fy * fy).sqrt() * 0.16 - p * 1.1).sin();
            // Map [-4, 4] → [0, 1].
            let base = v * 0.125 + 0.5;
            // Bass lifts overall intensity; treble adds a moving shimmer.
            let shimmer = treble * 0.2 * ((fx * 0.5 + p * 3.0).sin() * 0.5 + 0.5);
            let t = (base * (0.45 + 0.85 * bass) + shimmer).clamp(0.0, 1.0);
            let color = heatmap(t);
            if let Some(cell) = buf.cell_mut((area.x + xx as u16, area.y + yy as u16)) {
                cell.set_char(' ');
                cell.set_bg(color);
            }
        }
    }
}

fn draw_waveform(frame: &mut Frame, area: Rect, app: &mut App, active: bool) {
    let w = area.width as usize;
    let h = area.height as usize;
    let points = app.visualizer.waveform(app.audio.tap(), w, active);
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
            cell.set_bg(panel_bg());
        }
    }
    for (i, p) in points.iter().enumerate() {
        let amp = (*p * mid as f32) as usize;
        let amp = amp.min(mid);
        for d in 0..=amp {
            let color = lerp(cyan(), pink(), d as f32 / mid.max(1) as f32);
            let yu = mid.saturating_sub(d);
            let yd = (mid + d).min(h - 1);
            // Inner cells fill via background (no web-canvas row gap); the tip
            // cell keeps a half-block glyph for the amplitude edge.
            if let Some(cell) = buf.cell_mut((area.x + i as u16, area.y + yu as u16)) {
                if d == amp {
                    cell.set_char('▀');
                    cell.set_fg(color);
                    cell.set_bg(panel_bg());
                } else {
                    cell.set_char(' ');
                    cell.set_bg(color);
                }
            }
            if let Some(cell) = buf.cell_mut((area.x + i as u16, area.y + yd as u16)) {
                if d == amp {
                    cell.set_char('▄');
                    cell.set_fg(color);
                    cell.set_bg(panel_bg());
                } else {
                    cell.set_char(' ');
                    cell.set_bg(color);
                }
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
    let points = app.visualizer.scrolling_waveform(app.audio.tap(), dots, active);
    // Slight headroom so peaks at 1.0 don't clip into the top border.
    let y_max = 1.05f64;
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(panel_bg())
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
                let color = lerp(cyan(), pink(), *p);
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
    let sr = app.audio.tap().sample_rate();
    let cols = app.visualizer.spectrogram(app.audio.tap(), w, h, sr, active);
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
                // Paint the cell via its background so it fills the full cell on
                // the web canvas backend (a `█` glyph leaves a row gap there).
                cell.set_char(' ');
                cell.set_bg(color);
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
    let pts = app.visualizer.lissajous(app.audio.tap(), 2048, active);
    // Keep the plot square in pixel terms: cells are ~2:1, so the X bounds
    // are twice the Y bounds and we letterbox via the Canvas bounds.
    let cell_aspect = 2.0f64;
    let h_unit = 1.05f64;
    let w_unit = h_unit * (w as f64 / h as f64) / cell_aspect;
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(panel_bg())
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
                let color = lerp(cyan(), pink(), r);
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
    let sr = app.audio.tap().sample_rate();
    let rows = app.visualizer.spectrum_3d(app.audio.tap(), bins, sr, depth_rows, active);
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
        .background_color(panel_bg())
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
