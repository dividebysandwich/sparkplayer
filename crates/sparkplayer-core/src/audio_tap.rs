//! The audio sample tap shared between the playback backend and the
//! visualizer. This is the single seam that lets every visualizer mode work
//! unchanged on both platforms: the native backend pushes every decoded sample
//! through it, while the web backend copies the Web Audio `AnalyserNode`'s
//! time-domain data into it each frame. The visualizer only ever reads from a
//! `SampleBuffer`, so it neither knows nor cares where the samples came from.

use std::sync::{Arc, Mutex};
use std::time::Duration;

// Holds enough interleaved samples for the largest FFT window (16384 mono
// frames × 2 channels) the visualizer can be configured to use, so a stereo
// window is never truncated.
const TAP_CAPACITY: usize = 32768;

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

    /// Push a single interleaved sample. Used by the native tap, one sample at
    /// a time, on the playback thread.
    pub fn push(&self, sample: f32) {
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

    /// Push a block of interleaved samples. Used by the web backend to copy a
    /// frame's worth of `AnalyserNode` data in one call.
    pub fn push_slice(&self, samples: &[f32]) {
        if let Ok(mut g) = self.inner.lock() {
            for &sample in samples {
                let idx = g.write;
                g.data[idx] = sample;
                g.write = (g.write + 1) % TAP_CAPACITY;
                if g.filled < TAP_CAPACITY {
                    g.filled += 1;
                }
                g.samples_consumed += 1;
            }
        }
    }

    pub fn set_format(&self, channels: u16, sample_rate: u32) {
        if let Ok(mut g) = self.inner.lock() {
            g.channels = channels.max(1);
            g.sample_rate = sample_rate.max(1);
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
