use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;

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
    key_rx: Receiver<ForwardedKey>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    init_err: Arc<Mutex<Option<String>>>,
}

impl ExternalVideoWindow {
    pub fn spawn() -> Result<Self> {
        let latest_frame: Arc<Mutex<Option<VideoFrame>>> = Arc::new(Mutex::new(None));
        let (key_tx, key_rx): (Sender<ForwardedKey>, Receiver<ForwardedKey>) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let init_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();

        let latest_frame_t = Arc::clone(&latest_frame);
        let stop_t = Arc::clone(&stop);
        let init_err_t = Arc::clone(&init_err);

        let handle = thread::Builder::new()
            .name("sparkplayer-video-window".into())
            .spawn(move || {
                if let Err(e) = run_window(latest_frame_t, key_tx, stop_t, ready_tx) {
                    *init_err_t.lock().unwrap() = Some(e.to_string());
                }
            })
            .context("spawning external video window thread")?;

        // Wait briefly for the window thread to confirm initialization so we
        // can surface a clean error to the user instead of silently failing.
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Self {
                latest_frame,
                key_rx,
                stop,
                handle: Some(handle),
                init_err,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("video window init failed: {e}")),
            Err(_) => Err(anyhow::anyhow!("video window init timed out")),
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
            if let Some(tex) = texture.as_ref() {
                let (out_w, out_h) = canvas.output_size().unwrap_or((tex_w, tex_h));
                let dst = aspect_fit(tex_w, tex_h, out_w, out_h);
                let _ = canvas.copy(tex, None, dst);
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
