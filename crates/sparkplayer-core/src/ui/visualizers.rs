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
use super::palette::{cyan, green, lerp, panel_bg, pink, purple, rgb, yellow};

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
        VisMode::Waveform => draw_waveform(frame, inner, app, active),
        VisMode::ScrollingWaveform => draw_scrolling_waveform(frame, inner, app, active),
        VisMode::Spectrogram => draw_spectrogram(frame, inner, app, active),
        VisMode::Lissajous => draw_lissajous(frame, inner, app, active),
        VisMode::Spectrum3D => draw_spectrum_3d(frame, inner, app, active),
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
