mod audio;
mod backends;
mod external_window;
mod library_native;
mod metadata_native;
mod subtitles_native;
mod video;

use std::io::{self, Stdout};
use std::path::PathBuf;
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
use sparkplayer_core::{App, ui};

use crate::audio::AudioPlayer;
use crate::backends::{
    build_picker, GraphicsChoice, NativeAlbumArt, NativeConfigStore, NativeVideoBackend,
};
use crate::library_native::NativeLibrary;

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
        _ => CoreKey::Other,
    };
    CoreKeyEvent::with_ctrl(core, mods.contains(KeyModifiers::CONTROL))
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let target_path = cli
        .path
        .clone()
        .unwrap_or_else(library_native::default_music_dir);

    let tracks = library_native::load_tracks(&target_path).unwrap_or_default();
    let initial_dir = if target_path.is_dir() {
        target_path.clone()
    } else {
        target_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(library_native::default_music_dir)
    };

    let config = NativeConfigStore;
    let cfg = config.load();

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
        let _ = app.play_index(0);
    }

    let res = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal).ok();
    res
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

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    let frame_dur = Duration::from_millis(33);
    let start = Instant::now();
    let mut last_tick = Instant::now();

    loop {
        app.set_clock(start.elapsed().as_secs_f64());
        terminal.draw(|f| ui::draw(f, app))?;

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

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
