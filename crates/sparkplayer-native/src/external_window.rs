use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::BlendMode;

use crate::video::VideoFrame;

const WINDOW_TITLE: &str = "SparkPlayer Video";

/// A keyboard event forwarded from the SDL window back into the main loop so
/// the same shortcuts work whether the terminal or the playback window has
/// focus.
pub struct ForwardedKey {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// External GUI window for fullscreen video playback. Runs SDL on a dedicated
/// thread: the main loop pushes frames in, and the window's event pump pushes
/// keyboard events back out.
pub struct ExternalVideoWindow {
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    subtitle: Arc<Mutex<Option<String>>>,
    key_rx: Receiver<ForwardedKey>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    init_err: Arc<Mutex<Option<String>>>,
}

impl ExternalVideoWindow {
    pub fn spawn() -> Result<Self> {
        let latest_frame: Arc<Mutex<Option<VideoFrame>>> = Arc::new(Mutex::new(None));
        let subtitle: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (key_tx, key_rx): (Sender<ForwardedKey>, Receiver<ForwardedKey>) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let init_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();

        let latest_frame_t = Arc::clone(&latest_frame);
        let subtitle_t = Arc::clone(&subtitle);
        let stop_t = Arc::clone(&stop);
        let init_err_t = Arc::clone(&init_err);

        let handle = thread::Builder::new()
            .name("sparkplayer-video-window".into())
            .spawn(move || {
                if let Err(e) =
                    run_window(latest_frame_t, subtitle_t, key_tx, stop_t, ready_tx)
                {
                    *init_err_t.lock().unwrap() = Some(e.to_string());
                }
            })
            .context("spawning external video window thread")?;

        // Wait briefly for the window thread to confirm initialization so we
        // can surface a clean error to the user instead of silently failing.
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Self {
                latest_frame,
                subtitle,
                key_rx,
                stop,
                handle: Some(handle),
                init_err,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("video window init failed: {e}")),
            Err(_) => Err(anyhow::anyhow!("video window init timed out")),
        }
    }

    /// Push the active subtitle line (or `None` to clear). The window thread
    /// re-rasterizes the overlay texture only when the string actually changes.
    pub fn set_subtitle(&self, text: Option<String>) {
        if let Ok(mut slot) = self.subtitle.lock() {
            *slot = text;
        }
    }

    /// Stash the most recent frame for the window thread to pick up. Older
    /// pending frames are dropped — the window always renders the freshest.
    pub fn submit_frame(&self, frame: VideoFrame) {
        if let Ok(mut slot) = self.latest_frame.lock() {
            *slot = Some(frame);
        }
    }

    /// Drain any keyboard events the SDL event pump has accumulated since the
    /// last poll.
    pub fn drain_keys(&self) -> Vec<ForwardedKey> {
        let mut out = Vec::new();
        while let Ok(k) = self.key_rx.try_recv() {
            out.push(k);
        }
        out
    }

    /// Returns true when the window thread has died (e.g. user clicked the
    /// close button). The main loop uses this to drop the wrapper and return
    /// rendering to the terminal.
    pub fn is_alive(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
            && self.handle.as_ref().is_some_and(|h| !h.is_finished())
            && self.init_err.lock().map(|e| e.is_none()).unwrap_or(false)
    }
}

impl Drop for ExternalVideoWindow {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run_window(
    latest_frame: Arc<Mutex<Option<VideoFrame>>>,
    subtitle: Arc<Mutex<Option<String>>>,
    key_tx: Sender<ForwardedKey>,
    stop: Arc<AtomicBool>,
    ready_tx: Sender<Result<(), String>>,
) -> Result<()> {
    let sdl = match sdl2::init() {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return Err(anyhow::anyhow!("sdl init: {e}"));
        }
    };
    let video_sub = match sdl.video() {
        Ok(v) => v,
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return Err(anyhow::anyhow!("sdl video subsystem: {e}"));
        }
    };

    let window = match video_sub
        .window(WINDOW_TITLE, 1280, 720)
        .position_centered()
        .resizable()
        .fullscreen_desktop()
        .build()
    {
        Ok(w) => w,
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return Err(anyhow::anyhow!("create window: {e}"));
        }
    };

    let mut canvas = match window.into_canvas().accelerated().present_vsync().build() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return Err(anyhow::anyhow!("build canvas: {e}"));
        }
    };
    canvas.set_draw_color(Color::BLACK);
    canvas.clear();
    canvas.present();

    let texture_creator = canvas.texture_creator();
    let mut texture: Option<sdl2::render::Texture> = None;
    let mut tex_w: u32 = 0;
    let mut tex_h: u32 = 0;
    let font: Option<FontVec> = load_system_font();
    let mut sub_cache: Option<SubtitleTexture> = None;

    let mut event_pump = match sdl.event_pump() {
        Ok(e) => e,
        Err(e) => {
            let _ = ready_tx.send(Err(e.to_string()));
            return Err(anyhow::anyhow!("event pump: {e}"));
        }
    };

    let _ = ready_tx.send(Ok(()));

    'main: loop {
        if stop.load(Ordering::Relaxed) {
            break 'main;
        }

        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::Window {
                    win_event: WindowEvent::Close,
                    ..
                } => {
                    // Signal closure to the main loop via a Ctrl+C-equivalent
                    // and stop the thread.
                    let _ = key_tx.send(ForwardedKey {
                        code: KeyCode::Esc,
                        mods: KeyModifiers::empty(),
                    });
                    stop.store(true, Ordering::Relaxed);
                    break 'main;
                }
                Event::KeyDown {
                    keycode: Some(kc),
                    keymod,
                    repeat: false,
                    ..
                } => {
                    if let Some(fk) = translate_key(kc, keymod) {
                        let _ = key_tx.send(fk);
                    }
                }
                _ => {}
            }
        }

        let frame_opt = latest_frame.lock().ok().and_then(|mut s| s.take());
        let sub_text = subtitle.lock().ok().and_then(|s| s.clone());
        if let Some(frame) = frame_opt {
            if texture.is_none() || frame.width != tex_w || frame.height != tex_h {
                texture = texture_creator
                    .create_texture_streaming(
                        PixelFormatEnum::RGB24,
                        frame.width,
                        frame.height,
                    )
                    .ok();
                tex_w = frame.width;
                tex_h = frame.height;
            }
            if let Some(tex) = texture.as_mut() {
                let pitch = (frame.width * 3) as usize;
                let _ = tex.update(None, &frame.data, pitch);
            }
            canvas.set_draw_color(Color::BLACK);
            canvas.clear();
            let (out_w, out_h) = canvas.output_size().unwrap_or((tex_w, tex_h));
            let video_rect = aspect_fit(tex_w, tex_h, out_w, out_h);
            if let Some(tex) = texture.as_ref() {
                let _ = canvas.copy(tex, None, video_rect);
            }
            // Subtitles: redraw the cached overlay if the line changed; blit
            // centered along the video rect's bottom.
            refresh_subtitle_cache(
                &mut sub_cache,
                font.as_ref(),
                sub_text.as_deref(),
                &texture_creator,
                out_w,
            );
            if let Some(cache) = sub_cache.as_ref() {
                let sx = video_rect.x()
                    + ((video_rect.width() as i32) - (cache.width as i32)) / 2;
                let sy = video_rect.y() + (video_rect.height() as i32)
                    - (cache.height as i32)
                    - (video_rect.height() as i32 / 20).max(16);
                let dst = Rect::new(sx, sy, cache.width, cache.height);
                let _ = canvas.copy(&cache.texture, None, dst);
            }
            canvas.present();
        } else {
            // No new frame — sleep briefly so we don't spin the CPU. Vsync on
            // present() handles pacing when frames are arriving.
            thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(())
}

/// Compute a destination rect that fits `(src_w, src_h)` inside `(dst_w, dst_h)`
/// preserving aspect ratio.
fn aspect_fit(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Rect {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Rect::new(0, 0, dst_w.max(1), dst_h.max(1));
    }
    let src_aspect = src_w as f32 / src_h as f32;
    let dst_aspect = dst_w as f32 / dst_h as f32;
    let (w, h) = if src_aspect > dst_aspect {
        let w = dst_w;
        let h = (dst_w as f32 / src_aspect).round() as u32;
        (w, h.max(1))
    } else {
        let h = dst_h;
        let w = (dst_h as f32 * src_aspect).round() as u32;
        (w.max(1), h)
    };
    let x = ((dst_w as i32) - (w as i32)) / 2;
    let y = ((dst_h as i32) - (h as i32)) / 2;
    Rect::new(x, y, w, h)
}

/// Translate an SDL keycode into the same crossterm `KeyCode` + `KeyModifiers`
/// pair the main loop already dispatches on, so existing shortcuts work
/// uniformly regardless of which window has focus.
fn translate_key(kc: Keycode, keymod: Mod) -> Option<ForwardedKey> {
    let mut mods = KeyModifiers::empty();
    let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    let shift = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
    let alt = keymod.intersects(Mod::LALTMOD | Mod::RALTMOD);
    if ctrl {
        mods |= KeyModifiers::CONTROL;
    }
    if shift {
        mods |= KeyModifiers::SHIFT;
    }
    if alt {
        mods |= KeyModifiers::ALT;
    }

    let code = match kc {
        Keycode::Escape => KeyCode::Esc,
        Keycode::Return | Keycode::Return2 | Keycode::KpEnter => KeyCode::Enter,
        Keycode::Tab => KeyCode::Tab,
        Keycode::Backspace => KeyCode::Backspace,
        Keycode::Space => KeyCode::Char(' '),
        Keycode::Left => KeyCode::Left,
        Keycode::Right => KeyCode::Right,
        Keycode::Up => KeyCode::Up,
        Keycode::Down => KeyCode::Down,
        Keycode::PageUp => KeyCode::PageUp,
        Keycode::PageDown => KeyCode::PageDown,
        Keycode::Home => KeyCode::Home,
        Keycode::End => KeyCode::End,
        Keycode::Minus | Keycode::KpMinus => KeyCode::Char('-'),
        Keycode::Equals | Keycode::Plus | Keycode::KpPlus => {
            KeyCode::Char(if shift { '+' } else { '=' })
        }
        Keycode::LeftBracket => KeyCode::Char('['),
        Keycode::RightBracket => KeyCode::Char(']'),
        Keycode::Question => KeyCode::Char('?'),
        Keycode::Slash => KeyCode::Char(if shift { '?' } else { '/' }),
        other => {
            // SDL maps letter keys to their lowercase ASCII codes (97..=122).
            let raw = other.into_i32();
            if (b'a' as i32..=b'z' as i32).contains(&raw) {
                let lower = raw as u8 as char;
                // Crossterm convention: Ctrl+<letter> chords use the
                // lowercase form; the matching arm in `handle_key` expects
                // that. Without Ctrl, shift uppercases the char.
                let ch = if !ctrl && shift {
                    lower.to_ascii_uppercase()
                } else {
                    lower
                };
                KeyCode::Char(ch)
            } else {
                return None;
            }
        }
    };

    Some(ForwardedKey { code, mods })
}

/// Cached subtitle texture so we don't re-rasterize glyphs every frame — only
/// when the displayed line actually changes.
struct SubtitleTexture<'r> {
    text: String,
    texture: sdl2::render::Texture<'r>,
    width: u32,
    height: u32,
}

/// Refresh `cache` in place: rasterizes a new texture only when `text` differs
/// from the cached entry. Clears the cache when `text` is None or empty.
fn refresh_subtitle_cache<'r>(
    cache: &mut Option<SubtitleTexture<'r>>,
    font: Option<&FontVec>,
    text: Option<&str>,
    creator: &'r sdl2::render::TextureCreator<sdl2::video::WindowContext>,
    out_w: u32,
) {
    let text = text.unwrap_or("").trim();
    if text.is_empty() || font.is_none() {
        *cache = None;
        return;
    }
    if let Some(c) = cache.as_ref() {
        if c.text == text {
            return;
        }
    }
    let font = font.unwrap();
    // Font size scales with window width so subs stay legible at any
    // resolution. Roughly 3.5% of width, clamped to a sensible range.
    let px = ((out_w as f32) * 0.035).clamp(22.0, 64.0);
    let max_w = ((out_w as f32) * 0.9) as u32;
    let Some((rgba, w, h)) = rasterize_subtitle(font, text, px, max_w) else {
        *cache = None;
        return;
    };
    let mut texture = match creator.create_texture_static(PixelFormatEnum::ABGR8888, w, h) {
        Ok(t) => t,
        Err(_) => {
            *cache = None;
            return;
        }
    };
    texture.set_blend_mode(BlendMode::Blend);
    if texture.update(None, &rgba, (w * 4) as usize).is_err() {
        *cache = None;
        return;
    }
    *cache = Some(SubtitleTexture {
        text: text.to_string(),
        texture,
        width: w,
        height: h,
    });
}

/// Rasterize one or more lines (split on '\n') into an RGBA buffer with a
/// black outline + white fill. Returns the buffer and its dimensions.
fn rasterize_subtitle(
    font: &FontVec,
    text: &str,
    px: f32,
    max_w: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let scale = PxScale::from(px);
    let sf = font.as_scaled(scale);
    let line_h = (sf.height() + sf.line_gap()).ceil() as i32;
    let ascent = sf.ascent().ceil() as i32;

    // Wrap on whitespace if a line exceeds max_w; otherwise keep author breaks.
    let mut lines: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        wrap_line(raw_line, &sf, max_w as f32, &mut lines);
    }
    if lines.is_empty() {
        return None;
    }

    // Lay out glyphs per line, tracking max width.
    let outline = 2_i32;
    let mut per_line: Vec<(Vec<(ab_glyph::Glyph, f32)>, f32)> = Vec::new();
    let mut max_line_w = 0.0_f32;
    for line in &lines {
        let mut cursor = 0.0_f32;
        let mut glyphs: Vec<(ab_glyph::Glyph, f32)> = Vec::new();
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            if let Some(p) = prev {
                cursor += sf.kern(p, gid);
            }
            let glyph = gid.with_scale_and_position(scale, ab_glyph::point(cursor, 0.0));
            let advance = sf.h_advance(gid);
            glyphs.push((glyph, cursor));
            cursor += advance;
            prev = Some(gid);
        }
        max_line_w = max_line_w.max(cursor);
        per_line.push((glyphs, cursor));
    }

    let pad = outline + 2;
    let img_w = (max_line_w.ceil() as i32 + pad * 2).max(1) as u32;
    let img_h = (line_h * lines.len() as i32 + pad * 2).max(1) as u32;
    let mut buf = vec![0_u8; (img_w * img_h * 4) as usize];

    // Two passes per glyph: outline (black) at offsets, then fill (white).
    for (li, (glyphs, line_w)) in per_line.iter().enumerate() {
        let x_off = pad + ((max_line_w - line_w) / 2.0).round() as i32;
        let y_off = pad + (li as i32) * line_h + ascent;
        for (glyph, _) in glyphs {
            if let Some(outlined) = font.outline_glyph(glyph.clone()) {
                let bb = outlined.px_bounds();
                let gx = x_off + bb.min.x as i32;
                let gy = y_off + bb.min.y as i32;
                // Outline pass — splat coverage at 8 offsets.
                for dy in -outline..=outline {
                    for dx in -outline..=outline {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        if dx * dx + dy * dy > outline * outline {
                            continue;
                        }
                        outlined.draw(|gpx, gpy, cov| {
                            let px_ = gx + gpx as i32 + dx;
                            let py_ = gy + gpy as i32 + dy;
                            blend_pixel(&mut buf, img_w, img_h, px_, py_, 0, 0, 0, cov);
                        });
                    }
                }
                // Fill pass — white on top.
                outlined.draw(|gpx, gpy, cov| {
                    let px_ = gx + gpx as i32;
                    let py_ = gy + gpy as i32;
                    blend_pixel(&mut buf, img_w, img_h, px_, py_, 255, 255, 255, cov);
                });
            }
        }
    }

    Some((buf, img_w, img_h))
}

/// Naive whitespace word-wrap: appends one or more wrapped fragments of `raw`
/// to `out`. Falls back to a hard break if a single word exceeds `max_w`.
fn wrap_line(
    raw: &str,
    sf: &ab_glyph::PxScaleFont<&FontVec>,
    max_w: f32,
    out: &mut Vec<String>,
) {
    let trimmed = raw.trim_end();
    if trimmed.is_empty() {
        return;
    }
    let mut current = String::new();
    let mut current_w = 0.0_f32;
    let mut iter = trimmed.split_whitespace().peekable();
    while let Some(word) = iter.next() {
        let word_w: f32 = word.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum();
        let space_w = if current.is_empty() {
            0.0
        } else {
            sf.h_advance(sf.glyph_id(' '))
        };
        if current_w + space_w + word_w > max_w && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_w += space_w;
        }
        current.push_str(word);
        current_w += word_w;
    }
    if !current.is_empty() {
        out.push(current);
    }
}

/// Alpha-over an (r,g,b) sample weighted by `cov` (0..=1) into the RGBA buffer.
#[inline]
fn blend_pixel(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    r: u8,
    g: u8,
    b: u8,
    cov: f32,
) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 || cov <= 0.0 {
        return;
    }
    let idx = ((y as u32 * w + x as u32) * 4) as usize;
    let src_a = (cov.clamp(0.0, 1.0) * 255.0).round() as u8;
    if src_a == 0 {
        return;
    }
    let dst_r = buf[idx];
    let dst_g = buf[idx + 1];
    let dst_b = buf[idx + 2];
    let dst_a = buf[idx + 3];
    // "src over dst" porter-duff in 8-bit.
    let sa = src_a as u32;
    let da = dst_a as u32;
    let inv = 255 - sa;
    let out_a = sa + (da * inv) / 255;
    if out_a == 0 {
        return;
    }
    let mix = |s: u8, d: u8| -> u8 {
        let n = (s as u32) * sa + (d as u32) * da * inv / 255;
        ((n + (out_a / 2).max(1)) / out_a.max(1)) as u8
    };
    buf[idx] = mix(r, dst_r);
    buf[idx + 1] = mix(g, dst_g);
    buf[idx + 2] = mix(b, dst_b);
    buf[idx + 3] = out_a as u8;
}

/// Walk a handful of well-known font paths and load the first match. Returns
/// None when no usable font is found; the window will then render video
/// without subtitles, with no other side effects.
fn load_system_font() -> Option<FontVec> {
    const CANDIDATES: &[&str] = &[
        // Linux — Arch / RHEL / Fedora / generic.
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        // Linux — Debian / Ubuntu.
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        // Liberation as a fallback (RHEL family without DejaVu).
        "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/TTF/LiberationSans-Bold.ttf",
        // Noto Sans is the GNOME default on many distros.
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Bold.ttf",
        // Ubuntu-bundled.
        "/usr/share/fonts/ubuntu/Ubuntu-B.ttf",
        // macOS.
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        // Windows.
        "C:\\Windows\\Fonts\\arialbd.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(font) = FontVec::try_from_vec(bytes) {
                return Some(font);
            }
        }
    }
    None
}
