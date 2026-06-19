mod audio;
mod backends;
mod external_window;
mod library_native;
mod media_controls;
mod metadata_native;
mod subtitles_native;
mod video;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use sparkplayer_core::backend::{ConfigStore, CoreKey, CoreKeyEvent};
use sparkplayer_core::library::Track;
use sparkplayer_core::{App, ui};

use crate::audio::AudioPlayer;
use crate::backends::{
    build_picker, GraphicsChoice, NativeAlbumArt, NativeConfigStore, NativeVideoBackend,
};
use crate::library_native::NativeLibrary;
use crate::media_controls::{MediaCommand, MediaOs};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum GraphicsArg {
    /// Auto-detect via terminal queries (Kitty / iTerm / Sixel / Halfblocks)
    Auto,
    /// Force colored halfblocks — works on every truecolor terminal
    Halfblocks,
    /// Force the Sixel protocol (xterm, foot, wezterm, mlterm)
    Sixel,
    /// Force the Kitty graphics protocol (kitty, ghostty, wezterm)
    Kitty,
    /// Force the iTerm2 inline-images protocol (iTerm2, WezTerm)
    Iterm,
}

impl From<GraphicsArg> for GraphicsChoice {
    fn from(v: GraphicsArg) -> Self {
        match v {
            GraphicsArg::Auto => GraphicsChoice::Auto,
            GraphicsArg::Halfblocks => GraphicsChoice::Halfblocks,
            GraphicsArg::Sixel => GraphicsChoice::Sixel,
            GraphicsArg::Kitty => GraphicsChoice::Kitty,
            GraphicsArg::Iterm => GraphicsChoice::Iterm,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "sparkplayer",
    version,
    about = "A vibrant terminal music player powered by ratatui"
)]
struct Cli {
    /// File, directory, or playlist (.m3u/.m3u8/.pls) to play. Defaults to your music directory.
    path: Option<PathBuf>,

    /// Auto-start playback once the playlist is loaded.
    #[arg(long, default_value_t = true)]
    autoplay: bool,

    /// Override the album-art graphics protocol.
    #[arg(long, value_enum, default_value_t = GraphicsArg::Auto)]
    graphics: GraphicsArg,

    /// Open the dedicated SDL fullscreen window for video playback at startup.
    #[arg(long)]
    video_window: bool,

    /// Preferred subtitle language (ISO code like `eng`/`en` or a label like
    /// `English`).
    #[arg(long, value_name = "LANG")]
    subtitle_lang: Option<String>,
}

/// Translate a crossterm key into the platform-neutral [`CoreKeyEvent`] the
/// core dispatcher understands. Public so the external-window backend can map
/// keys it forwards from SDL through the same path.
pub fn map_key(code: KeyCode, mods: KeyModifiers) -> CoreKeyEvent {
    let core = match code {
        KeyCode::Char(c) => CoreKey::Char(c),
        KeyCode::Up => CoreKey::Up,
        KeyCode::Down => CoreKey::Down,
        KeyCode::Left => CoreKey::Left,
        KeyCode::Right => CoreKey::Right,
        KeyCode::PageUp => CoreKey::PageUp,
        KeyCode::PageDown => CoreKey::PageDown,
        KeyCode::Home => CoreKey::Home,
        KeyCode::End => CoreKey::End,
        KeyCode::Tab => CoreKey::Tab,
        KeyCode::Enter => CoreKey::Enter,
        KeyCode::Esc => CoreKey::Esc,
        KeyCode::Backspace => CoreKey::Backspace,
        KeyCode::Delete => CoreKey::Delete,
        _ => CoreKey::Other,
    };
    CoreKeyEvent::with_ctrl(core, mods.contains(KeyModifiers::CONTROL))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = NativeConfigStore;
    let cfg = config.load();

    // With an explicit path, behave as before. Without one, resume the last
    // session: restore the saved playlist and browser directory.
    let resuming = cli.path.is_none();
    let (tracks, initial_dir) = if let Some(path) = cli.path.clone() {
        let tracks = library_native::load_tracks(&path).unwrap_or_default();
        let dir = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(library_native::default_music_dir)
        };
        (tracks, dir)
    } else {
        let tracks: Vec<_> = cfg
            .playlist
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map(Track::from_path)
            .collect();
        let dir = cfg
            .last_dir
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(library_native::default_music_dir);
        (tracks, dir)
    };

    let mut terminal = setup_terminal().context("setting up terminal")?;
    // Picker queries the terminal — must happen after raw mode is enabled so
    // escape responses come through stdin without echoing as characters.
    let picker = build_picker(cli.graphics.into());

    let audio = AudioPlayer::new()?;
    let video = NativeVideoBackend::new(picker.clone());
    let art = NativeAlbumArt::new(picker);

    let mut app = App::new(
        Box::new(audio),
        Box::new(video),
        Box::new(NativeLibrary),
        Box::new(config),
        Box::new(art),
        tracks,
        initial_dir,
        &cfg,
    );
    app.preferred_subtitle_lang = cli.subtitle_lang.clone();
    if cli.video_window {
        app.video.set_external_window(true);
    }

    if cli.autoplay && !app.tracks.is_empty() {
        // When resuming, restart the track that was playing and seek back to
        // where it left off; otherwise start at the top.
        let start_idx = if resuming {
            cfg.playing_index.filter(|&i| i < app.tracks.len())
        } else {
            None
        };
        match start_idx {
            Some(idx) => {
                if app.play_index(idx).is_ok() && cfg.position_secs > 1.0 {
                    app.seek_to_secs(cfg.position_secs);
                }
            }
            None => {
                let _ = app.play_index(0);
            }
        }
    }

    // Build the whole-library search index in the background so startup stays
    // instant; the result is drained into the app once it lands.
    let index_root = library_native::default_music_dir();
    let (idx_tx, idx_rx) = mpsc::channel::<Vec<Track>>();
    std::thread::spawn(move || {
        let _ = idx_tx.send(library_native::scan_directory(&index_root));
    });

    // Best-effort OS media integration (MPRIS / Now Playing). Absent D-Bus etc.
    // it just stays off.
    let mut media = MediaOs::new().ok();

    let res = run_loop(&mut terminal, &mut app, idx_rx, media.as_mut());
    restore_terminal(&mut terminal).ok();
    app.save_session();
    res
}

/// Translate an OS media-control request into an app action.
fn apply_media(cmd: MediaCommand, app: &mut App) -> Result<()> {
    match cmd {
        MediaCommand::Toggle => app.audio.toggle_pause(),
        MediaCommand::Play => {
            if app.audio.is_paused() {
                app.audio.toggle_pause();
            }
        }
        MediaCommand::Pause => {
            if !app.audio.is_paused() {
                app.audio.toggle_pause();
            }
        }
        MediaCommand::Next => app.next_track()?,
        MediaCommand::Prev => app.prev_track()?,
        MediaCommand::Stop => app.audio.stop(),
        MediaCommand::SeekForward => app.seek_seconds(10.0),
        MediaCommand::SeekBack => app.seek_seconds(-10.0),
        MediaCommand::SetPosition(pos) => app.seek_to_secs(pos.as_secs_f64()),
        MediaCommand::Quit => app.should_quit = true,
    }
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    index_rx: Receiver<Vec<Track>>,
    mut media: Option<&mut MediaOs>,
) -> Result<()> {
    let frame_dur = Duration::from_millis(33);
    let start = Instant::now();
    let mut last_tick = Instant::now();

    loop {
        app.set_clock(start.elapsed().as_secs_f64());
        terminal.draw(|f| ui::draw(f, app))?;

        // Adopt the library index once the background scan reports in.
        if let Ok(tracks) = index_rx.try_recv() {
            app.set_search_index(tracks);
        }

        // Apply any OS media-control requests (media keys, desktop widget).
        if let Some(m) = media.as_deref_mut() {
            for cmd in m.poll() {
                apply_media(cmd, app)?;
            }
        }

        let timeout = frame_dur
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Release {
                    app.handle_key(map_key(key.code, key.modifiers))?;
                }
            }
        }

        // Forward keystrokes captured by the SDL playback window.
        for ev in app.drain_external_keys() {
            app.handle_key(ev)?;
        }

        if last_tick.elapsed() >= frame_dur {
            app.check_advance()?;
            app.tick_video();
            last_tick = Instant::now();
        }

        // Publish current metadata + playback state to the OS.
        if let Some(m) = media.as_deref_mut() {
            m.sync(app);
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
