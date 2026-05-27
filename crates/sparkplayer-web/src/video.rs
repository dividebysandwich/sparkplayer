//! Web video backend. The actual picture is a real `<video>` element (the same
//! element the audio graph plays through) floated over the ratzilla canvas;
//! `lib.rs` positions/hides it after each draw using `App::last_video_rect`.
//! This backend only tracks whether the current track *is* a video so the core
//! layout knows to reserve the video panel.

use std::time::Duration;

use web_sys::HtmlVideoElement;

use sparkplayer_core::backend::VideoBackend;
use sparkplayer_core::library::{self, TrackRef};
use sparkplayer_core::ratatui::layout::Rect;
use sparkplayer_core::ratatui::Frame;

pub struct WebVideoBackend {
    element: HtmlVideoElement,
    loaded: bool,
}

impl WebVideoBackend {
    pub fn new(element: HtmlVideoElement) -> Self {
        Self {
            element,
            loaded: false,
        }
    }
}

impl VideoBackend for WebVideoBackend {
    fn open(&mut self, source: &TrackRef) -> Option<(u32, u32)> {
        self.loaded = library::is_video(source);
        // Real pixel dims arrive asynchronously with the element's metadata; the
        // overlay uses object-fit:contain so exact dims aren't needed here.
        if self.loaded {
            Some((
                self.element.video_width().max(1),
                self.element.video_height().max(1),
            ))
        } else {
            None
        }
    }

    fn close(&mut self) {
        self.loaded = false;
    }

    fn is_loaded(&self) -> bool {
        self.loaded
    }

    fn has_image(&self) -> bool {
        self.loaded
    }

    fn seek(&self, _target: Duration) {
        // The shared element is already sought by the audio backend.
    }

    fn advance(&mut self, _display_pos: f64, _paused: bool, _subtitle: Option<&str>) -> Option<f64> {
        // The `<video>` element self-syncs to its own clock; nothing to pull.
        None
    }

    fn render(&mut self, _frame: &mut Frame, _area: Rect) {
        // No terminal-cell drawing on web; positioning happens post-draw.
    }
}
