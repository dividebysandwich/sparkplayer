use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::format::sample::{Sample, Type as SampleType};
use ffmpeg::media::Type as MediaType;
use ffmpeg::software::resampling::Context as Resampler;
use ffmpeg::util::frame::audio::Audio;
use ffmpeg::ChannelLayout;
use rodio::source::Source;
use rodio::{ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate};

use crate::library;

const TAP_CAPACITY: usize = 8192;

#[derive(Clone, Default)]
pub struct SampleBuffer {
    inner: Arc<Mutex<SampleBufferInner>>,
}

#[derive(Default)]
struct SampleBufferInner {
    data: Vec<f32>,
    write: usize,
    filled: usize,
    samples_consumed: u64,
    channels: u16,
    sample_rate: u32,
    base_offset_secs: f64,
}

impl SampleBuffer {
    pub fn new() -> Self {
        let mut data = Vec::with_capacity(TAP_CAPACITY);
        data.resize(TAP_CAPACITY, 0.0);
        Self {
            inner: Arc::new(Mutex::new(SampleBufferInner {
                data,
                write: 0,
                filled: 0,
                samples_consumed: 0,
                channels: 2,
                sample_rate: 44100,
                base_offset_secs: 0.0,
            })),
        }
    }

    fn push(&self, sample: f32) {
        if let Ok(mut g) = self.inner.lock() {
            let idx = g.write;
            g.data[idx] = sample;
            g.write = (g.write + 1) % TAP_CAPACITY;
            if g.filled < TAP_CAPACITY {
                g.filled += 1;
            }
            g.samples_consumed += 1;
        }
    }

    fn set_format(&self, channels: ChannelCount, sample_rate: SampleRate) {
        if let Ok(mut g) = self.inner.lock() {
            g.channels = channels.get();
            g.sample_rate = sample_rate.get();
        }
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.data.iter_mut().for_each(|s| *s = 0.0);
            g.write = 0;
            g.filled = 0;
            g.samples_consumed = 0;
            g.base_offset_secs = 0.0;
        }
    }

    pub fn set_base_offset(&self, offset: Duration) {
        if let Ok(mut g) = self.inner.lock() {
            g.base_offset_secs = offset.as_secs_f64();
        }
    }

    /// Copy the most recent `n` mono-mixed samples into `out`. Returns the count copied.
    pub fn latest_mono(&self, out: &mut [f32]) -> usize {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let channels = g.channels.max(1) as usize;
        let mono_needed = out.len();
        let interleaved_needed = mono_needed * channels;
        let available = g.filled.min(interleaved_needed);
        if available == 0 {
            for v in out.iter_mut() {
                *v = 0.0;
            }
            return 0;
        }
        let mut idx = (g.write + TAP_CAPACITY - available) % TAP_CAPACITY;
        let frames = available / channels;
        for v in out.iter_mut().take(frames) {
            let mut acc = 0.0;
            for _ in 0..channels {
                acc += g.data[idx];
                idx = (idx + 1) % TAP_CAPACITY;
            }
            *v = acc / channels as f32;
        }
        for v in out.iter_mut().skip(frames) {
            *v = 0.0;
        }
        frames
    }

    /// Copy the most recent stereo frames into `out` as (left, right) pairs.
    /// Mono sources are duplicated to both channels; sources with >2 channels
    /// expose the first two. Returns the count of frames written.
    pub fn latest_stereo(&self, out: &mut [(f32, f32)]) -> usize {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let channels = g.channels.max(1) as usize;
        let frames_needed = out.len();
        let interleaved_needed = frames_needed * channels;
        let available = g.filled.min(interleaved_needed);
        if available == 0 {
            for v in out.iter_mut() {
                *v = (0.0, 0.0);
            }
            return 0;
        }
        let mut idx = (g.write + TAP_CAPACITY - available) % TAP_CAPACITY;
        let frames = available / channels;
        for v in out.iter_mut().take(frames) {
            if channels == 1 {
                let m = g.data[idx];
                idx = (idx + 1) % TAP_CAPACITY;
                *v = (m, m);
            } else {
                let l = g.data[idx];
                idx = (idx + 1) % TAP_CAPACITY;
                let r = g.data[idx];
                idx = (idx + 1) % TAP_CAPACITY;
                for _ in 2..channels {
                    idx = (idx + 1) % TAP_CAPACITY;
                }
                *v = (l, r);
            }
        }
        for v in out.iter_mut().skip(frames) {
            *v = (0.0, 0.0);
        }
        frames
    }

    pub fn position(&self) -> Duration {
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Duration::ZERO,
        };
        let frames = g.samples_consumed / g.channels.max(1) as u64;
        let secs = g.base_offset_secs + frames as f64 / g.sample_rate.max(1) as f64;
        Duration::from_secs_f64(secs.max(0.0))
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.lock().map(|g| g.sample_rate).unwrap_or(44100)
    }

    pub fn channels(&self) -> u16 {
        self.inner.lock().map(|g| g.channels).unwrap_or(2)
    }

    pub fn samples_consumed(&self) -> u64 {
        self.inner.lock().map(|g| g.samples_consumed).unwrap_or(0)
    }
}

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
    pub fn open(path: &Path) -> Result<Self> {
        ffmpeg::init().ok();
        // Mute libav warnings ("Could not update timestamps for skipped
        // samples", etc.) — they corrupt the TUI when written to stderr.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);
        let ictx = ffmpeg::format::input(&path.to_path_buf())
            .with_context(|| format!("opening {}", path.display()))?;
        let stream = ictx
            .streams()
            .best(MediaType::Audio)
            .context("file has no audio stream")?;
        let stream_index = stream.index();
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

    /// Seek the underlying input to `target` and reset decoder state. The
    /// container seek lands on the nearest keyframe ≤ target, which for video
    /// files can be many seconds before target. `pending_seek_secs` tells the
    /// decode path to drop samples earlier than `target` so what we emit
    /// actually starts there.
    pub fn seek(&mut self, target: Duration) -> Result<()> {
        let ts = (target.as_micros() as i64) * (ffmpeg::ffi::AV_TIME_BASE as i64) / 1_000_000;
        self.ictx.seek(ts, ..ts).ok();
        self.decoder.flush();
        self.buffer.clear();
        self.finished = false;
        self.pending_seek_secs = Some(target.as_secs_f64());
        Ok(())
    }

    /// Inspect a freshly-decoded frame against `pending_seek_secs` and decide
    /// whether to drop it entirely, drop a leading slice, or pass it through.
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
        // Entire frame is before target — drop it, keep the seek pending.
        if frame_pts_secs + frame_dur_secs <= target {
            return FrameDisposition::DropAll;
        }
        // Already at/past target — keep everything, clear the seek.
        if frame_pts_secs >= target {
            self.pending_seek_secs = None;
            return FrameDisposition::Keep { skip_interleaved: 0 };
        }
        // Partial overlap: skip the leading (target - frame_pts) seconds.
        let skip_per_channel =
            ((target - frame_pts_secs) * self.out_rate as f64).round() as i64;
        let skip_per_channel = skip_per_channel.max(0) as usize;
        let skip_interleaved =
            skip_per_channel.saturating_mul(self.out_channels as usize);
        self.pending_seek_secs = None;
        FrameDisposition::Keep { skip_interleaved }
    }

    /// Resample `decoded`, append to `self.buffer`, then drop the leading
    /// `skip_interleaved` samples from what was just appended.
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
        // ffmpeg-next's `plane::<T>(0)` returns only `nb_samples` elements,
        // which is wrong for packed (interleaved) formats — packed stereo
        // actually has `nb_samples * channels` floats in plane 0. Read raw
        // bytes via `data(0)` (linesize-sized) and reinterpret as f32 so we
        // pick up every sample.
        let bytes = frame.data(0);
        let needed_bytes = samples
            .saturating_mul(self.out_channels as usize)
            .saturating_mul(std::mem::size_of::<f32>());
        let usable = bytes.len().min(needed_bytes);
        if usable < std::mem::size_of::<f32>() {
            return;
        }
        // SAFETY: ffmpeg audio buffers are guaranteed to be aligned for the
        // sample type (4-byte aligned for f32) and `usable` is a multiple of
        // sizeof(f32) by construction.
        let n_f32 = usable / std::mem::size_of::<f32>();
        let interleaved: &[f32] = unsafe {
            std::slice::from_raw_parts(bytes.as_ptr() as *const f32, n_f32)
        };
        self.buffer.extend(interleaved.iter().copied());
    }

    fn fill_buffer(&mut self) {
        while self.buffer.is_empty() && !self.finished {
            // Try to receive any frames already buffered inside the decoder.
            let mut decoded = Audio::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    self.ingest_frame(&decoded);
                    continue;
                }
                Err(ffmpeg::Error::Other { errno })
                    if errno == ffmpeg::util::error::EAGAIN => {}
                Err(_) => {
                    // Decoder error — try to keep going with more packets.
                }
            }

            // Need more packets.
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

struct TapSource<S> {
    inner: S,
    tap: SampleBuffer,
}

impl<S> TapSource<S>
where
    S: Source<Item = f32>,
{
    fn new(inner: S, tap: SampleBuffer) -> Self {
        tap.set_format(inner.channels(), inner.sample_rate());
        Self { inner, tap }
    }
}

impl<S> Iterator for TapSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let v = self.inner.next()?;
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

pub struct AudioPlayer {
    sink: MixerDeviceSink,
    player: Player,
    pub tap: SampleBuffer,
    volume: f32,
    pub current_path: Option<PathBuf>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .context("failed to open default audio output")?;
        // We intentionally drop the sink on quit; suppress rodio's "audio
        // playing through this sink will stop" log so it doesn't print into
        // the user's shell after the TUI tears down.
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        let tap = SampleBuffer::new();
        Ok(Self {
            sink,
            player,
            tap,
            volume: 0.8,
            current_path: None,
        })
    }

    pub fn play_file(&mut self, path: &Path) -> Result<Option<Duration>> {
        self.player.stop();
        self.player = Player::connect_new(self.sink.mixer());
        self.player.set_volume(self.volume);
        self.tap.reset();
        self.current_path = Some(path.to_path_buf());

        let total = if library::is_video_file(path) {
            let source = FfmpegAudioSource::open(path)?;
            let total = source.total_duration();
            let tapped = TapSource::new(source, self.tap.clone());
            self.player.append(tapped);
            total
        } else {
            let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
            let source = Decoder::new(BufReader::new(file))
                .with_context(|| format!("decoding {}", path.display()))?;
            let total = source.total_duration();
            let tapped = TapSource::new(source, self.tap.clone());
            self.player.append(tapped);
            total
        };
        self.player.play();
        Ok(total)
    }

    /// Seek forwards or backwards by `delta` seconds, clamped to [0, total].
    pub fn seek_relative(&mut self, delta_secs: f64, total: Option<Duration>) -> Result<()> {
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

    fn seek_to(&mut self, path: &Path, target: Duration) -> Result<()> {
        let was_paused = self.player.is_paused();
        self.player.stop();
        self.player = Player::connect_new(self.sink.mixer());
        self.player.set_volume(self.volume);

        self.tap.reset();
        self.tap.set_base_offset(target);

        if library::is_video_file(path) {
            let mut source = FfmpegAudioSource::open(path)?;
            source.seek(target)?;
            let tapped = TapSource::new(source, self.tap.clone());
            self.player.append(tapped);
        } else {
            let file = File::open(path)?;
            let source = Decoder::new(BufReader::new(file))?;
            let skipped = source.skip_duration(target);
            let tapped = TapSource::new(skipped, self.tap.clone());
            self.player.append(tapped);
        }

        if was_paused {
            self.player.pause();
        } else {
            self.player.play();
        }
        Ok(())
    }

    /// Time between a sample being pulled by the mixer (tap increment) and it
    /// actually leaving the DAC, as far as we can know from inside the process.
    /// Reads the rodio sink's negotiated CPAL buffer; defaults to 50 ms if the
    /// backend reports Default/unknown.
    pub fn output_buffer_latency(&self) -> Duration {
        let cfg = self.sink.config();
        let rate = cfg.sample_rate().get().max(1) as f64;
        let frames = match cfg.buffer_size() {
            rodio::cpal::BufferSize::Fixed(n) => *n as f64,
            rodio::cpal::BufferSize::Default => rate * 0.050,
        };
        Duration::from_secs_f64(frames / rate)
    }

    pub fn toggle_pause(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    pub fn is_finished(&self) -> bool {
        self.player.empty()
    }

    #[allow(dead_code)]
    pub fn stop(&mut self) {
        self.player.stop();
        self.tap.reset();
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        self.player.set_volume(self.volume);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }
}
