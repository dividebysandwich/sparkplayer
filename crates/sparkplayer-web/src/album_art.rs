//! Web album-art backend. Artwork bytes become a blob object URL on an `<img>`
//! element that `lib.rs` floats over the canvas. (The web `MediaLibrary`
//! currently returns no artwork bytes, so this stays inert unless a future
//! source supplies them — the plumbing is here either way.)

use web_sys::{Blob, HtmlImageElement, Url};

use sparkplayer_core::backend::AlbumArtRenderer;
use sparkplayer_core::ratatui::layout::Rect;
use sparkplayer_core::ratatui::Frame;

pub struct WebAlbumArt {
    img: HtmlImageElement,
    has_art: bool,
    object_url: Option<String>,
    last_key: Option<(usize, usize)>,
}

impl WebAlbumArt {
    pub fn new(img: HtmlImageElement) -> Self {
        Self {
            img,
            has_art: false,
            object_url: None,
            last_key: None,
        }
    }

    fn revoke(&mut self) {
        if let Some(url) = self.object_url.take() {
            let _ = Url::revoke_object_url(&url);
        }
    }
}

impl AlbumArtRenderer for WebAlbumArt {
    fn set_artwork(&mut self, bytes: Option<&[u8]>, key: Option<(usize, usize)>) {
        let Some(bytes) = bytes else {
            self.revoke();
            self.has_art = false;
            self.last_key = None;
            return;
        };
        if self.last_key == key && key.is_some() && self.has_art {
            return;
        }
        let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        array.copy_from(bytes);
        let parts = js_sys::Array::of1(&array);
        match Blob::new_with_u8_array_sequence(&parts) {
            Ok(blob) => match make_object_url(&blob) {
                Ok(url) => {
                    self.revoke();
                    self.img.set_src(&url);
                    self.object_url = Some(url);
                    self.has_art = true;
                    self.last_key = key;
                }
                Err(_) => self.has_art = false,
            },
            Err(_) => self.has_art = false,
        }
    }

    fn has_art(&self) -> bool {
        self.has_art
    }

    fn render(&mut self, _frame: &mut Frame, _area: Rect) {
        // No terminal-cell drawing on web; positioning happens post-draw.
    }
}

fn make_object_url(blob: &Blob) -> Result<String, wasm_bindgen::JsValue> {
    Url::create_object_url_with_blob(blob)
}
