//! Native implementations of the core platform traits that aren't audio:
//! video (ffmpeg + ratatui-image + optional SDL window), album art
//! (ratatui-image), and the config store (a file under the OS config dir).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{FontSize, Resize, StatefulImage};

use sparkplayer_core::backend::{AlbumArtRenderer, ConfigStore, CoreKeyEvent, VideoBackend};
use sparkplayer_core::config::Config;
use sparkplayer_core::library::{self, TrackRef};

use crate::external_window::{ExternalVideoWindow, VideoSync};
use crate::map_key;
use crate::video::VideoStream;

/// Album-art / video share one terminal capability probe (`Picker`). Querying
/// the terminal twice would consume each other's escape responses, so we query
/// once and hand both renderers a clone of the same cell.
pub type SharedPicker = Rc<RefCell<Picker>>;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum GraphicsChoice {
    Auto,
    Halfblocks,
    Sixel,
    Kitty,
    Iterm,
}

impl GraphicsChoice {
    fn into_protocol(self) -> Option<ProtocolType> {
        match self {
            GraphicsChoice::Auto => None,
            GraphicsChoice::Halfblocks => Some(ProtocolType::Halfblocks),
            GraphicsChoice::Sixel => Some(ProtocolType::Sixel),
            GraphicsChoice::Kitty => Some(ProtocolType::Kitty),
            GraphicsChoice::Iterm => Some(ProtocolType::Iterm2),
        }
    }
}

/// Probe the terminal for graphics capabilities. Must be called after raw mode
/// is enabled. Falls back to halfblocks when the terminal doesn't respond.
pub fn build_picker(choice: GraphicsChoice) -> SharedPicker {
    let mut picker = Picker::from_query_stdio()
        .ok()
        .unwrap_or_else(Picker::halfblocks);
    if let Some(forced) = choice.into_protocol() {
        picker.set_protocol_type(forced);
    }
    Rc::new(RefCell::new(picker))
}

/// Fit `(iw, ih)` pixels into `area` cells (preserving aspect) given the
/// terminal's cell pixel size, returning the centered cell rectangle.
fn fit_image_rect(area: Rect, iw: u32, ih: u32, font: FontSize) -> Rect {
    let iw = iw.max(1);
    let ih = ih.max(1);
    let font_w = font.width.max(1) as u32;
    let font_h = font.height.max(1) as u32;
    let avail_w_px = area.width as u32 * font_w;
    let avail_h_px = area.height as u32 * font_h;
    let scale = (avail_w_px as f64 / iw as f64).min(avail_h_px as f64 / ih as f64);
    let fit_w_px = (iw as f64 * scale).round() as u32;
    let fit_h_px = (ih as f64 * scale).round() as u32;
    let cells_w = ((fit_w_px + font_w - 1) / font_w)
        .max(1)
        .min(area.width as u32) as u16;
    let cells_h = ((fit_h_px + font_h - 1) / font_h)
        .max(1)
        .min(area.height as u32) as u16;
    let x = area.x + (area.width - cells_w) / 2;
    let y = area.y + (area.height - cells_h) / 2;
    Rect::new(x, y, cells_w, cells_h)
}

pub struct NativeVideoBackend {
    picker: SharedPicker,
    video: Option<VideoStream>,
    video_protocol: Option<StatefulProtocol>,
    video_dims: Option<(u32, u32)>,
    external_window: Option<ExternalVideoWindow>,
    external_window_enabled: bool,
    /// Playback state shared with the SDL window thread, which uses it to pace
    /// frame selection at vsync and to drive the OSD. Lives for the backend's
    /// lifetime so `publish_clock`/`show_osd` work even before a window spawns.
    sync: Arc<VideoSync>,
}

impl NativeVideoBackend {
    pub fn new(picker: SharedPicker) -> Self {
        Self {
            picker,
            video: None,
            video_protocol: None,
            video_dims: None,
            external_window: None,
            external_window_enabled: false,
            sync: VideoSync::new(),
        }
    }

    fn spawn_external_window(&mut self) {
        if self.external_window.is_some() || self.video.is_none() {
            return;
        }
        // Point the window at the current decoder queue before it starts.
        if let Some(v) = self.video.as_ref() {
            self.sync.set_queue(v.queue_handle());
        }
        match ExternalVideoWindow::spawn(Arc::clone(&self.sync)) {
            Ok(w) => {
                self.external_window = Some(w);
                self.video_protocol = None;
            }
            Err(_) => {
                self.external_window_enabled = false;
            }
        }
    }
}

impl VideoBackend for NativeVideoBackend {
    fn open(&mut self, source: &TrackRef) -> Option<(u32, u32)> {
        let TrackRef::Path(path) = source else {
            return None;
        };
        if !library::is_video_file(path) {
            return None;
        }
        match VideoStream::open(path) {
            Ok(v) => {
                let dims = (v.width, v.height);
                self.video_dims = Some(dims);
                self.video = Some(v);
                if self.external_window.is_some() {
                    // Window already open (switching videos): repoint it at the
                    // new decoder queue.
                    if let Some(v) = self.video.as_ref() {
                        self.sync.set_queue(v.queue_handle());
                    }
                } else if self.external_window_enabled {
                    self.spawn_external_window();
                }
                Some(dims)
            }
            Err(_) => None,
        }
    }

    fn close(&mut self) {
        self.video = None;
        self.video_protocol = None;
        self.video_dims = None;
        // Drop the window but keep the armed flag, so the next video reopens it.
        self.external_window = None;
    }

    fn is_loaded(&self) -> bool {
        self.video.is_some()
    }

    fn has_image(&self) -> bool {
        self.video_protocol.is_some()
    }

    fn seek(&self, target: Duration) {
        if let Some(v) = self.video.as_ref() {
            v.seek(target);
            // The seek clears the decoder queue; let the window present the
            // first frame at the new position.
            self.sync.reset_frame_marker();
        }
    }

    fn publish_clock(&self, display_pos: f64, paused: bool, duration: Option<f64>) {
        self.sync.publish(display_pos, paused, duration);
    }

    fn show_osd(&self, message: Option<String>) {
        self.sync.show_osd(message);
    }

    fn advance(&mut self, display_pos: f64, _paused: bool, subtitle: Option<&str>) -> Option<f64> {
        // Drop the external window if it died (close button / init failure).
        if self.external_window.as_ref().is_some_and(|w| !w.is_alive()) {
            self.external_window = None;
            self.external_window_enabled = false;
        }

        // If the SDL window is up it owns frame selection (vsync-locked); we
        // only feed it the current subtitle line. The in-terminal image path is
        // skipped entirely — it is expensive and pointless behind the window.
        if let Some(window) = self.external_window.as_ref() {
            window.set_subtitle(subtitle.map(|s| s.to_string()));
            return None;
        }

        let video = self.video.as_ref()?;
        let frame = video.frame_at(display_pos)?;
        let started = Instant::now();
        let buf = image::RgbImage::from_raw(frame.width, frame.height, frame.data)?;
        let dyn_img = image::DynamicImage::ImageRgb8(buf);
        self.video_dims = Some((frame.width, frame.height));
        let proto = self.picker.borrow_mut().new_resize_protocol(dyn_img);
        self.video_protocol = Some(proto);
        Some(started.elapsed().as_secs_f64())
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let font = self.picker.borrow().font_size();
        let dims = self.video_dims;
        if let Some(proto) = self.video_protocol.as_mut() {
            let (iw, ih) = dims.unwrap_or((1, 1));
            let img_area = fit_image_rect(area, iw, ih, font);
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Scale(None)),
                img_area,
                proto,
            );
        }
    }

    fn supports_external_window(&self) -> bool {
        true
    }

    fn external_window_enabled(&self) -> bool {
        self.external_window_enabled
    }

    fn set_external_window(&mut self, enabled: bool) {
        if enabled {
            self.external_window_enabled = true;
            if self.video.is_some() {
                self.spawn_external_window();
            }
        } else {
            self.external_window_enabled = false;
            self.external_window = None;
        }
    }

    fn drain_external_keys(&self) -> Vec<CoreKeyEvent> {
        let Some(window) = self.external_window.as_ref() else {
            return Vec::new();
        };
        window
            .drain_keys()
            .into_iter()
            .map(|k| map_key(k.code, k.mods))
            .collect()
    }
}

pub struct NativeAlbumArt {
    picker: SharedPicker,
    protocol: Option<StatefulProtocol>,
    dims: Option<(u32, u32)>,
    last_key: Option<(usize, usize)>,
}

impl NativeAlbumArt {
    pub fn new(picker: SharedPicker) -> Self {
        Self {
            picker,
            protocol: None,
            dims: None,
            last_key: None,
        }
    }
}

impl AlbumArtRenderer for NativeAlbumArt {
    fn set_artwork(&mut self, bytes: Option<&[u8]>, key: Option<(usize, usize)>) {
        let Some(bytes) = bytes else {
            self.protocol = None;
            self.dims = None;
            self.last_key = None;
            return;
        };
        if self.last_key == key && key.is_some() && self.protocol.is_some() {
            return;
        }
        match image::load_from_memory(bytes) {
            Ok(img) => {
                self.dims = Some((img.width(), img.height()));
                self.protocol = Some(self.picker.borrow_mut().new_resize_protocol(img));
                self.last_key = key;
            }
            Err(_) => {
                self.protocol = None;
                self.dims = None;
                self.last_key = None;
            }
        }
    }

    fn has_art(&self) -> bool {
        self.protocol.is_some()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let font = self.picker.borrow().font_size();
        let dims = self.dims;
        if let Some(proto) = self.protocol.as_mut() {
            let (iw, ih) = dims.unwrap_or((1, 1));
            let img_area = fit_image_rect(area, iw, ih, font);
            // `Resize::Scale` (not the default `Resize::Fit`) so small artwork is
            // upscaled to fill the area — Fit only ever scales down, leaving the
            // image stuck at its native size when the window grows.
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Scale(None)),
                img_area,
                proto,
            );
        }
    }

    fn graphics_available(&self) -> bool {
        // Halfblocks are just colored cells, not pixel graphics.
        !matches!(self.picker.borrow().protocol_type(), ProtocolType::Halfblocks)
    }

    fn render_rgb_frame(&mut self, frame: &mut Frame, area: Rect, rgb: &[u8], w: u32, h: u32) {
        if area.width == 0 || area.height == 0 || w == 0 || h == 0 {
            return;
        }
        let Some(src) = image::RgbImage::from_raw(w, h, rgb.to_vec()) else {
            return;
        };
        // Stretch to the panel's exact pixel size (fill, not aspect-fit) with a
        // smooth filter — the interpolated look of a hardware waterfall.
        let font = self.picker.borrow().font_size();
        let px_w = (area.width as u32 * font.width.max(1) as u32).max(1);
        let px_h = (area.height as u32 * font.height.max(1) as u32).max(1);
        let stretched =
            image::imageops::resize(&src, px_w, px_h, image::imageops::FilterType::Triangle);
        let dyn_img = image::DynamicImage::ImageRgb8(stretched);
        let mut proto = self.picker.borrow_mut().new_resize_protocol(dyn_img);
        frame.render_stateful_widget(
            StatefulImage::default().resize(Resize::Scale(None)),
            area,
            &mut proto,
        );
    }
}

/// Config store backed by a file under the OS config dir.
pub struct NativeConfigStore;

impl NativeConfigStore {
    fn path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("sparkplayer").join("config.toml"))
    }
}

impl ConfigStore for NativeConfigStore {
    fn load(&self) -> Config {
        let Some(path) = Self::path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => Config::parse(&content),
            Err(_) => Config::default(),
        }
    }

    fn save(&self, cfg: &Config) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let _ = std::fs::write(&path, cfg.serialize());
    }
}
