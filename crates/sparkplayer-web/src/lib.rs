//! Browser/WASM entry point for SparkPlayer, rendered with Ratzilla.
//!
//! Wiring:
//! - One hidden `<video>` element decodes/plays the media; its audio routes
//!   through a Web Audio graph ([`audio`]) that taps samples for the visualizer,
//!   and it doubles as the on-screen picture, floated over the ratzilla canvas.
//! - An `<img>` element does the same for album art.
//! - Media comes from a fetched `manifest.json` (web-playlist mode) or from
//!   user-picked local files (the `#file-input` element).

mod album_art;
mod audio;
mod library;
mod metadata_web;
mod video;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    Document, Event, HtmlElement, HtmlImageElement, HtmlInputElement, HtmlVideoElement, Url,
    Window,
};

use ratzilla::backend::canvas::CanvasBackendOptions;
use ratzilla::CanvasBackend;

use sparkplayer_core::backend::{CoreKey, CoreKeyEvent};
use sparkplayer_core::library::Track;
use sparkplayer_core::ratatui::layout::Rect;
use sparkplayer_core::ratatui::Terminal;
use sparkplayer_core::subtitles::{self, SubtitleTrack};
use sparkplayer_core::App;

use crate::album_art::WebAlbumArt;
use crate::audio::WebAudioBackend;
use crate::library::{LocalStorageConfig, MetaMap, WebLibrary};
use crate::video::WebVideoBackend;

const ROOT_ID: &str = "sparkplayer-root";
const MANIFEST_URL: &str = "manifest.json";

type SharedApp = Rc<RefCell<App>>;

/// Per-track extras from the manifest that load asynchronously after a track
/// starts: a cover-image URL and a `.vtt` subtitle URL. Keyed by track locator.
#[derive(Clone, Default)]
struct TrackExtra {
    artwork: Option<String>,
    subtitles: Option<String>,
}

type Extras = Rc<RefCell<HashMap<String, TrackExtra>>>;

/// Convert any `Display` error (ratzilla / io) into a `JsValue` for `?`.
fn to_js<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    // The shared media element: plays audio (routed through the analyser) and
    // shows video when floated over the canvas.
    let video_el: HtmlVideoElement = document
        .create_element("video")?
        .dyn_into::<HtmlVideoElement>()?;
    let img_el: HtmlImageElement = document
        .create_element("img")?
        .dyn_into::<HtmlImageElement>()?;
    init_overlay(&video_el)?;
    init_overlay(&img_el)?;
    if let Some(body) = document.body() {
        body.append_child(&video_el)?;
        body.append_child(&img_el)?;
    }

    // Metadata parsed from picked local files, shared with the file-input
    // handler and read back through the library.
    let meta_map: MetaMap = Rc::new(RefCell::new(HashMap::new()));

    let audio = WebAudioBackend::new(video_el.clone())?;
    let video = WebVideoBackend::new(video_el.clone());
    let art = WebAlbumArt::new(img_el.clone());
    let config = LocalStorageConfig;
    let cfg = sparkplayer_core::backend::ConfigStore::load(&config);

    let mut app = App::new(
        Box::new(audio),
        Box::new(video),
        Box::new(WebLibrary::new(meta_map.clone())),
        Box::new(config),
        Box::new(art),
        Vec::new(),
        PathBuf::new(),
        &cfg,
    );
    // The browser can open external links, so expose the escape-menu GitHub entry.
    app.url_open_supported = true;
    let app: SharedApp = Rc::new(std::cell::RefCell::new(app));
    app.borrow_mut().status =
        String::from("Press any key (or pick files) to start — browser audio needs a gesture");

    // Per-track artwork/subtitle URLs, populated from the manifest and loaded
    // lazily when each track starts.
    let extras: Extras = Rc::new(RefCell::new(HashMap::new()));

    // Fetch the manifest; if it lists tracks, switch to web-playlist mode.
    spawn_local(load_manifest(app.clone(), extras.clone()));

    // Wire the local-file and folder pickers, if present.
    wire_file_input(&document, "file-input", app.clone(), meta_map.clone());
    wire_file_input(&document, "dir-input", app.clone(), meta_map.clone());

    // The canvas backend can't resize its grid after creation, so we own the
    // terminal in a cell and rebuild it whenever the window resizes. We also
    // drive rendering with our own requestAnimationFrame loop (instead of
    // ratzilla's `draw_web`, which owns an unstoppable loop) so the terminal can
    // be swapped cleanly, and handle keys with our own listener.
    let terminal: Rc<RefCell<Option<Terminal<CanvasBackend>>>> = Rc::new(RefCell::new(None));
    rebuild_terminal(&window, &document, &terminal)?;

    // Mark a resize as pending; the render loop rebuilds at most once per frame.
    let resize_pending = Rc::new(Cell::new(false));
    {
        let resize_pending = resize_pending.clone();
        let on_resize = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
            resize_pending.set(true);
        });
        let _ =
            window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        on_resize.forget();
    }

    install_keyboard(&document, app.clone());

    // Render loop.
    {
        let app = app.clone();
        let window = window.clone();
        let document = document.clone();
        let video_el = video_el.clone();
        let img_el = img_el.clone();
        let terminal = terminal.clone();
        let perf = window.performance();
        let loaded_extras_for: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
        let g = f.clone();
        *g.borrow_mut() = Some(Closure::<dyn FnMut()>::new(move || {
            if resize_pending.replace(false) {
                let _ = rebuild_terminal(&window, &document, &terminal);
            }
            let now = perf.as_ref().map(|p| p.now() / 1000.0).unwrap_or(0.0);
            if let Some(term) = terminal.borrow_mut().as_mut() {
                let mut a = app.borrow_mut();
                a.set_clock(now);
                a.audio.pump();
                if a.current_duration.is_none() {
                    a.current_duration = a.audio.duration();
                }
                let _ = a.check_advance();
                a.tick_video();
                let mut area = Rect::default();
                let _ = term.draw(|frame| {
                    area = frame.area();
                    sparkplayer_core::ui::draw(frame, &mut a);
                });
                position_overlays(&document, &a, &video_el, &img_el, area);

                // Open a URL queued by the escape menu (the GitHub entry).
                if let Some(url) = a.take_pending_url_open() {
                    let _ = window.open_with_url_and_target(&url, "_blank");
                }

                // On track change, load this track's manifest extras.
                let cur = a.playing_track.as_ref().map(|t| t.locator());
                if cur != *loaded_extras_for.borrow() {
                    *loaded_extras_for.borrow_mut() = cur.clone();
                    if let Some(url) = cur {
                        if let Some(extra) = extras.borrow().get(&url).cloned() {
                            load_track_extras(app.clone(), url, extra);
                        }
                    }
                }
            }
            request_animation_frame(f.borrow().as_ref().unwrap());
        }));
        request_animation_frame(g.borrow().as_ref().unwrap());
    }

    Ok(())
}

/// Point the canvas backend's 2D context at the bundled Meslo Nerd Font Mono.
/// `getContext("2d")` returns the same context ratzilla drew with, so this
/// sticks for all subsequent frames.
fn set_canvas_font(document: &Document) {
    let Some(canvas) = document
        .query_selector("#sparkplayer-root canvas")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        return;
    };
    if let Ok(Some(ctx)) = canvas.get_context("2d") {
        if let Ok(ctx) = ctx.dyn_into::<web_sys::CanvasRenderingContext2d>() {
            ctx.set_font("16px 'MesloLGM Nerd Font Mono', monospace");
            install_text_centering(&ctx);
        }
    }
}

/// ratzilla's canvas backend top-aligns glyphs (textBaseline "top", drawn at
/// `row*CELL_HEIGHT`), but its 19px cell is taller than the 16px font, so text
/// hugs the top. Cell backgrounds are drawn with `fillRect` (unaffected), while
/// glyphs go through `fillText` — so we shadow the context's `fillText` with a
/// wrapper that nudges the y down, vertically centering the text only.
fn install_text_centering(ctx: &web_sys::CanvasRenderingContext2d) {
    // ~half the (cell - font) leading, biased a touch lower since the visible
    // glyph sits high within the em box.
    const Y_OFFSET: f64 = 2.5;
    let key = JsValue::from_str("fillText");
    let Ok(orig) = js_sys::Reflect::get(ctx, &key) else {
        return;
    };
    if !orig.is_function() {
        return;
    }
    let orig_fn: js_sys::Function = orig.unchecked_into();
    let this: JsValue = ctx.clone().into();
    let wrapper = Closure::<dyn FnMut(JsValue, f64, f64) -> JsValue>::new(
        move |text: JsValue, x: f64, y: f64| -> JsValue {
            let args =
                js_sys::Array::of3(&text, &JsValue::from_f64(x), &JsValue::from_f64(y + Y_OFFSET));
            orig_fn.apply(&this, &args).unwrap_or(JsValue::UNDEFINED)
        },
    );
    let _ = js_sys::Reflect::set(ctx, &key, wrapper.as_ref());
    wrapper.forget();
}

/// Current inner window size in CSS pixels, for sizing the canvas grid.
fn window_size(window: &Window) -> (u32, u32) {
    let w = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(1280.0);
    let h = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(720.0);
    (w.max(1.0) as u32, h.max(1.0) as u32)
}

/// (Re)create the canvas terminal sized to the current window, replacing any
/// previous one. Clears the old `<canvas>` out of the root first so they don't
/// stack, and re-applies the Meslo font to the fresh context.
fn rebuild_terminal(
    window: &Window,
    document: &Document,
    cell: &Rc<RefCell<Option<Terminal<CanvasBackend>>>>,
) -> Result<(), JsValue> {
    *cell.borrow_mut() = None;
    if let Some(root) = document.get_element_by_id(ROOT_ID) {
        root.set_inner_html("");
    }
    let (cw, ch) = window_size(window);
    let backend = CanvasBackend::new_with_options(
        CanvasBackendOptions::default().grid_id(ROOT_ID).size((cw, ch)),
    )
    .map_err(to_js)?;
    let term = Terminal::new(backend).map_err(to_js)?;
    // The canvas backend hardcodes "16px monospace" at creation and reuses that
    // context for every fill_text; point it at the bundled Meslo Nerd Font.
    set_canvas_font(document);
    *cell.borrow_mut() = Some(term);
    Ok(())
}

fn request_animation_frame(closure: &Closure<dyn FnMut()>) {
    if let Some(w) = web_sys::window() {
        let _ = w.request_animation_frame(closure.as_ref().unchecked_ref());
    }
}

/// Install a `keydown` listener that maps browser key events to the core keymap.
/// Independent of the terminal's lifetime so resizes don't drop it.
fn install_keyboard(document: &Document, app: SharedApp) {
    let first_gesture = Rc::new(Cell::new(true));
    let handler = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
        let core = map_keyboard_event(&ev);
        // Stop the browser from scrolling/focus-cycling on keys we consume.
        if should_prevent_default(core.code) {
            ev.prevent_default();
        }
        let mut a = app.borrow_mut();
        if first_gesture.replace(false) {
            a.audio.on_user_gesture();
            if a.playing_index.is_none() && !a.tracks.is_empty() {
                let _ = a.play_index(0);
            }
        }
        let _ = a.handle_key(core);
    });
    let _ = document.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();
}

/// Map a browser `KeyboardEvent` to the platform-neutral [`CoreKeyEvent`].
fn map_keyboard_event(ev: &web_sys::KeyboardEvent) -> CoreKeyEvent {
    let key = ev.key();
    let code = match key.as_str() {
        "ArrowUp" => CoreKey::Up,
        "ArrowDown" => CoreKey::Down,
        "ArrowLeft" => CoreKey::Left,
        "ArrowRight" => CoreKey::Right,
        "Enter" => CoreKey::Enter,
        "Tab" => CoreKey::Tab,
        "Escape" => CoreKey::Esc,
        "Home" => CoreKey::Home,
        "End" => CoreKey::End,
        "PageUp" => CoreKey::PageUp,
        "PageDown" => CoreKey::PageDown,
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => CoreKey::Char(c),
                _ => CoreKey::Other,
            }
        }
    };
    CoreKeyEvent::with_ctrl(code, ev.ctrl_key())
}

fn should_prevent_default(code: CoreKey) -> bool {
    matches!(
        code,
        CoreKey::Tab
            | CoreKey::Up
            | CoreKey::Down
            | CoreKey::Left
            | CoreKey::Right
            | CoreKey::PageUp
            | CoreKey::PageDown
            | CoreKey::Home
            | CoreKey::End
            | CoreKey::Char(' ')
    )
}

/// Common style for a floating overlay element (hidden until positioned).
fn init_overlay(el: &HtmlElement) -> Result<(), JsValue> {
    let style = el.style();
    style.set_property("position", "fixed")?;
    style.set_property("display", "none")?;
    style.set_property("z-index", "10")?;
    style.set_property("object-fit", "contain")?;
    style.set_property("background", "#000")?;
    style.set_property("pointer-events", "none")?;
    Ok(())
}

/// Place (or hide) the `<video>` / `<img>` overlays to match the cell rects the
/// UI recorded this frame. Cell→pixel scale comes from the root container's
/// on-screen size divided by the terminal's column/row count.
fn position_overlays(
    document: &Document,
    app: &App,
    video_el: &HtmlVideoElement,
    img_el: &HtmlImageElement,
    term: Rect,
) {
    let toolbar = document
        .get_element_by_id("toolbar")
        .and_then(|e| e.dyn_into::<HtmlElement>().ok());

    // Popups (escape menu / help) are drawn on the canvas above everything, so
    // hide the floating overlays while one is open.
    if app.show_escape_menu || app.show_help {
        let _ = video_el.style().set_property("display", "none");
        let _ = img_el.style().set_property("display", "none");
        if let Some(tb) = &toolbar {
            let _ = tb.style().set_property("display", "none");
        }
        return;
    }

    let root = document.get_element_by_id(ROOT_ID);
    let rect = root.as_ref().map(|r| r.get_bounding_client_rect());
    place(video_el, app.last_video_rect, term, rect.as_ref(), "block");
    place(img_el, app.last_art_rect, term, rect.as_ref(), "block");
    // The file-picker buttons live inside the (otherwise empty) browser panel,
    // inset by one cell so the panel's border stays visible.
    if let Some(tb) = &toolbar {
        place(tb, app.last_browser_rect.map(inset_one), term, rect.as_ref(), "flex");
    }
}

/// Shrink a cell rect by one cell on every side (to sit inside a panel border).
fn inset_one(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

fn place(
    el: &HtmlElement,
    cell: Option<Rect>,
    term: Rect,
    container: Option<&web_sys::DomRect>,
    display: &str,
) {
    let style = el.style();
    let (Some(cell), Some(container)) = (cell, container) else {
        let _ = style.set_property("display", "none");
        return;
    };
    if term.width == 0 || term.height == 0 || container.width() <= 0.0 || cell.width == 0 {
        let _ = style.set_property("display", "none");
        return;
    }
    // The canvas backend reports its size as (cols-1, rows-1) but draws the full
    // cols×rows grid, so the true column/row count is term + 1.
    let cw = container.width() / (term.width as f64 + 1.0);
    let ch = container.height() / (term.height as f64 + 1.0);
    let left = container.left() + cell.x as f64 * cw;
    let top = container.top() + cell.y as f64 * ch;
    let w = cell.width as f64 * cw;
    let h = cell.height as f64 * ch;
    let _ = style.set_property("left", &format!("{left}px"));
    let _ = style.set_property("top", &format!("{top}px"));
    let _ = style.set_property("width", &format!("{w}px"));
    let _ = style.set_property("height", &format!("{h}px"));
    let _ = style.set_property("display", display);
}

/// Hide the `HtmlElement` overlay (used when no rect was recorded this frame).
#[allow(dead_code)]
fn hide(el: &HtmlElement) {
    let _ = el.style().set_property("display", "none");
}

/// Fetch `manifest.json` and, if it lists tracks, load them into the playlist
/// and record each track's artwork/subtitle URLs into `extras`.
async fn load_manifest(app: SharedApp, extras: Extras) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let (tracks, extra_map) = match fetch_manifest(&window).await {
        Ok(t) => t,
        Err(_) => (Vec::new(), HashMap::new()),
    };
    if tracks.is_empty() {
        app.borrow_mut().status =
            String::from("No manifest — use the file picker to add local files");
        return;
    }
    *extras.borrow_mut() = extra_map;
    let mut a = app.borrow_mut();
    let n = tracks.len();
    a.tracks = tracks;
    a.selected = 0;
    a.playlist_state.select(Some(0));
    a.status = format!("Loaded {n} track(s) — press any key to start");
}

async fn fetch_manifest(window: &Window) -> Result<(Vec<Track>, HashMap<String, TrackExtra>), JsValue> {
    let resp_val = JsFuture::from(window.fetch_with_str(MANIFEST_URL)).await?;
    let resp: web_sys::Response = resp_val.dyn_into()?;
    if !resp.ok() {
        return Ok((Vec::new(), HashMap::new()));
    }
    let text = JsFuture::from(resp.text()?).await?;
    let text = text.as_string().unwrap_or_default();
    Ok(parse_manifest(&text))
}

/// Parse `{ "tracks": [ {"url", "title"?, "artwork"?, "subtitles"?} ] }`.
fn parse_manifest(text: &str) -> (Vec<Track>, HashMap<String, TrackExtra>) {
    let mut tracks = Vec::new();
    let mut extras = HashMap::new();
    let Ok(value) = js_sys::JSON::parse(text) else {
        return (tracks, extras);
    };
    let Ok(list) = js_sys::Reflect::get(&value, &JsValue::from_str("tracks")) else {
        return (tracks, extras);
    };
    if !list.is_array() {
        return (tracks, extras);
    }
    let str_field = |entry: &JsValue, key: &str| -> Option<String> {
        js_sys::Reflect::get(entry, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
    };
    for entry in js_sys::Array::from(&list).iter() {
        let Some(url) = str_field(&entry, "url") else {
            continue;
        };
        let title = str_field(&entry, "title").unwrap_or_else(|| url.clone());
        let extra = TrackExtra {
            artwork: str_field(&entry, "artwork"),
            subtitles: str_field(&entry, "subtitles"),
        };
        if extra.artwork.is_some() || extra.subtitles.is_some() {
            extras.insert(url.clone(), extra);
        }
        tracks.push(Track::from_url(url, title));
    }
    (tracks, extras)
}

/// Fetch a track's cover image and `.vtt` subtitles (if any) and apply them,
/// guarding against the track having changed while the fetch was in flight.
fn load_track_extras(app: SharedApp, url: String, extra: TrackExtra) {
    spawn_local(async move {
        if let Some(art_url) = extra.artwork {
            if let Some(bytes) = fetch_bytes(&art_url).await {
                let mut a = app.borrow_mut();
                if a.playing_track.as_ref().map(|t| t.locator()).as_deref() == Some(&url) {
                    let key = Some((bytes.len(), a.playing_index.unwrap_or(0)));
                    a.art.set_artwork(Some(&bytes), key);
                }
            }
        }
        if let Some(sub_url) = extra.subtitles {
            if let Some(text) = fetch_text(&sub_url).await {
                let cues = subtitles::parse_vtt(&text);
                if !cues.is_empty() {
                    let a = app.borrow();
                    if a.playing_track.as_ref().map(|t| t.locator()).as_deref() == Some(&url) {
                        a.subtitles.extend(vec![SubtitleTrack {
                            label: "Subtitles (web)".to_string(),
                            language: None,
                            cues,
                        }]);
                    }
                }
            }
        }
    });
}

async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let window = web_sys::window()?;
    let resp_val = JsFuture::from(window.fetch_with_str(url)).await.ok()?;
    let resp: web_sys::Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let buf = JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    let array = js_sys::Uint8Array::new(&buf);
    Some(array.to_vec())
}

async fn fetch_text(url: &str) -> Option<String> {
    let window = web_sys::window()?;
    let resp_val = JsFuture::from(window.fetch_with_str(url)).await.ok()?;
    let resp: web_sys::Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let text = JsFuture::from(resp.text().ok()?).await.ok()?;
    text.as_string()
}

fn is_media_name(name: &str) -> bool {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str());
    sparkplayer_core::library::is_audio_ext(ext)
}

/// Attach a change handler to a file/folder `<input>` that turns picked media
/// files into object-URL tracks (parsing each file's tags + cover art up front)
/// and starts playing the first one. Folder picks (`webkitdirectory`) yield all
/// descendant files, so non-media entries are filtered out.
fn wire_file_input(document: &Document, input_id: &str, app: SharedApp, meta_map: MetaMap) {
    let Some(input) = document
        .get_element_by_id(input_id)
        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };
    let input_for_handler = input.clone();
    let handler = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
        let Some(files) = input_for_handler.files() else {
            return;
        };
        let mut picked: Vec<web_sys::File> = Vec::new();
        for i in 0..files.length() {
            if let Some(f) = files.item(i) {
                if is_media_name(&f.name()) {
                    picked.push(f);
                }
            }
        }
        if picked.is_empty() {
            return;
        }
        picked.sort_by_key(|f| f.name());
        let app = app.clone();
        let meta_map = meta_map.clone();
        spawn_local(async move {
            let mut first_new: Option<usize> = None;
            for file in picked {
                let bytes = read_file_bytes(&file).await;
                let Ok(url) = Url::create_object_url_with_blob(&file) else {
                    continue;
                };
                if let Some(bytes) = bytes {
                    let meta = metadata_web::parse_metadata(&bytes);
                    meta_map.borrow_mut().insert(url.clone(), meta);
                }
                let mut a = app.borrow_mut();
                let idx = a.tracks.len();
                a.tracks.push(Track::from_url(url, file.name()));
                first_new.get_or_insert(idx);
            }
            if let Some(idx) = first_new {
                let mut a = app.borrow_mut();
                // A file pick is a user gesture; start playback of the first added.
                a.audio.on_user_gesture();
                let _ = a.play_index(idx);
            }
        });
    });
    let _ = input.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref());
    handler.forget();
}

async fn read_file_bytes(file: &web_sys::File) -> Option<Vec<u8>> {
    let buf = JsFuture::from(file.array_buffer()).await.ok()?;
    let array = js_sys::Uint8Array::new(&buf);
    Some(array.to_vec())
}
