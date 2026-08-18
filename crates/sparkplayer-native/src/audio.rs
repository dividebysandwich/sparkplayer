use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::format::sample::{Sample, Type as SampleType};
use ffmpeg::media::Type as MediaType;
use ffmpeg::software::resampling::Context as Resampler;
use ffmpeg::util::frame::audio::Audio;
use ffmpeg::ChannelLayout;
use rodio::source::{Source, UniformSourceIterator};
use rodio::{ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate};

use sparkplayer_core::backend::{AudioBackend, StartedTrack};
use sparkplayer_core::library;
use sparkplayer_core::{SampleBuffer, TrackRef};

/// Audio source backed by an ffmpeg input. Used when playing video files
/// (and also as a generic fallback for audio formats rodio's symphonia layer
/// doesn't accept). Pulls and demuxes lazily so the decode work happens on
/// rodio's playback thread.
pub struct FfmpegAudioSource {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::codec::decoder::Audio,
    resampler: Resampler,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    out_channels: u16,
    out_rate: u32,
    duration: Option<Duration>,
    buffer: VecDeque<f32>,
    finished: bool,
    /// Set on seek. The next decoded frame at-or-after this PTS becomes the
    /// first sample we emit; earlier ones (from keyframe-aligned demux seek)
    /// are dropped so the tap's base_offset corresponds to the actual audio.
    pending_seek_secs: Option<f64>,
}

enum FrameDisposition {
    DropAll,
    Keep { skip_interleaved: usize },
}

impl FfmpegAudioSource {
    /// Open the best audio stream of `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let ictx = open_input(path)?;
        let stream_index = ictx
            .streams()
            .best(MediaType::Audio)
            .context("file has no audio stream")?
            .index();
        Self::from_input(ictx, stream_index)
    }

    /// Open a specific audio stream of `path` (used to switch between the
    /// multiple audio tracks a video container may carry).
    pub fn open_stream(path: &Path, stream_index: usize) -> Result<Self> {
        let ictx = open_input(path)?;
        Self::from_input(ictx, stream_index)
    }

    fn from_input(ictx: ffmpeg::format::context::Input, stream_index: usize) -> Result<Self> {
        let stream = ictx
            .stream(stream_index)
            .context("audio stream index out of range")?;
        let time_base = stream.time_base();
        let duration = {
            let dur = stream.duration();
            if dur > 0 {
                Some(Duration::from_secs_f64(
                    dur as f64 * time_base.numerator() as f64 / time_base.denominator() as f64,
                ))
            } else {
                let d = ictx.duration();
                if d > 0 {
                    Some(Duration::from_secs_f64(
                        d as f64 / ffmpeg::ffi::AV_TIME_BASE as f64,
                    ))
                } else {
                    None
                }
            }
        };

        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = codec_ctx.decoder().audio()?;

        let in_rate = decoder.rate();
        let in_channels = decoder.channels();
        let in_layout = if decoder.channel_layout() == ChannelLayout::default(0) {
            ChannelLayout::default(in_channels as i32)
        } else {
            decoder.channel_layout()
        };
        let in_format = decoder.format();

        let out_rate: u32 = if in_rate == 0 { 44_100 } else { in_rate };
        let out_layout = ChannelLayout::STEREO;
        let out_channels: u16 = 2;

        let resampler = Resampler::get(
            in_format,
            in_layout,
            in_rate.max(1),
            Sample::F32(SampleType::Packed),
            out_layout,
            out_rate,
        )
        .context("creating audio resampler")?;

        Ok(Self {
            ictx,
            decoder,
            resampler,
            stream_index,
            stream_time_base: time_base,
            out_channels,
            out_rate,
            duration,
            buffer: VecDeque::with_capacity(8192),
            finished: false,
            pending_seek_secs: None,
        })
    }

    /// Seek the underlying input to `target` and reset decoder state.
    pub fn seek(&mut self, target: Duration) -> Result<()> {
        let ts = (target.as_micros() as i64) * (ffmpeg::ffi::AV_TIME_BASE as i64) / 1_000_000;
        self.ictx.seek(ts, ..ts).ok();
        self.decoder.flush();
        self.buffer.clear();
        self.finished = false;
        self.pending_seek_secs = Some(target.as_secs_f64());
        Ok(())
    }

    fn frame_disposition(&mut self, frame: &Audio) -> FrameDisposition {
        let Some(target) = self.pending_seek_secs else {
            return FrameDisposition::Keep { skip_interleaved: 0 };
        };
        let Some(pts) = frame.pts() else {
            self.pending_seek_secs = None;
            return FrameDisposition::Keep { skip_interleaved: 0 };
        };
        let tb_num = self.stream_time_base.numerator() as f64;
        let tb_den = self.stream_time_base.denominator() as f64;
        if tb_den == 0.0 {
            self.pending_seek_secs = None;
            return FrameDisposition::Keep { skip_interleaved: 0 };
        }
        let frame_pts_secs = pts as f64 * tb_num / tb_den;
        let in_rate = frame.rate() as f64;
        let frame_dur_secs = if in_rate > 0.0 {
            frame.samples() as f64 / in_rate
        } else {
            0.0
        };
        if frame_pts_secs + frame_dur_secs <= target {
            return FrameDisposition::DropAll;
        }
        if frame_pts_secs >= target {
            self.pending_seek_secs = None;
            return FrameDisposition::Keep { skip_interleaved: 0 };
        }
        let skip_per_channel = ((target - frame_pts_secs) * self.out_rate as f64).round() as i64;
        let skip_per_channel = skip_per_channel.max(0) as usize;
        let skip_interleaved = skip_per_channel.saturating_mul(self.out_channels as usize);
        self.pending_seek_secs = None;
        FrameDisposition::Keep { skip_interleaved }
    }

    fn ingest_frame(&mut self, decoded: &Audio) {
        let skip = match self.frame_disposition(decoded) {
            FrameDisposition::DropAll => return,
            FrameDisposition::Keep { skip_interleaved } => skip_interleaved,
        };
        let mut resampled = Audio::empty();
        if self.resampler.run(decoded, &mut resampled).is_err() {
            return;
        }
        let before = self.buffer.len();
        self.append_samples(&resampled);
        if skip > 0 {
            let added = self.buffer.len() - before;
            let to_drain = skip.min(added);
            self.buffer.drain(before..before + to_drain);
        }
    }

    fn drain_decoder(&mut self) {
        let mut decoded = Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            self.ingest_frame(&decoded);
        }
    }

    fn append_samples(&mut self, frame: &Audio) {
        let samples = frame.samples();
        if samples == 0 {
            return;
        }
        let bytes = frame.data(0);
        let needed_bytes = samples
            .saturating_mul(self.out_channels as usize)
            .saturating_mul(std::mem::size_of::<f32>());
        let usable = bytes.len().min(needed_bytes);
        if usable < std::mem::size_of::<f32>() {
            return;
        }
        // SAFETY: ffmpeg audio buffers are 4-byte aligned for f32 and `usable`
        // is a multiple of sizeof(f32) by construction.
        let n_f32 = usable / std::mem::size_of::<f32>();
        let interleaved: &[f32] =
            unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, n_f32) };
        self.buffer.extend(interleaved.iter().copied());
    }

    fn fill_buffer(&mut self) {
        while self.buffer.is_empty() && !self.finished {
            let mut decoded = Audio::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    self.ingest_frame(&decoded);
                    continue;
                }
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::util::error::EAGAIN => {}
                Err(_) => {}
            }

            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut self.ictx) {
                Ok(()) => {
                    if packet.stream() == self.stream_index {
                        let _ = self.decoder.send_packet(&packet);
                    }
                }
                Err(_) => {
                    let _ = self.decoder.send_eof();
                    self.drain_decoder();
                    self.finished = true;
                }
            }
        }
    }
}

impl Iterator for FfmpegAudioSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.buffer.is_empty() {
            self.fill_buffer();
        }
        self.buffer.pop_front()
    }
}

impl Source for FfmpegAudioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.out_channels).unwrap_or(ChannelCount::new(2).unwrap())
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.out_rate).unwrap_or(SampleRate::new(44_100).unwrap())
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// Shared handshake between a gaplessly queued source and the [`AudioPlayer`]
/// that queued it. rodio's queue polls the next source only once the current
/// one runs dry, so `started` flipping *is* the track boundary — observed on
/// the playback thread, read on the UI thread.
#[derive(Default)]
struct Handover {
    /// Set by the queued source the first time it feeds the output.
    started: AtomicBool,
    /// Set by the player to drop a queue that is no longer wanted. A source
    /// already in rodio's queue can't be pulled back out, so it yields nothing
    /// instead and the queue moves straight past it.
    cancelled: AtomicBool,
}

/// What a queued source applies to the tap when it takes over: the new track's
/// format, plus the handshake to signal through.
struct Takeover {
    channels: u16,
    sample_rate: u32,
    handover: Arc<Handover>,
}

struct TapSource<S> {
    inner: S,
    tap: SampleBuffer,
    /// `Some` only for a source queued behind another, until the moment it
    /// starts playing. A source that plays immediately configures the tap up
    /// front and leaves this `None`.
    takeover: Option<Takeover>,
}

impl<S> TapSource<S>
where
    S: Source<Item = f32>,
{
    /// Wrap a source that starts playing right away.
    fn new(inner: S, tap: SampleBuffer) -> Self {
        tap.set_format(inner.channels().get(), inner.sample_rate().get());
        Self {
            inner,
            tap,
            takeover: None,
        }
    }

    /// Wrap a source queued behind the playing one. The tap is left alone until
    /// this source actually reaches the output — retuning it any earlier would
    /// corrupt the position and visualizers of the track still playing.
    fn queued(inner: S, tap: SampleBuffer, handover: Arc<Handover>) -> Self {
        let takeover = Takeover {
            channels: inner.channels().get(),
            sample_rate: inner.sample_rate().get(),
            handover,
        };
        Self {
            inner,
            tap,
            takeover: Some(takeover),
        }
    }
}

impl<S> Iterator for TapSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(t) = &self.takeover
            && t.handover.cancelled.load(Ordering::Acquire)
        {
            return None;
        }
        let v = self.inner.next()?;
        if let Some(t) = self.takeover.take() {
            // First sample of a queued track: hand the tap over. `rebase`
            // rather than `reset` — the samples already in the ring are the
            // tail of the previous track, and keeping them is what makes the
            // visualizers flow through the seam.
            self.tap.set_format(t.channels, t.sample_rate);
            self.tap.rebase(Duration::ZERO);
            t.handover.started.store(true, Ordering::Release);
        }
        self.tap.push(v);
        Some(v)
    }
}

impl<S> Source for TapSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

/// Open an ffmpeg input, muting libav's chatty stderr warnings first (they
/// corrupt the TUI when written to the terminal).
fn open_input(path: &Path) -> Result<ffmpeg::format::context::Input> {
    ffmpeg::init().ok();
    ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);
    ffmpeg::format::input(&path.to_path_buf()).with_context(|| format!("opening {}", path.display()))
}

/// Open a plain audio file through rodio's symphonia decoder, returning the
/// boxed source and its duration hint. Shared by immediate playback and by the
/// gapless preload so the two can't drift apart.
fn open_audio_source(path: &Path) -> Result<(Box<dyn Source + Send>, Option<Duration>)> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let source = Decoder::new(BufReader::new(file))
        .with_context(|| format!("decoding {}", path.display()))?;
    let duration = source.total_duration();
    Ok((Box::new(source), duration))
}

/// One selectable audio track inside a container, paired with the ffmpeg
/// stream index needed to decode it.
#[derive(Clone)]
struct AudioTrackInfo {
    stream_index: usize,
    label: String,
}

/// Enumerate every audio stream in `path`, returning the tracks (in container
/// order) and the index — into the returned vector — of the default ("best")
/// track ffmpeg would otherwise pick. Returns an empty list on any failure.
fn list_audio_tracks(path: &Path) -> (Vec<AudioTrackInfo>, usize) {
    let Ok(ictx) = open_input(path) else {
        return (Vec::new(), 0);
    };
    let best_index = ictx.streams().best(MediaType::Audio).map(|s| s.index());
    let mut tracks: Vec<AudioTrackInfo> = Vec::new();
    let mut default_idx = 0;
    for stream in ictx.streams() {
        if stream.parameters().medium() != MediaType::Audio {
            continue;
        }
        let idx = stream.index();
        if Some(idx) == best_index {
            default_idx = tracks.len();
        }
        let label = audio_track_label(&stream, tracks.len() + 1);
        tracks.push(AudioTrackInfo {
            stream_index: idx,
            label,
        });
    }
    (tracks, default_idx)
}

/// Build a human label for an audio stream from its language/title metadata,
/// suffixed with a channel-layout hint (e.g. "English (5.1)").
fn audio_track_label(stream: &ffmpeg::format::stream::Stream<'_>, n: usize) -> String {
    let meta = stream.metadata();
    let title = meta
        .get("title")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let language = meta
        .get("language")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "und");
    let lang_name = language
        .as_deref()
        .map(sparkplayer_core::subtitles::language_display_name);

    // Prefer the language name (matching the subtitle loader); fall back to a
    // meaningful title, then to a numbered placeholder. The channel-layout hint
    // disambiguates same-language tracks (e.g. "French (5.1)" vs "French (stereo)").
    let base = match lang_name {
        Some(name) => name,
        None => match title {
            Some(t) => t,
            None => format!("Track {n}"),
        },
    };
    match channel_desc(stream) {
        Some(c) => format!("{base} ({c})"),
        None => base,
    }
}

/// Best-effort channel-layout descriptor ("mono", "stereo", "5.1", …).
fn channel_desc(stream: &ffmpeg::format::stream::Stream<'_>) -> Option<String> {
    let ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters()).ok()?;
    let decoder = ctx.decoder().audio().ok()?;
    Some(match decoder.channels() {
        0 => return None,
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        6 => "5.1".to_string(),
        8 => "7.1".to_string(),
        c => format!("{c}ch"),
    })
}

/// A track already decoded-ready and sitting in rodio's queue behind the
/// playing one, waiting to take over without a gap.
struct Preloaded {
    path: PathBuf,
    duration: Option<Duration>,
    handover: Arc<Handover>,
}

pub struct AudioPlayer {
    sink: MixerDeviceSink,
    player: Player,
    pub tap: SampleBuffer,
    volume: f32,
    pub current_path: Option<PathBuf>,
    /// Audio tracks of the current file (only populated for video containers).
    audio_tracks: Vec<AudioTrackInfo>,
    /// Index into `audio_tracks` of the track currently being decoded.
    active_audio_track: usize,
    /// The gaplessly queued next track, if one is armed.
    preloaded: Option<Preloaded>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .context("failed to open default audio output")?;
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        let tap = SampleBuffer::new();
        Ok(Self {
            sink,
            player,
            tap,
            volume: 0.8,
            current_path: None,
            audio_tracks: Vec::new(),
            active_audio_track: 0,
            preloaded: None,
        })
    }

    /// Open the audio of a video file on the currently selected track, falling
    /// back to ffmpeg's "best" stream when no track list is available.
    fn open_video_audio(&self, path: &Path) -> Result<FfmpegAudioSource> {
        match self.audio_tracks.get(self.active_audio_track) {
            Some(t) => FfmpegAudioSource::open_stream(path, t.stream_index),
            None => FfmpegAudioSource::open(path),
        }
    }

    pub fn play_file(&mut self, path: &Path) -> Result<Option<Duration>> {
        self.rebuild_player();
        self.tap.reset();
        self.current_path = Some(path.to_path_buf());
        self.audio_tracks.clear();
        self.active_audio_track = 0;

        let total = if library::is_video_file(path) {
            let (tracks, default_idx) = list_audio_tracks(path);
            self.audio_tracks = tracks;
            self.active_audio_track = default_idx;
            let source = self.open_video_audio(path)?;
            let total = source.total_duration();
            let tapped = TapSource::new(source, self.tap.clone());
            self.player.append(tapped);
            total
        } else {
            let (source, total) = open_audio_source(path)?;
            self.player.append(TapSource::new(source, self.tap.clone()));
            total
        };
        self.player.play();
        Ok(total)
    }

    fn seek_to(&mut self, path: &Path, target: Duration) -> Result<()> {
        let was_paused = self.player.is_paused();
        self.rebuild_player();

        self.tap.reset();
        self.tap.set_base_offset(target);

        if library::is_video_file(path) {
            let mut source = self.open_video_audio(path)?;
            source.seek(target)?;
            let tapped = TapSource::new(source, self.tap.clone());
            self.player.append(tapped);
        } else {
            let file = File::open(path)?;
            let mut source = Decoder::new(BufReader::new(file))?;
            // Prefer a container-level seek (symphonia): cost is independent of
            // the target position. Falling back to `skip_duration` decodes and
            // discards every sample from the start of the file, so the delay
            // grows the further into the track we seek.
            if source.try_seek(target).is_ok() {
                let tapped = TapSource::new(source, self.tap.clone());
                self.player.append(tapped);
            } else {
                let file = File::open(path)?;
                let source = Decoder::new(BufReader::new(file))?;
                let skipped = source.skip_duration(target);
                let tapped = TapSource::new(skipped, self.tap.clone());
                self.player.append(tapped);
            }
        }

        if was_paused {
            self.player.pause();
        } else {
            self.player.play();
        }
        Ok(())
    }

    /// Switch to a different audio track, re-decoding from the current playback
    /// position so picture and sound stay put. No-op for non-video files or an
    /// out-of-range index.
    fn set_audio_track(&mut self, idx: usize) -> Result<()> {
        let Some(path) = self.current_path.clone() else {
            return Ok(());
        };
        if !library::is_video_file(&path) || idx >= self.audio_tracks.len() {
            return Ok(());
        }
        self.active_audio_track = idx;
        let target = self.tap.position();
        let was_paused = self.player.is_paused();
        self.rebuild_player();
        self.tap.reset();
        self.tap.set_base_offset(target);

        let mut source = self.open_video_audio(&path)?;
        source.seek(target)?;
        let tapped = TapSource::new(source, self.tap.clone());
        self.player.append(tapped);

        if was_paused {
            self.player.pause();
        } else {
            self.player.play();
        }
        Ok(())
    }

    /// Tear the playback chain down and start a fresh one. Everything queued —
    /// the playing track and any gapless preload behind it — goes with it, so
    /// callers that rebuild must re-arm the preload afterwards.
    fn rebuild_player(&mut self) {
        self.cancel_preload();
        self.player.stop();
        self.player = Player::connect_new(self.sink.mixer());
        self.player.set_volume(self.volume);
    }

    /// Drop the queued track. It is already inside rodio's queue and can't be
    /// taken back out, so it is flagged instead: it yields nothing when the
    /// queue reaches it, and playback moves straight past.
    fn cancel_preload(&mut self) {
        if let Some(p) = self.preloaded.take() {
            p.handover.cancelled.store(true, Ordering::Release);
        }
    }

    /// Queue `path` to start the instant the current track ends.
    ///
    /// Only plain audio files are queued: a video needs its picture pipeline
    /// and subtitles rebuilt at the boundary, which is exactly the stop-and-open
    /// work gapless playback exists to avoid, so those keep the normal path.
    fn preload(&mut self, path: &Path) -> Result<bool> {
        let playing_video = self
            .current_path
            .as_deref()
            .is_some_and(library::is_video_file);
        if self.current_path.is_none() || playing_video || self.preloaded.is_some() {
            return Ok(false);
        }
        if library::is_video_file(path) {
            return Ok(false);
        }
        let (source, duration) = open_audio_source(path)?;
        let source = self.match_playing_format(source);
        let handover = Arc::new(Handover::default());
        self.player.append(TapSource::queued(
            source,
            self.tap.clone(),
            handover.clone(),
        ));
        self.preloaded = Some(Preloaded {
            path: path.to_path_buf(),
            duration,
            handover,
        });
        Ok(true)
    }

    /// Present `source` in the format of the track already playing, converting
    /// it first if they differ.
    ///
    /// rodio's mixer builds its resampler from whatever was playing when the
    /// chain was set up and doesn't fully re-derive it when a queued source
    /// takes over, so a track at another sample rate comes out at the wrong
    /// speed — measurably so: 8 kHz behind 44.1 kHz plays in half its length.
    /// Converting up front means the mixer only ever sees one continuous
    /// stream, which is what gapless playback is asking of it anyway.
    fn match_playing_format(&self, source: Box<dyn Source + Send>) -> Box<dyn Source + Send> {
        let (Some(channels), Some(rate)) = (
            ChannelCount::new(self.tap.channels()),
            SampleRate::new(self.tap.sample_rate()),
        ) else {
            return source;
        };
        if source.channels() == channels && source.sample_rate() == rate {
            return source;
        }
        Box::new(UniformSourceIterator::new(source, channels, rate))
    }

    /// Best-effort audio output latency from the negotiated CPAL buffer.
    pub fn output_buffer_latency(&self) -> Duration {
        let cfg = self.sink.config();
        let rate = cfg.sample_rate().get().max(1) as f64;
        let frames = match cfg.buffer_size() {
            rodio::cpal::BufferSize::Fixed(n) => *n as f64,
            rodio::cpal::BufferSize::Default => rate * 0.050,
        };
        Duration::from_secs_f64(frames / rate)
    }
}

/// The native audio backend. Method names mirror the `AudioBackend` trait;
/// inherent methods on `AudioPlayer` take precedence inside the impl, so the
/// delegations below do not recurse.
impl AudioBackend for AudioPlayer {
    fn play(&mut self, source: &TrackRef) -> Result<Option<Duration>> {
        match source {
            TrackRef::Path(p) => self.play_file(p),
            TrackRef::Url(..) => anyhow::bail!("the native build cannot play URLs"),
        }
    }

    fn toggle_pause(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }

    fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    fn is_finished(&self) -> bool {
        self.player.empty()
    }

    fn stop(&mut self) {
        self.cancel_preload();
        self.player.stop();
        self.tap.reset();
        self.current_path = None;
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        self.player.set_volume(self.volume);
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn seek_relative(&mut self, delta_secs: f64, total: Option<Duration>) -> Result<()> {
        let Some(path) = self.current_path.clone() else {
            return Ok(());
        };
        let cur = self.tap.position().as_secs_f64();
        let mut target_secs = (cur + delta_secs).max(0.0);
        if let Some(t) = total {
            let max = t.as_secs_f64();
            if max > 0.0 && target_secs > max - 0.05 {
                target_secs = (max - 0.05).max(0.0);
            }
        }
        self.seek_to(&path, Duration::from_secs_f64(target_secs))
    }

    fn position(&self) -> Duration {
        self.tap.position()
    }

    fn tap(&self) -> &SampleBuffer {
        &self.tap
    }

    fn output_buffer_latency(&self) -> Duration {
        AudioPlayer::output_buffer_latency(self)
    }

    fn audio_tracks(&self) -> Vec<String> {
        self.audio_tracks.iter().map(|t| t.label.clone()).collect()
    }

    fn active_audio_track(&self) -> Option<usize> {
        if self.audio_tracks.is_empty() {
            None
        } else {
            Some(self.active_audio_track)
        }
    }

    fn set_audio_track(&mut self, idx: usize) -> Result<()> {
        AudioPlayer::set_audio_track(self, idx)
    }

    fn preload_next(&mut self, source: &TrackRef) -> Result<bool> {
        match source {
            TrackRef::Path(p) => self.preload(p),
            TrackRef::Url(..) => Ok(false),
        }
    }

    fn has_preload(&self) -> bool {
        self.preloaded.is_some()
    }

    fn clear_preload(&mut self) {
        AudioPlayer::cancel_preload(self);
    }

    fn take_started_track(&mut self) -> Option<StartedTrack> {
        let started = self
            .preloaded
            .as_ref()?
            .handover
            .started
            .load(Ordering::Acquire);
        if !started {
            return None;
        }
        let preloaded = self.preloaded.take()?;
        self.current_path = Some(preloaded.path);
        // Preloading is audio-only, so the new file has no selectable video
        // audio tracks to carry over.
        self.audio_tracks.clear();
        self.active_audio_track = 0;
        Some(StartedTrack {
            duration: preloaded.duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    /// A queued track's stand-in: four samples of a format deliberately unlike
    /// the "previous track" the tap is left holding.
    fn queued_buffer() -> SamplesBuffer {
        SamplesBuffer::new(
            ChannelCount::new(1).unwrap(),
            SampleRate::new(8_000).unwrap(),
            vec![0.25f32; 4],
        )
    }

    /// Leave the tap looking like a stereo 44.1 kHz track has been playing for
    /// a while, which is what a queued source has to take over from.
    fn tap_mid_track() -> SampleBuffer {
        let tap = SampleBuffer::new();
        tap.set_format(2, 44_100);
        for _ in 0..44_100 {
            tap.push(0.5);
        }
        tap
    }

    #[test]
    fn queued_source_takes_the_tap_over_on_its_first_sample() {
        let tap = tap_mid_track();
        let handover = Arc::new(Handover::default());
        let mut source = TapSource::queued(queued_buffer(), tap.clone(), handover.clone());

        // Constructing it must not disturb the track still playing: rodio holds
        // the source in its queue for as long as that one keeps producing.
        assert_eq!(tap.sample_rate(), 44_100);
        assert_eq!(tap.channels(), 2);
        assert!(!handover.started.load(Ordering::Acquire));
        let mid_track_pos = tap.position();
        assert!(mid_track_pos > Duration::from_millis(400));

        assert_eq!(source.next(), Some(0.25));

        // The first sample is the boundary: format and position follow it.
        assert!(handover.started.load(Ordering::Acquire));
        assert_eq!(tap.sample_rate(), 8_000);
        assert_eq!(tap.channels(), 1);
        assert!(tap.position() < Duration::from_millis(1));

        // ...but the waveform history survives, so the visualizers flow through
        // the seam instead of blinking to silence.
        let mut window = [0.0f32; 64];
        assert_eq!(tap.latest_mono(&mut window), 64);
        assert!(window.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn cancelled_queue_yields_nothing_and_leaves_the_tap_alone() {
        let tap = tap_mid_track();
        let handover = Arc::new(Handover::default());
        let mut source = TapSource::queued(queued_buffer(), tap.clone(), handover.clone());
        let mid_track_pos = tap.position();

        handover.cancelled.store(true, Ordering::Release);

        // Ending immediately is how a queued track is un-queued: rodio can't be
        // asked to drop it, so it plays nothing and the queue moves straight on.
        assert_eq!(source.next(), None);
        assert!(!handover.started.load(Ordering::Acquire));
        assert_eq!(tap.sample_rate(), 44_100);
        assert_eq!(tap.position(), mid_track_pos);
    }

    #[test]
    fn immediately_playing_source_configures_the_tap_up_front() {
        let tap = tap_mid_track();
        let mut source = TapSource::new(queued_buffer(), tap.clone());
        // No handover to wait for — this one is the track being started.
        assert_eq!(tap.sample_rate(), 8_000);
        assert_eq!(source.next(), Some(0.25));
    }

    /// Write a mono 16-bit PCM WAV holding a constant `value` — a flat signal
    /// makes any inserted silence, and the exact seam between two tracks,
    /// unmistakable in the mixed output.
    fn write_dc_wav(name: &str, rate: u32, secs: f32, value: f32) -> PathBuf {
        let frames = (rate as f32 * secs) as u32;
        let data_len = frames * 2;
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for _ in 0..frames {
            out.extend_from_slice(&((value * i16::MAX as f32) as i16).to_le_bytes());
        }
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, out).unwrap();
        path
    }

    /// Write a mono 16-bit PCM WAV of `secs` seconds at `rate` into the temp
    /// dir. Hand-rolled so the test needs no fixtures and no external tools.
    fn write_wav(name: &str, rate: u32, secs: f32) -> PathBuf {
        let frames = (rate as f32 * secs) as u32;
        let data_len = frames * 2;
        let mut out: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let t = i as f32 / rate as f32;
            let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
            out.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
        }
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, out).expect("writing test wav");
        path
    }

    /// Play `first` then `second` through one rodio queue and the same
    /// conversion the mixer applies, returning the mixed 44.1 kHz stereo
    /// output. This is the real signal path minus the sound card.
    fn mix_two_tracks(first: &Path, second: &Path, samples: usize) -> Vec<f32> {
        let tap = SampleBuffer::new();
        let (player, queue) = Player::new();
        let (a, _) = open_audio_source(first).expect("opening the first track");
        let (b, _) = open_audio_source(second).expect("opening the second track");
        let (channels, rate) = (a.channels(), a.sample_rate());
        player.append(TapSource::new(a, tap.clone()));
        let b: Box<dyn Source + Send> = if b.channels() == channels && b.sample_rate() == rate {
            b
        } else {
            Box::new(UniformSourceIterator::new(b, channels, rate))
        };
        player.append(TapSource::queued(b, tap, Arc::new(Handover::default())));
        let mut mixed = UniformSourceIterator::new(
            queue,
            ChannelCount::new(2).unwrap(),
            SampleRate::new(44_100).unwrap(),
        );
        (0..samples).map(|_| mixed.next().unwrap_or(0.0)).collect()
    }

    /// The property the whole feature is named after, measured on the mixed
    /// output: the second track starts on the sample after the first one ends,
    /// with no silence anywhere in between — and it plays at its true length,
    /// whatever sample rate it was recorded at.
    #[test]
    fn queued_track_follows_the_first_with_no_silence_between_them() {
        // 0.1 s per track = 4410 output frames = 8820 interleaved samples each.
        const PER_TRACK: usize = 8_820;
        // Resampling either end costs a handful of samples at the edges.
        const TOLERANCE: usize = 64;

        for rate in [44_100u32, 22_050, 48_000, 96_000, 8_000] {
            let first = write_dc_wav("sparkplayer-seam-a.wav", 44_100, 0.1, 0.5);
            let second = write_dc_wav("sparkplayer-seam-b.wav", rate, 0.1, -0.7);
            let out = mix_two_tracks(&first, &second, 44_100);
            let _ = std::fs::remove_file(&first);
            let _ = std::fs::remove_file(&second);

            // The first negative sample is the first sample of track two.
            let seam = out
                .iter()
                .position(|&v| v < -0.3)
                .unwrap_or_else(|| panic!("{rate} Hz: the second track never played"));
            let end = out.iter().rposition(|&v| v.abs() > 0.3).unwrap();

            assert!(
                seam.abs_diff(PER_TRACK) <= TOLERANCE,
                "{rate} Hz: the seam landed at {seam}, not {PER_TRACK}"
            );
            assert!(
                end.abs_diff(2 * PER_TRACK) <= TOLERANCE,
                "{rate} Hz: playback ran to {end}, not {}: the queued track was \
                 resampled at the wrong ratio",
                2 * PER_TRACK
            );
            // Nothing quiet anywhere before the end: no gap, no drop-out.
            let quiet = out[..end].iter().filter(|v| v.abs() < 0.05).count();
            assert!(
                quiet <= TOLERANCE,
                "{rate} Hz: {quiet} silent samples mid-stream"
            );
        }
    }

    /// The end-to-end property gapless playback rests on: rodio picks the
    /// queued source up in the same callback the previous one runs dry, so the
    /// handover needs no restart of the playback chain — and the tap retunes to
    /// the new track's format at exactly that moment.
    ///
    /// Skipped when no audio device can be opened (headless CI). Runs silent:
    /// rodio applies volume downstream of the tap, so muting the output leaves
    /// the samples this asserts on untouched.
    #[test]
    fn preloaded_track_takes_over_the_output_when_the_first_ends() {
        let Ok(mut player) = AudioPlayer::new() else {
            eprintln!("skipping: no audio output device");
            return;
        };
        let first = write_wav("sparkplayer-gapless-a.wav", 22_050, 0.4);
        let second = write_wav("sparkplayer-gapless-b.wav", 8_000, 0.4);

        player.set_volume(0.0);
        player.play_file(&first).expect("playing the first track");
        assert!(player.preload(&second).expect("queueing the second track"));
        assert!(player.has_preload());
        assert_eq!(player.current_path.as_deref(), Some(first.as_path()));

        let mut started = None;
        for _ in 0..500 {
            if let Some(s) = player.take_started_track() {
                started = Some(s);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let started = started.expect("the queued track never took over");
        assert_eq!(player.current_path.as_deref(), Some(second.as_path()));
        assert!(!player.has_preload());
        assert!(!player.is_finished(), "playback stopped at the seam");
        // The duration hint travels with the handover so the UI can fill the
        // progress bar in without reopening the file.
        let hint = started
            .duration
            .expect("no duration hint for the queued track");
        assert!(hint > Duration::from_millis(300) && hint < Duration::from_millis(500));
        // The second track was recorded at 8 kHz but is presented in the
        // playing track's format, so the mixer sees one unbroken stream.
        assert_eq!(player.tap.sample_rate(), 22_050);
        // Position restarts with the new track rather than carrying on.
        assert!(player.tap.position() < Duration::from_millis(400));

        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    /// The 1 GB sample lives at the repo root; tests run from the crate dir.
    fn sample() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../marsexpress.mkv")
    }

    #[test]
    fn enumerates_multiple_audio_tracks() {
        let path = sample();
        if !path.exists() {
            eprintln!("skipping: {} not present", path.display());
            return;
        }
        let (tracks, default_idx) = list_audio_tracks(&path);
        assert!(
            tracks.len() >= 2,
            "expected >=2 audio tracks, got {}: {:?}",
            tracks.len(),
            tracks.iter().map(|t| &t.label).collect::<Vec<_>>()
        );
        assert!(default_idx < tracks.len());
        // The sample carries a French 5.1 and an English stereo track.
        let labels: Vec<&str> = tracks.iter().map(|t| t.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("French")),
            "labels: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("English")),
            "labels: {labels:?}"
        );

        // Each enumerated stream must actually open and decode at least a sample.
        for t in &tracks {
            let mut src = FfmpegAudioSource::open_stream(&path, t.stream_index)
                .unwrap_or_else(|e| panic!("opening track '{}' ({e})", t.label));
            assert!(
                src.next().is_some(),
                "track '{}' produced no samples",
                t.label
            );
        }
    }
}
