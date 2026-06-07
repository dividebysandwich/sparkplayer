//! Terminal UI rendering. [`draw`] is the per-frame entry point that lays out
//! the screen and dispatches to the panel, visualizer, video and overlay
//! submodules. Image content (video frames, album art) is drawn through the
//! `VideoBackend`/`AlbumArtRenderer` trait objects: the native build paints it
//! into the terminal via ratatui-image, while the web build records the target
//! rectangle (`App::last_video_rect`/`last_art_rect`) so it can float real
//! `<video>`/`<img>` elements over the canvas.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app::App;

mod cassette;
mod overlays;
mod palette;
mod panels;
mod video_panel;
mod visualizers;

use overlays::{draw_escape_menu, draw_help};
use panels::{draw_album_art, draw_browser, draw_footer, draw_now_playing, draw_playlist};
use video_panel::draw_video;
use visualizers::draw_visualizer;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Reset per-frame overlay rects; the panel renderers set them when (and
    // where) image content is shown so the web build can position overlays.
    app.last_video_rect = None;
    app.last_art_rect = None;
    app.last_browser_rect = None;

    let has_video = app.video.has_image();

    if app.fullscreen_vis {
        // Give the entire screen to the video/visualizer — no footer.
        if has_video {
            draw_video(frame, area, app);
        } else {
            draw_visualizer(frame, area, app);
        }
        if app.show_help {
            draw_help(frame, area, app);
        }
        if app.show_escape_menu {
            draw_escape_menu(frame, area, app);
        }
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(outer[0]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[0]);

    draw_playlist(frame, left[0], app);
    draw_browser(frame, left[1], app);

    if has_video {
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(16)])
            .split(body[1]);
        draw_video(frame, right[0], app);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(right[1]);
        draw_now_playing(frame, bottom[0], app);
        draw_visualizer(frame, bottom[1], app);
    } else {
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(16), Constraint::Min(8)])
            .split(body[1]);

        let has_art = app.art.has_art();
        if has_art {
            let top_row = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
                .split(right[0]);
            draw_album_art(frame, top_row[0], app);
            draw_now_playing(frame, top_row[1], app);
        } else {
            draw_now_playing(frame, right[0], app);
        }
        draw_visualizer(frame, right[1], app);
    }

    draw_footer(frame, outer[1], app);

    if app.show_help {
        draw_help(frame, area, app);
    }
    if app.show_escape_menu {
        draw_escape_menu(frame, area, app);
    }
}
