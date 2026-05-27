mod app;
mod audio;
mod config;
mod library;
mod metadata;
mod subtitles;
mod theme;
mod ui;
mod video;
mod visualizer;

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

use crate::app::{App, AV_OFFSET_STEP_SECS, GraphicsChoice};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum GraphicsArg {
    /// Auto-detect via terminal queries (Kitty / iTerm / Sixel / Halfblocks)
    Auto,
    /// Force colored halfblocks — works on every truecolor terminal (incl. Alacritty)
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

    /// Override the album-art graphics protocol. Alacritty has no graphics
    /// protocol support, so it always falls back to halfblocks.
    #[arg(long, value_enum, default_value_t = GraphicsArg::Auto)]
    graphics: GraphicsArg,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let target_path = cli.path.clone().unwrap_or_else(library::default_music_dir);

    let tracks = library::load_tracks(&target_path).unwrap_or_default();
    let initial_dir = if target_path.is_dir() {
        target_path.clone()
    } else {
        target_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(library::default_music_dir)
    };

    let cfg = config::load();
    let mut app = App::new(tracks, initial_dir, cli.graphics.into(), &cfg)?;

    let mut terminal = setup_terminal().context("setting up terminal")?;
    // Picker queries the terminal — must happen after raw mode is enabled
    // so escape responses come through stdin without echoing as characters.
    app.init_graphics();

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

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    let frame_dur = Duration::from_millis(33);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        let timeout = frame_dur
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Release {
                    handle_key(app, key.code, key.modifiers)?;
                }
            }
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

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Result<()> {
    if app.show_help {
        match code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('q') => {
                app.show_help = false;
            }
            _ => {}
        }
        return Ok(());
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
        KeyCode::Char(' ') => app.player.toggle_pause(),
        KeyCode::Char('n') => app.next_track()?,
        KeyCode::Char('p') => app.prev_track()?,
        KeyCode::Char('v') => app.cycle_visualizer(),
        KeyCode::Char('t') => app.cycle_theme(),
        KeyCode::Char('f') => app.fullscreen_vis = !app.fullscreen_vis,
        KeyCode::Char('r') => app.cycle_repeat(),
        KeyCode::Char('s') => app.toggle_shuffle(),
        KeyCode::Char('a') => app.queue_selected_browser(),
        KeyCode::Char('A') => app.queue_browser_directory(),
        KeyCode::Char('C') => app.clear_playlist(),
        KeyCode::Char('c') => app.cycle_subtitle_track(),
        KeyCode::Char('?') | KeyCode::Char('h') => app.show_help = true,
        KeyCode::Tab => app.focus_next(),
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::PageUp => app.page(-1),
        KeyCode::PageDown => app.page(1),
        KeyCode::Home => app.select_first(),
        KeyCode::End => app.select_last(),
        KeyCode::Left if mods.contains(KeyModifiers::CONTROL) => app.seek_seconds(-30.0),
        KeyCode::Right if mods.contains(KeyModifiers::CONTROL) => app.seek_seconds(30.0),
        KeyCode::Left => app.seek_seconds(-10.0),
        KeyCode::Right => app.seek_seconds(10.0),
        KeyCode::Char('-') | KeyCode::Char('_') => app.volume_step(-0.05),
        KeyCode::Char('+') | KeyCode::Char('=') => app.volume_step(0.05),
        KeyCode::Char('[') => app.adjust_av_offset(-AV_OFFSET_STEP_SECS),
        KeyCode::Char(']') => app.adjust_av_offset(AV_OFFSET_STEP_SECS),
        KeyCode::Enter => app.activate_selection()?,
        _ => {}
    }
    Ok(())
}
