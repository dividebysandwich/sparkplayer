# sparkplayer

A fun, no-nonsense terminal media player built with Ratatui.

<img width="1501" height="932" alt="image" src="https://github.com/user-attachments/assets/6f7388cb-c2fb-4b06-8d11-93f82e916906" />

<img width="992" height="1098" alt="demo" src="https://github.com/user-attachments/assets/b2d4c0fd-9398-4c25-a6bc-c0ef3ee0758a" />

<img width="1992" height="1298" alt="image" src="https://github.com/user-attachments/assets/64343b32-d799-44da-b27c-15384851166d" />

## Overview

SparkPlayer is a media player for every day use that's fun to use and looks nice. Point it
at your on-disk media library and it just works: it opens to your music or video
directory, browses your folders, plays virtually every media  format you own, and
keeps your playlist a single keypress away. No setup ceremony, no library
import, no fuss.

Enjoy eleven colorful audio visualizers (FFT bars, mirrored
bars, a radial spectrum, waveforms, spectrograms, a stereo X/Y scope, VU meters,
a 3D spectrum, an audio-reactive plasma, even a spinning cassette tape),
embedded album art rendered as real graphics, and video
playback right in the terminal. On terminals with a graphics protocol you get
true images; everywhere else SparkPlayer falls back to 24-bit-color halfblocks
so it still works, anywhere.

Highlights:

- **Easy to use** — opens to your music directory, browse-and-play, helpful
  help overlay (`?`), sensible keyboard shortcuts.
- **Great daily driver** — lightweight, fast, and built to be your main player
  for a local music library. Filter long lists with `/`, edit and save your
  playlist, and it picks up where you left off next time you open it.
- **Lots of formats** — common audio, video, and playlist formats all
  supported out of the box.
- **Graphics in the terminal** — album art and video as real images on capable
  terminals, with a colored-halfblock fallback for everything else.
- **Fun** — a pile of visualizers to flip through while you listen.
- **Runs in the browser too** — there's a WASM build (see below).

## Supported formats

- **Audio:** `mp3`, `wav`, `ogg`, `flac`, `m4a`, `aac`, `opus`, `wma`
- **Video:** `mp4`, `mkv`, `avi`, `mov`, `webm`, `m4v`
- **Playlists:** `m3u`, `m3u8`, `pls`

## Usage

Run without arguments to open your operating system's music directory:

```sh
sparkplayer
```

A specific file, directory, or playlist can be passed as the only positional
argument:

```sh
sparkplayer ~/Music/album/        # play every audio file under the directory, recursively
sparkplayer song.flac             # play a single file
sparkplayer playlist.m3u          # load an M3U / M3U8 playlist
sparkplayer playlist.pls          # load a PLS playlist
```

The browser pane on the left lets you navigate the filesystem; pressing `Enter`
on a directory descends into it, on a playlist file replaces the current
playlist, and on an audio file appends it to the playlist and starts playing it.

Press `/` to filter the focused list — handy for finding a track or folder in a
large library; `Esc` clears the filter. Curate the playlist in place with `d`
(remove) and `Ctrl+Up`/`Ctrl+Down` (reorder), and write it out with `w`.

### Sessions

Launched with no arguments, SparkPlayer reopens where you left off: the same
browser directory and playlist, the track and position that were playing, and
your repeat/shuffle modes. Pass a path explicitly to start fresh from it
instead. (In the browser build, settings and repeat/shuffle persist via
`localStorage`; locally-picked files can't be restored across reloads.)

### Album art

Album art is read from embedded tags (ID3 APIC, MP4 `covr`, FLAC PICTURE,
Vorbis METADATA_BLOCK_PICTURE) and, if none is embedded, from a sidecar file in
the same directory named `cover`, `folder`, `front`, `albumart`, `album`, or
`artwork` with a `.jpg`, `.jpeg`, `.png`, or `.webp` extension.

### Video playback

Video files are decoded with FFmpeg on a background thread and rendered in the
album-art pane in place of cover artwork. Frames are pulled in PTS order and
sampled against the audio clock, so picture stays synchronised with the
soundtrack. Seeking, pausing, and skipping work the same as for audio-only
tracks.

Picture quality depends on the terminal's graphics protocol:

- Kitty, WezTerm, Ghostty, or iTerm2 render true graphics — choose one of these
  for the best result.
- Sixel-capable terminals (foot, mlterm, xterm with `--enable-sixel`) also
  render real images.
- Other terminals fall back to 24-bit-color halfblocks, which is watchable but
  coarse.

If audio and video drift apart, use `[` / `]` to nudge the A/V offset in 25 ms
steps (clamped between −500 ms and +2000 ms). Toggling fullscreen with `f`
scales the video to fill the whole window.

When a video carries more than one audio track (e.g. different languages or a
commentary), press `b` to cycle between them — the picture keeps playing in
place. Subtitle tracks cycle the same way with `c`. Both are also available from
the `Esc` menu, where ‹ › adjust the selection.

### Command-line options

```
--autoplay              Auto-start playback once the playlist is loaded (default: on)
--graphics <PROTOCOL>   Override the album-art graphics protocol.
                        One of: auto, halfblocks, sixel, kitty, iterm.
                        Default: auto.
```

Use `--graphics` to force a specific renderer when terminal auto-detection
misses. On terminals such as Alacritty, only halfblocks will render — the
terminal does not implement Sixel, Kitty, or iTerm2 inline images.

## Keyboard shortcuts

Playback

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `Space`            | Play / pause                            |
| `n`                | Next track                              |
| `p`                | Previous track                          |
| `Left` / `Right`   | Seek backward / forward by 10 seconds   |
| `Ctrl+Left` / `Ctrl+Right` | Seek backward / forward by 30 seconds |
| `+` / `=`          | Volume up (5%)                          |
| `-`                | Volume down (5%)                        |
| `[` / `]`          | Nudge A/V sync offset by 25 ms (video)  |
| `b`                | Cycle audio track (video)               |
| `c`                | Cycle subtitle track (video)            |
| `Enter`            | Play the highlighted item               |

Navigation

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `Up` / `Down`      | Move selection in the focused list      |
| `PgUp` / `PgDn`    | Page through the focused list           |
| `Home` / `End`     | Jump to first / last entry              |
| `Tab`              | Switch focus between playlist and browser |
| `/`                | Filter the focused list (type to narrow, `Esc` clears) |

Modes

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `v`                | Cycle visualizer: FFT bars, mirror bars, radial, waveform, scrolling waveform, spectrogram, stereo X/Y, VU meters, spectrum 3D, plasma, cassette tape |
| `f`                | Toggle fullscreen visualizer            |
| `r`                | Cycle repeat mode: Off, All, One        |
| `s`                | Shuffle the remaining tracks            |

Playlist

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `a`                | Queue the highlighted browser item (file, directory, or playlist) |
| `Shift+A`          | Queue every audio file under the currently browsed directory (recursive) |
| `d` / `Delete`     | Remove the highlighted track from the playlist |
| `Ctrl+Up` / `Ctrl+Down` | Move the highlighted track up / down |
| `w`                | Save the playlist to an `.m3u` file in the browsed directory |
| `Shift+C`          | Clear the playlist and stop playback    |

Other

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `?` or `h`         | Toggle the help overlay                 |
| `q`, `Esc`, `Ctrl+C` | Quit                                  |

# Installation

SparkPlayer is written in Rust. Stable Rust 1.85 or newer is required (the
crate uses the 2024 edition).

System dependencies on Linux:

- ALSA development headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib` on
  Arch, `alsa-lib-devel` on Fedora) for audio output.
- FFmpeg development libraries (`libavcodec-dev`, `libavformat-dev`,
  `libavutil-dev`, `libswscale-dev`, `libswresample-dev` on Debian/Ubuntu,
  `ffmpeg` on Arch, `ffmpeg-devel` on Fedora) for video decoding.
- A terminal capable of 24-bit color. Most modern terminals qualify
  (Alacritty, Kitty, WezTerm, foot, Ghostty, iTerm2, Windows Terminal, ...).
- Optional: a terminal that implements a graphics protocol (Kitty, Sixel, or
  iTerm2 inline images) to render embedded album art and video as real graphics
  instead of colored halfblocks.

SparkPlayer is a Cargo workspace:

- `crates/sparkplayer-core` — platform-agnostic core (UI, visualizers, app
  state, keymap, and the backend traits), shared by both builds.
- `crates/sparkplayer-native` — the terminal player (this binary).
- `crates/sparkplayer-web` — the browser/WASM build (see below).

Build the terminal player from source:

```sh
git clone https://github.com/dividebysandwich/sparkplayer.git
cd sparkplayer
cargo build --release
```

The compiled binary lands in `target/release/sparkplayer`. Copy it somewhere on
your `PATH` (for example `~/.local/bin/`) or run it in place.

Or install directly from a checkout with cargo:

```sh
cargo install --path crates/sparkplayer-native
```

### Building from source on Windows

Unlike Linux, Windows has no system package manager that ships FFmpeg/SDL2
development libraries, so `ffmpeg-sys-next` can't find them on its own. If you
just `cargo build` you'll hit:

```
Could not find ffmpeg with vcpkg: Could not find Vcpkg root ...
The pkg-config command could not be found.
```

You don't need vcpkg or pkg-config — you just need to point the build at
prebuilt libraries with a few environment variables (this is exactly what CI
does in `.github/workflows/release.yml`). One-time setup:

1. **FFmpeg shared dev build** — download
   [`ffmpeg-8.1.1-full_build-shared.7z`](https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.1-full_build-shared.7z)
   from gyan.dev (LGPL) and extract it somewhere, e.g.
   `C:\dev\ffmpeg`. The extracted folder must contain `include\`, `lib\` and
   `bin\`. Pin this exact version — the bundled DLL names are
   version-specific.
2. **LLVM/Clang** — `ffmpeg-sys-next` uses `bindgen`, which needs `libclang`.
   Install LLVM (`winget install LLVM.LLVM`, or grab it from
   <https://releases.llvm.org/>); it lands in `C:\Program Files\LLVM`.
3. **SDL2 VC dev libraries** — download
   [`SDL2-devel-2.32.4-VC.zip`](https://github.com/libsdl-org/SDL/releases/download/release-2.32.4/SDL2-devel-2.32.4-VC.zip)
   and extract it, e.g. `C:\dev\SDL2`. The linker only needs `SDL2.lib` from
   `lib\x64`.

Then set the environment variables and build (PowerShell). Adjust the paths to
where you extracted each archive:

```powershell
$env:FFMPEG_DIR   = "C:\dev\ffmpeg\ffmpeg-8.1.1-full_build-shared"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
$env:LIB          = "C:\dev\SDL2\SDL2-2.32.4\lib\x64;$env:LIB"

cargo build --release
```

`ffmpeg-sys-next` reads `FFMPEG_DIR\include` and `FFMPEG_DIR\lib` directly, so
no pkg-config/vcpkg is involved.

To **run** the result, the FFmpeg DLLs (`avcodec-*.dll`, `avformat-*.dll`,
`avutil-*.dll`, `swresample-*.dll`, `swscale-*.dll` from `FFMPEG_DIR\bin`) and
`SDL2.dll` (from the SDL2 `lib\x64` folder) must sit next to
`sparkplayer.exe` or be on your `PATH` — otherwise the program exits
immediately at startup. (The released `.zip` and `.msi` bundle these for you;
this only applies to your own source builds.)

> Using an IDE / rust-analyzer? The variables above are per-shell. So the
> editor's background `cargo check` sees them, set them as persistent user
> environment variables (`setx FFMPEG_DIR "..."`, etc., or via *System
> Properties → Environment Variables*) and **restart the IDE**, or add them to
> rust-analyzer's `cargo.extraEnv` setting.

### Building from source on macOS

macOS has no FFmpeg/SDL2 in the base system, but [Homebrew](https://brew.sh/)
provides both. On Apple Silicon they install under `/opt/homebrew`:

```sh
brew install ffmpeg sdl2 pkg-config
cargo build --release
```

`ffmpeg-sys-next` locates FFmpeg through pkg-config and `sdl2-sys` links a bare
`-lSDL2`, so both resolve from the Homebrew prefix automatically (Cargo's build
already searches `/opt/homebrew/lib`; if a fresh shell can't find them, set
`PKG_CONFIG_PATH=/opt/homebrew/lib/pkgconfig` and `LIBRARY_PATH=/opt/homebrew/lib`).

A source build links against the Homebrew dylibs, so it runs only while those
remain installed. The released `macos-aarch64` `.zip` bundles the FFmpeg/SDL2
dylibs next to the binary (in `libs/`, re-signed ad-hoc), so it runs without
Homebrew.

## Browser build (WASM)

SparkPlayer also runs in the browser, rendered with
[Ratzilla](https://github.com/ratatui/ratzilla) on a canvas (fixed-cell grid),
using a bundled [Meslo Nerd Font Mono](https://github.com/ryanoasis/nerd-fonts)
webfont. Audio plays through the Web Audio API (with the visualizer tapping the
analyser), and video is shown via a real `<video>` element floated over the
terminal grid. No FFmpeg/SDL/ALSA — the browser does the decoding.

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk            # one-time
cd crates/sparkplayer-web
trunk serve                    # dev server at http://localhost:8080
# or: trunk build --release    # static bundle in crates/sparkplayer-web/dist/
```

It chooses its media source at startup from `manifest.json` (served next to the
page):

- **Web-playlist mode** — if `manifest.json` lists `tracks`, they are loaded and
  played. Each entry takes a `url` (required) plus optional `title`, `artwork`
  (cover image URL) and `subtitles` (a `.vtt` URL):
  `{ "tracks": [ { "url": "https://host/song.mp3", "title": "Song", "artwork": "https://host/cover.jpg", "subtitles": "https://host/song.en.vtt" } ] }`.
  Hosted media must send permissive CORS headers (the player loads media with
  `crossorigin="anonymous"`), otherwise the visualizer's analyser reads silence.
- **Local-file mode** — if `tracks` is empty, use the file picker at the top of
  the page to play files from your PC. Picked files have their tags and embedded
  cover art parsed in-browser (via `lofty`), so title/artist/album and album art
  show just like the native build.

Browsers require a user gesture before audio can start, so the first keypress (or
file pick) begins playback.

# Motivation

I am a huge fan of VLC, which is a fantastic piece of software. However for over
a decade now I am irritated by the fact that VLC always seems to alter the pitch
of music during playback, especially shortly after pressing play, after
scrolling, or skipping to the next song. I have turned off any re-sampling
feature in the settings but the issue persists.

After looking through a bunch of terminal based players, here's SparkPlayer.
It's lightweight, runs in the terminal, defaults to your music directory and
does exactly what I want it to do. Video playback is supported too, for when a
music video shows up next to the album.
