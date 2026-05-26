# sparkplayer

A fun, no-nonsense terminal based media player using Ratatui

<img width="1501" height="932" alt="image" src="https://github.com/user-attachments/assets/6f7388cb-c2fb-4b06-8d11-93f82e916906" />

# Installation

SparkPlayer is written in Rust. Stable Rust 1.85 or newer is required (the
crate uses the 2024 edition).

System dependencies on Linux:

- ALSA development headers (`libasound2-dev` on Debian/Ubuntu,
  `alsa-lib` on Arch, `alsa-lib-devel` on Fedora) for audio output.
- A terminal capable of 24-bit color. Most modern terminals qualify
  (Alacritty, Kitty, WezTerm, foot, Ghostty, iTerm2, Windows Terminal, ...).
- Optional: a terminal that implements a graphics protocol (Kitty, Sixel,
  or iTerm2 inline images) to render embedded album art as real graphics
  instead of colored halfblocks.

Build from source:

```sh
git clone https://github.com/dividebysandwich/sparkplayer.git
cd sparkplayer
cargo build --release
```

The compiled binary lands in `target/release/sparkplayer`. Copy it
somewhere on your `PATH` (for example `~/.local/bin/`) or run it in place.

Or install directly from a checkout with cargo:

```sh
cargo install --path .
```

# Keyboard shortcuts

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
| `Enter`            | Play the highlighted item               |

Navigation

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `Up` / `Down`      | Move selection in the focused list      |
| `PgUp` / `PgDn`    | Page through the focused list           |
| `Home` / `End`     | Jump to first / last entry              |
| `Tab`              | Switch focus between playlist and browser |

Modes

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `v`                | Cycle visualizer: FFT bars, waveform, scrolling waveform, spectrogram, stereo X/Y, spectrum 3D, cassette tape |
| `f`                | Toggle fullscreen visualizer            |
| `r`                | Cycle repeat mode: Off, All, One        |
| `s`                | Shuffle the remaining tracks            |

Playlist

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `a`                | Queue the highlighted browser item (file, directory, or playlist) |
| `Shift+A`          | Queue every audio file under the currently browsed directory (recursive) |
| `Shift+C`          | Clear the playlist and stop playback    |

Other

| Key                | Action                                  |
|--------------------|-----------------------------------------|
| `?` or `h`         | Toggle the help overlay                 |
| `q`, `Esc`, `Ctrl+C` | Quit                                  |

# Usage

Run without arguments to open your operating system's music directory:

```sh
sparkplayer
```

A specific file, directory, or playlist can be passed as the only
positional argument.

```sh
sparkplayer ~/Music/album/        # play every audio file under the directory, recursively
sparkplayer song.flac             # play a single file
sparkplayer playlist.m3u          # load an M3U / M3U8 playlist
sparkplayer playlist.pls          # load a PLS playlist
```

Supported audio formats: `mp3`, `wav`, `ogg`, `flac`, `m4a`, `aac`,
`opus`, `wma`. Supported playlist formats: `m3u`, `m3u8`, `pls`.

The browser pane on the left lets you navigate the filesystem; pressing
`Enter` on a directory descends into it, on a playlist file replaces the
current playlist, and on an audio file appends it to the playlist and
starts playing it.

Album art is read from embedded tags (ID3 APIC, MP4 `covr`, FLAC PICTURE,
Vorbis METADATA_BLOCK_PICTURE) and, if none is embedded, from a sidecar
file in the same directory named `cover`, `folder`, `front`, `albumart`,
`album`, or `artwork` with a `.jpg`, `.jpeg`, `.png`, or `.webp`
extension.

Command-line options:

```
--autoplay              Auto-start playback once the playlist is loaded (default: on)
--graphics <PROTOCOL>   Override the album-art graphics protocol.
                        One of: auto, halfblocks, sixel, kitty, iterm.
                        Default: auto.
```

Use `--graphics` to force a specific renderer when terminal auto-detection
misses. On terminals such as Alacritty, only halfblocks will render — the terminal does not
implement Sixel, Kitty, or iTerm2 inline images.

# Motivation

I am a huge fan of VLC, which is a fantastic piece of software. However for over a decaded now I am irritated by the fact that VLC always seems to alter the pitch of music during playback, especially shortly after pressing play, after scrolling, or skipping to the next song. I have turned off any re-sampling feature in the settings but the issue persists. 

After looking through a bunch of terminal based players, here's SparkPlayer. It's lightweight, runs in the terminal, defaults to your music directory and does exactly what I want it to do. I still love VLC for video playback of course.
