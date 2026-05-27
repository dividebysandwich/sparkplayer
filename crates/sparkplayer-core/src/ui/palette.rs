//! Theme palette accessors and color math shared across the UI submodules.
//!
//! Every accessor reads the active theme from a thread-local, so swapping
//! themes at runtime instantly recolors every redraw.

use ratatui::style::Color;

use crate::theme;

pub(super) fn pink() -> Color {
    theme::current().primary
}
pub(super) fn cyan() -> Color {
    theme::current().accent
}
pub(super) fn purple() -> Color {
    theme::current().secondary
}
pub(super) fn yellow() -> Color {
    theme::current().highlight
}
pub(super) fn green() -> Color {
    theme::current().ok
}
pub(super) fn red() -> Color {
    theme::current().warn
}
pub(super) fn dim() -> Color {
    theme::current().dim
}
pub(super) fn text() -> Color {
    theme::current().text
}
pub(super) fn panel_bg() -> Color {
    theme::current().bg
}

pub(super) fn lerp(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let t = t.clamp(0.0, 1.0);
    let r = (ar as f32 + (br as f32 - ar as f32) * t) as u8;
    let g = (ag as f32 + (bg as f32 - ag as f32) * t) as u8;
    let b = (ab as f32 + (bb as f32 - ab as f32) * t) as u8;
    Color::Rgb(r, g, b)
}

pub(super) fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}
