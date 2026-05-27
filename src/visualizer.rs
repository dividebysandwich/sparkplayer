use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::audio::SampleBuffer;

const FFT_SIZE: usize = 1024;
const WAVE_SIZE: usize = 2048;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VisMode {
    Spectrum,
    Waveform,
    ScrollingWaveform,
    Spectrogram,
    Lissajous,
    Spectrum3D,
    Cassette,
}

impl VisMode {
    pub fn label(&self) -> &'static str {
        match self {
            VisMode::Spectrum => "FFT Bars",
            VisMode::Waveform => "Waveform",
            VisMode::ScrollingWaveform => "Scrolling Wave",
            VisMode::Spectrogram => "Spectrogram",
            VisMode::Lissajous => "Stereo X/Y",
            VisMode::Spectrum3D => "Spectrum 3D",
            VisMode::Cassette => "Cassette Tape",
        }
    }
    pub fn cycle(self) -> Self {
        match self {
            VisMode::Spectrum => VisMode::Waveform,
            VisMode::Waveform => VisMode::ScrollingWaveform,
            VisMode::ScrollingWaveform => VisMode::Spectrogram,
            VisMode::Spectrogram => VisMode::Lissajous,
            VisMode::Lissajous => VisMode::Spectrum3D,
            VisMode::Spectrum3D => VisMode::Cassette,
            VisMode::Cassette => VisMode::Spectrum,
        }
    }
    /// Stable identifier used in the persisted config file.
    pub fn name(&self) -> &'static str {
        match self {
            VisMode::Spectrum => "spectrum",
            VisMode::Waveform => "waveform",
            VisMode::ScrollingWaveform => "scrolling-wave",
            VisMode::Spectrogram => "spectrogram",
            VisMode::Lissajous => "lissajous",
            VisMode::Spectrum3D => "spectrum-3d",
            VisMode::Cassette => "cassette",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "spectrum" => VisMode::Spectrum,
            "waveform" => VisMode::Waveform,
            "scrolling-wave" => VisMode::ScrollingWaveform,
            "spectrogram" => VisMode::Spectrogram,
            "lissajous" => VisMode::Lissajous,
            "spectrum-3d" => VisMode::Spectrum3D,
            "cassette" => VisMode::Cassette,
            _ => return None,
        })
    }
}

pub struct Visualizer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    samples: Vec<f32>,
    spectrum_buf: Vec<Complex32>,
    mags: Vec<f32>,
    smoothed_bars: Vec<f32>,
    pub scroll_wave: VecDeque<f32>,
    pub spectrogram_cols: VecDeque<Vec<f32>>,
    last_consumed_for_scroll: u64,
    stereo_samples: Vec<(f32, f32)>,
    pub spectrum_3d_rows: VecDeque<Vec<f32>>,
    last_waveform: Vec<f32>,
    last_lissajous: Vec<(f32, f32)>,
    cassette_phase: f32,
    last_consumed_for_cassette: u64,
    pub mode: VisMode,
}

impl Visualizer {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos()
            })
            .collect();
        let window_sum: f32 = window.iter().sum();
        Self {
            fft,
            window,
            window_sum,
            samples: vec![0.0; FFT_SIZE.max(WAVE_SIZE)],
            spectrum_buf: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            mags: vec![0.0; FFT_SIZE / 2],
            smoothed_bars: Vec::new(),
            scroll_wave: VecDeque::new(),
            spectrogram_cols: VecDeque::new(),
            last_consumed_for_scroll: 0,
            stereo_samples: Vec::new(),
            spectrum_3d_rows: VecDeque::new(),
            last_waveform: Vec::new(),
            last_lissajous: Vec::new(),
            cassette_phase: 0.0,
            last_consumed_for_cassette: 0,
            mode: VisMode::Spectrum,
        }
    }

    /// Advance the cassette spindle phase based on samples played since the
    /// last call, returning the current angle in radians. Holds steady when
    /// playback is paused.
    pub fn cassette_phase(&mut self, tap: &SampleBuffer, active: bool) -> f32 {
        let consumed = tap.samples_consumed();
        if !active {
            self.last_consumed_for_cassette = consumed;
            return self.cassette_phase;
        }
        let channels = tap.channels().max(1) as u64;
        let sr = tap.sample_rate().max(1) as f32;
        let new_frames = consumed.saturating_sub(self.last_consumed_for_cassette) / channels;
        self.last_consumed_for_cassette = consumed;
        let dt = new_frames as f32 / sr;
        // Roughly one revolution per five seconds — slow enough that the eye
        // tracks individual spokes via subpixel steps.
        let revs_per_sec = 0.2;
        self.cassette_phase += dt * revs_per_sec * std::f32::consts::TAU;
        // Wrap to keep precision over long sessions.
        let tau = std::f32::consts::TAU;
        if self.cassette_phase > tau * 1024.0 {
            self.cassette_phase = self.cassette_phase.rem_euclid(tau);
        }
        self.cassette_phase
    }

    fn compute_fft(&mut self, tap: &SampleBuffer) {
        if self.samples.len() < FFT_SIZE {
            self.samples.resize(FFT_SIZE, 0.0);
        }
        let _ = tap.latest_mono(&mut self.samples[..FFT_SIZE]);
        for i in 0..FFT_SIZE {
            let s = self.samples[i] * self.window[i];
            self.spectrum_buf[i] = Complex32::new(s, 0.0);
        }
        self.fft.process(&mut self.spectrum_buf);
        let half = FFT_SIZE / 2;
        // Coherent normalization: full-scale sine -> magnitude 1.0 in its bin.
        let scale = 2.0 / self.window_sum;
        for i in 0..half {
            let c = self.spectrum_buf[i];
            let mag = (c.re * c.re + c.im * c.im).sqrt() * scale;
            self.mags[i] = mag;
        }
    }

    /// Reduce the half-spectrum into `bins` log-spaced bands, returning each as
    /// a normalized 0..1 value mapped from `db_floor..(db_floor + db_range)` dBFS.
    fn log_bin_db(&self, bins: usize, sample_rate: u32, db_floor: f32, db_range: f32) -> Vec<f32> {
        let half = FFT_SIZE / 2;
        let sr = sample_rate.max(1) as f32;
        let nyquist = sr * 0.5;
        let f_min = 30.0f32;
        let f_max = nyquist.min(16000.0);
        let mut out = vec![0.0f32; bins];
        for b in 0..bins {
            let t0 = b as f32 / bins as f32;
            let t1 = (b + 1) as f32 / bins as f32;
            let f0 = f_min * (f_max / f_min).powf(t0);
            let f1 = f_min * (f_max / f_min).powf(t1);
            let i0 = ((f0 / nyquist) * half as f32).floor() as usize;
            let i1 = ((f1 / nyquist) * half as f32).ceil() as usize;
            let i0 = i0.min(half - 1);
            let i1 = i1.min(half).max(i0 + 1);
            // Sum power across the band, then take amplitude.
            let mut power = 0.0f32;
            for v in &self.mags[i0..i1] {
                power += v * v;
            }
            let amp = power.sqrt();
            let db = 20.0 * (amp.max(1e-7)).log10();
            let n = ((db - db_floor) / db_range).clamp(0.0, 1.0);
            out[b] = n;
        }
        out
    }

    /// Spectrum bars: smoothed log-binned FFT mapped to 0..1.
    pub fn spectrum(
        &mut self,
        tap: &SampleBuffer,
        bins: usize,
        sample_rate: u32,
        active: bool,
    ) -> Vec<f32> {
        if self.smoothed_bars.len() != bins {
            self.smoothed_bars = vec![0.0; bins];
        }
        if !active {
            return self.smoothed_bars.clone();
        }
        self.compute_fft(tap);
        // -75 dB floor, 70 dB range => -5 dB ceiling.
        let raw = self.log_bin_db(bins, sample_rate, -75.0, 70.0);
        let attack = 0.55;
        let release = 0.12;
        for i in 0..bins {
            let target = raw[i];
            let cur = self.smoothed_bars[i];
            if target > cur {
                self.smoothed_bars[i] = cur + (target - cur) * attack;
            } else {
                self.smoothed_bars[i] = cur + (target - cur) * release;
            }
        }
        self.smoothed_bars.clone()
    }

    /// Instantaneous waveform: peak-binned absolute amplitudes from the tap.
    pub fn waveform(&mut self, tap: &SampleBuffer, points: usize, active: bool) -> Vec<f32> {
        if !active {
            if self.last_waveform.len() == points {
                return self.last_waveform.clone();
            }
            let mut out = vec![0.0f32; points];
            let take = self.last_waveform.len().min(points);
            out[..take].copy_from_slice(&self.last_waveform[..take]);
            return out;
        }
        let n = WAVE_SIZE.min(self.samples.len().max(WAVE_SIZE));
        if self.samples.len() < n {
            self.samples.resize(n, 0.0);
        }
        let frames = tap.latest_mono(&mut self.samples[..n]);
        let used = if frames == 0 { n } else { frames };
        let mut out = vec![0.0f32; points];
        if points == 0 {
            return out;
        }
        let chunk = used.max(1) as f32 / points as f32;
        for p in 0..points {
            let a = (p as f32 * chunk) as usize;
            let b = (((p + 1) as f32 * chunk) as usize).min(used);
            let mut peak = 0.0f32;
            for i in a..b {
                let v = self.samples[i].abs();
                if v > peak {
                    peak = v;
                }
            }
            out[p] = peak.min(1.0);
        }
        self.last_waveform = out.clone();
        out
    }

    /// Append one new peak column based on samples consumed since last call, and
    /// return a width-sized snapshot of the running history (newest at the right).
    pub fn scrolling_waveform(
        &mut self,
        tap: &SampleBuffer,
        width: usize,
        active: bool,
    ) -> Vec<f32> {
        if !active {
            // Keep the scroll cursor in sync so we don't dump a backlog of new
            // samples into the next column when playback resumes.
            self.last_consumed_for_scroll = tap.samples_consumed();
            let cap = width.max(2);
            while self.scroll_wave.len() > cap {
                self.scroll_wave.pop_front();
            }
            let mut out = vec![0.0f32; width];
            let start = width.saturating_sub(self.scroll_wave.len());
            for (i, v) in self.scroll_wave.iter().enumerate() {
                if start + i < width {
                    out[start + i] = *v;
                }
            }
            return out;
        }
        let consumed = tap.samples_consumed();
        let channels = tap.channels().max(1) as u64;
        let new_samples = consumed.saturating_sub(self.last_consumed_for_scroll);
        let new_frames = (new_samples / channels) as usize;
        self.last_consumed_for_scroll = consumed;

        // Sample only the freshly arrived window so each column reflects the
        // peak of *new* audio, producing genuine left-scrolling motion.
        let n = new_frames.clamp(1, 4096);
        if self.samples.len() < n {
            self.samples.resize(n, 0.0);
        }
        let got = tap.latest_mono(&mut self.samples[..n]);
        let used = got.min(n);
        let mut peak = 0.0f32;
        for v in &self.samples[..used] {
            let a = v.abs();
            if a > peak {
                peak = a;
            }
        }
        self.scroll_wave.push_back(peak.min(1.0));
        let cap = width.max(2);
        while self.scroll_wave.len() > cap {
            self.scroll_wave.pop_front();
        }
        let mut out = vec![0.0f32; width];
        let start = width.saturating_sub(self.scroll_wave.len());
        for (i, v) in self.scroll_wave.iter().enumerate() {
            if start + i < width {
                out[start + i] = *v;
            }
        }
        out
    }

    /// Append one new FFT column (binned to `height`) and return a snapshot of
    /// the running spectrogram history (newest at the right edge).
    pub fn spectrogram(
        &mut self,
        tap: &SampleBuffer,
        width: usize,
        height: usize,
        sample_rate: u32,
        active: bool,
    ) -> Vec<Vec<f32>> {
        if active {
            self.compute_fft(tap);
            // Slightly tighter dynamic range than the bars so colors saturate nicely.
            let col = self.log_bin_db(height, sample_rate, -70.0, 65.0);
            self.spectrogram_cols.push_back(col);
        }
        let cap = width.max(2);
        while self.spectrogram_cols.len() > cap {
            self.spectrogram_cols.pop_front();
        }
        self.spectrogram_cols.iter().cloned().collect()
    }

    /// Latest stereo frames for an X/Y oscillogram. Returns a slice of (L, R)
    /// pairs ready to be plotted on a Lissajous canvas.
    pub fn lissajous(
        &mut self,
        tap: &SampleBuffer,
        points: usize,
        active: bool,
    ) -> Vec<(f32, f32)> {
        if !active {
            if self.last_lissajous.len() <= points {
                return self.last_lissajous.clone();
            }
            return self.last_lissajous[..points].to_vec();
        }
        if self.stereo_samples.len() < points {
            self.stereo_samples.resize(points, (0.0, 0.0));
        }
        let frames = tap.latest_stereo(&mut self.stereo_samples[..points]);
        let used = frames.min(points);
        let out = self.stereo_samples[..used].to_vec();
        self.last_lissajous = out.clone();
        out
    }

    /// Append a fresh FFT row to the 3D history and return the visible window.
    /// The newest row is at the end of the returned vector.
    pub fn spectrum_3d(
        &mut self,
        tap: &SampleBuffer,
        bins: usize,
        sample_rate: u32,
        depth: usize,
        active: bool,
    ) -> Vec<Vec<f32>> {
        if active {
            self.compute_fft(tap);
            let col = self.log_bin_db(bins, sample_rate, -75.0, 70.0);
            self.spectrum_3d_rows.push_back(col);
        }
        let cap = depth.max(2);
        while self.spectrum_3d_rows.len() > cap {
            self.spectrum_3d_rows.pop_front();
        }
        self.spectrum_3d_rows.iter().cloned().collect()
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.cycle();
    }
}
