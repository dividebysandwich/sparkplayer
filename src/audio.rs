use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

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

    fn set_format(&self, channels: u16, sample_rate: u32) {
        if let Ok(mut g) = self.inner.lock() {
            g.channels = channels;
            g.sample_rate = sample_rate;
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
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

pub struct AudioPlayer {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Sink,
    pub tap: SampleBuffer,
    volume: f32,
    pub current_path: Option<PathBuf>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let (stream, handle) =
            OutputStream::try_default().context("failed to open default audio output")?;
        let sink = Sink::try_new(&handle).context("failed to create audio sink")?;
        let tap = SampleBuffer::new();
        Ok(Self {
            _stream: stream,
            handle,
            sink,
            tap,
            volume: 0.8,
            current_path: None,
        })
    }

    pub fn play_file(&mut self, path: &Path) -> Result<Option<Duration>> {
        self.sink.stop();
        self.sink =
            Sink::try_new(&self.handle).context("failed to create sink for new track")?;
        self.sink.set_volume(self.volume);
        self.tap.reset();
        self.current_path = Some(path.to_path_buf());

        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let decoder = Decoder::new(BufReader::new(file))
            .with_context(|| format!("decoding {}", path.display()))?;
        let source = decoder.convert_samples::<f32>();
        let total = source.total_duration();
        let tapped = TapSource::new(source, self.tap.clone());
        self.sink.append(tapped);
        self.sink.play();
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
        let was_paused = self.sink.is_paused();
        self.sink.stop();
        self.sink =
            Sink::try_new(&self.handle).context("failed to create sink for seek")?;
        self.sink.set_volume(self.volume);

        let file = File::open(path)?;
        let decoder = Decoder::new(BufReader::new(file))?;
        let source = decoder.convert_samples::<f32>();
        let skipped = source.skip_duration(target);

        self.tap.reset();
        self.tap.set_base_offset(target);
        let tapped = TapSource::new(skipped, self.tap.clone());
        self.sink.append(tapped);

        if was_paused {
            self.sink.pause();
        } else {
            self.sink.play();
        }
        Ok(())
    }

    pub fn toggle_pause(&self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    #[allow(dead_code)]
    pub fn stop(&mut self) {
        self.sink.stop();
        self.tap.reset();
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        self.sink.set_volume(self.volume);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }
}
