use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::audio_tap::SampleBuffer;

const FFT_SIZE: usize = 1024;
const WAVE_SIZE: usize = 2048;
/// Larger FFT used only by the time-frequency displays (spectrogram, waterfall)
/// for ~4× finer frequency resolution — most noticeable in the bass, where the
/// 1024-point FFT's ~43 Hz bins smear everything together. The longer window
/// trades a little time resolution, which those scrolling views can absorb.
/// At 44.1 kHz this is ~93 ms / ~11 Hz per bin; 4096 stereo frames also fit the
/// 8192-sample tap exactly.
const SPEC_FFT_SIZE: usize = 4096;

/// Graphics waterfall image: frequency bins across (width), time rows down
/// (height, newest on top). Scaled to the panel by the graphics backend.
const WATERFALL_BINS: usize = 256;
const WATERFALL_ROWS: usize = 192;
/// Seconds between scroll rows — decouples the scroll speed from the (variable)
/// redraw rate so the waterfall advances at a steady ~30 rows/s.
const WATERFALL_ROW_DT: f64 = 1.0 / 30.0;
const WATERFALL_DB_FLOOR: f32 = -72.0;
const WATERFALL_DB_RANGE: f32 = 70.0;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VisMode {
    Spectrum,
    MirrorBars,
    Radial,
    Waveform,
    ScrollingWaveform,
    Spectrogram,
    Waterfall,
    Lissajous,
    Vu,
    Spectrum3D,
    Plasma,
    Cassette,
}

impl VisMode {
    pub fn label(&self) -> &'static str {
        match self {
            VisMode::Spectrum => "FFT Bars",
            VisMode::MirrorBars => "Mirror Bars",
            VisMode::Radial => "Radial",
            VisMode::Waveform => "Waveform",
            VisMode::ScrollingWaveform => "Scrolling Wave",
            VisMode::Spectrogram => "Spectrogram",
            VisMode::Waterfall => "Waterfall",
            VisMode::Lissajous => "Stereo X/Y",
            VisMode::Vu => "VU Meters",
            VisMode::Spectrum3D => "Spectrum 3D",
            VisMode::Plasma => "Plasma",
            VisMode::Cassette => "Cassette Tape",
        }
    }
    pub fn cycle(self) -> Self {
        match self {
            VisMode::Spectrum => VisMode::MirrorBars,
            VisMode::MirrorBars => VisMode::Radial,
            VisMode::Radial => VisMode::Waveform,
            VisMode::Waveform => VisMode::ScrollingWaveform,
            VisMode::ScrollingWaveform => VisMode::Spectrogram,
            VisMode::Spectrogram => VisMode::Waterfall,
            VisMode::Waterfall => VisMode::Lissajous,
            VisMode::Lissajous => VisMode::Vu,
            VisMode::Vu => VisMode::Spectrum3D,
            VisMode::Spectrum3D => VisMode::Plasma,
            VisMode::Plasma => VisMode::Cassette,
            VisMode::Cassette => VisMode::Spectrum,
        }
    }
    pub fn cycle_back(self) -> Self {
        match self {
            VisMode::Spectrum => VisMode::Cassette,
            VisMode::MirrorBars => VisMode::Spectrum,
            VisMode::Radial => VisMode::MirrorBars,
            VisMode::Waveform => VisMode::Radial,
            VisMode::ScrollingWaveform => VisMode::Waveform,
            VisMode::Spectrogram => VisMode::ScrollingWaveform,
            VisMode::Waterfall => VisMode::Spectrogram,
            VisMode::Lissajous => VisMode::Waterfall,
            VisMode::Vu => VisMode::Lissajous,
            VisMode::Spectrum3D => VisMode::Vu,
            VisMode::Plasma => VisMode::Spectrum3D,
            VisMode::Cassette => VisMode::Plasma,
        }
    }
    /// Stable identifier used in the persisted config file.
    pub fn name(&self) -> &'static str {
        match self {
            VisMode::Spectrum => "spectrum",
            VisMode::MirrorBars => "spectrum-mirror",
            VisMode::Radial => "radial",
            VisMode::Waveform => "waveform",
            VisMode::ScrollingWaveform => "scrolling-wave",
            VisMode::Spectrogram => "spectrogram",
            VisMode::Waterfall => "waterfall",
            VisMode::Lissajous => "lissajous",
            VisMode::Vu => "vu",
            VisMode::Spectrum3D => "spectrum-3d",
            VisMode::Plasma => "plasma",
            VisMode::Cassette => "cassette",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "spectrum" => VisMode::Spectrum,
            "spectrum-mirror" => VisMode::MirrorBars,
            "radial" => VisMode::Radial,
            "waveform" => VisMode::Waveform,
            "scrolling-wave" => VisMode::ScrollingWaveform,
            "spectrogram" => VisMode::Spectrogram,
            "waterfall" => VisMode::Waterfall,
            "lissajous" => VisMode::Lissajous,
            "vu" => VisMode::Vu,
            "spectrum-3d" => VisMode::Spectrum3D,
            "plasma" => VisMode::Plasma,
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
    /// High-resolution FFT (`SPEC_FFT_SIZE`) for the spectrogram/waterfall.
    spec_fft: Arc<dyn Fft<f32>>,
    spec_window: Vec<f32>,
    spec_window_sum: f32,
    spec_samples: Vec<f32>,
    spec_buf: Vec<Complex32>,
    spec_mags: Vec<f32>,
    smoothed_bars: Vec<f32>,
    pub scroll_wave: VecDeque<f32>,
    pub spectrogram_cols: VecDeque<Vec<f32>>,
    /// Time-ordered magnitude rows for the waterfall (oldest front, newest
    /// back), the RGB image built from them, and the clock of the last row.
    waterfall_rows: VecDeque<Vec<f32>>,
    waterfall_img: Vec<u8>,
    waterfall_last_secs: Option<f64>,
    last_consumed_for_scroll: u64,
    stereo_samples: Vec<(f32, f32)>,
    pub spectrum_3d_rows: VecDeque<Vec<f32>>,
    last_waveform: Vec<f32>,
    last_lissajous: Vec<(f32, f32)>,
    cassette_phase: f32,
    last_consumed_for_cassette: u64,
    /// Falling peak-hold caps for the mirrored bars.
    mirror_peaks: Vec<f32>,
    last_consumed_for_mirror: u64,
    /// Falling L/R peak-hold markers for the VU meters.
    vu_peak_hold: [f32; 2],
    last_consumed_for_vu: u64,
    /// Animation phase for the plasma field.
    plasma_phase: f32,
    /// Slowly rotating hue offset (0..1) that morphs the plasma palette.
    plasma_hue: f32,
    last_consumed_for_plasma: u64,
    pub mode: VisMode,
}

/// Stereo level readout for the VU meter visualizer.
#[derive(Copy, Clone, Debug, Default)]
pub struct Levels {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
    /// Inter-channel correlation in [-1, 1] (1 = mono, 0 = uncorrelated,
    /// -1 = out of phase).
    pub correlation: f32,
}

/// Reduce a half-spectrum magnitude slice into `bins` log-spaced bands, each a
/// normalized 0..1 value mapped from `db_floor..(db_floor + db_range)` dBFS.
/// The frequency→bin mapping uses `mags.len()`, so a longer FFT (more bins)
/// automatically yields finer resolution — especially in the low end.
fn log_bin_db_from(
    mags: &[f32],
    bins: usize,
    sample_rate: u32,
    db_floor: f32,
    db_range: f32,
) -> Vec<f32> {
    let half = mags.len().max(1);
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
        let i0 = (((f0 / nyquist) * half as f32).floor() as usize).min(half - 1);
        let i1 = (((f1 / nyquist) * half as f32).ceil() as usize)
            .min(half)
            .max(i0 + 1);
        // Sum power across the band, then take amplitude.
        let mut power = 0.0f32;
        for v in &mags[i0..i1] {
            power += v * v;
        }
        let amp = power.sqrt();
        let db = 20.0 * (amp.max(1e-7)).log10();
        out[b] = ((db - db_floor) / db_range).clamp(0.0, 1.0);
    }
    out
}

/// Map a normalized magnitude (0..1) to the classic SDR waterfall gradient:
/// dark blue → blue → cyan → green → yellow → orange → red → white.
pub(crate) fn waterfall_color(t: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, (u8, u8, u8)); 9] = [
        (0.00, (6, 8, 45)),      // near-black deep blue (noise floor)
        (0.14, (16, 42, 135)),   // blue
        (0.30, (0, 120, 190)),   // cyan-blue
        (0.45, (0, 190, 120)),   // green
        (0.60, (170, 215, 40)),  // yellow-green
        (0.74, (245, 220, 45)),  // yellow
        (0.85, (245, 140, 30)),  // orange
        (0.94, (225, 45, 35)),   // red
        (1.00, (255, 255, 255)), // white (peaks)
    ];
    let t = t.clamp(0.0, 1.0);
    for i in 1..STOPS.len() {
        if t <= STOPS[i].0 {
            let (lo, hi) = (STOPS[i - 1].0, STOPS[i].0);
            let k = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
            let (r1, g1, b1) = STOPS[i - 1].1;
            let (r2, g2, b2) = STOPS[i].1;
            let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
            return (lerp(r1, r2), lerp(g1, g2), lerp(b1, b2));
        }
    }
    STOPS[STOPS.len() - 1].1
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
        let spec_fft = planner.plan_fft_forward(SPEC_FFT_SIZE);
        let spec_window: Vec<f32> = (0..SPEC_FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (SPEC_FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos()
            })
            .collect();
        let spec_window_sum: f32 = spec_window.iter().sum();
        Self {
            fft,
            window,
            window_sum,
            samples: vec![0.0; FFT_SIZE.max(WAVE_SIZE)],
            spectrum_buf: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            mags: vec![0.0; FFT_SIZE / 2],
            spec_fft,
            spec_window,
            spec_window_sum,
            spec_samples: vec![0.0; SPEC_FFT_SIZE],
            spec_buf: vec![Complex32::new(0.0, 0.0); SPEC_FFT_SIZE],
            spec_mags: vec![0.0; SPEC_FFT_SIZE / 2],
            smoothed_bars: Vec::new(),
            scroll_wave: VecDeque::new(),
            spectrogram_cols: VecDeque::new(),
            waterfall_rows: VecDeque::new(),
            waterfall_img: vec![0; WATERFALL_BINS * WATERFALL_ROWS * 3],
            waterfall_last_secs: None,
            last_consumed_for_scroll: 0,
            stereo_samples: Vec::new(),
            spectrum_3d_rows: VecDeque::new(),
            last_waveform: Vec::new(),
            last_lissajous: Vec::new(),
            cassette_phase: 0.0,
            last_consumed_for_cassette: 0,
            mirror_peaks: Vec::new(),
            last_consumed_for_mirror: 0,
            vu_peak_hold: [0.0, 0.0],
            last_consumed_for_vu: 0,
            plasma_phase: 0.0,
            plasma_hue: 0.0,
            last_consumed_for_plasma: 0,
            mode: VisMode::Spectrum,
        }
    }

    /// Seconds of audio played since `watermark` was last updated, syncing the
    /// watermark. Returns 0 (and just resyncs) when playback is paused, so any
    /// animation or peak decay driven by this freezes cleanly on pause. This is
    /// the shared core of the cassette spindle, plasma, and peak-hold timing.
    fn advance_dt(watermark: &mut u64, tap: &SampleBuffer, active: bool) -> f32 {
        let consumed = tap.samples_consumed();
        let prev = *watermark;
        *watermark = consumed;
        if !active {
            return 0.0;
        }
        let channels = tap.channels().max(1) as u64;
        let sr = tap.sample_rate().max(1) as f32;
        let new_frames = consumed.saturating_sub(prev) / channels;
        new_frames as f32 / sr
    }

    /// Advance the cassette spindle phase based on samples played since the
    /// last call, returning the current angle in radians. Holds steady when
    /// playback is paused.
    pub fn cassette_phase(&mut self, tap: &SampleBuffer, active: bool) -> f32 {
        let dt = Self::advance_dt(&mut self.last_consumed_for_cassette, tap, active);
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

    /// Fill `spec_mags` from the high-resolution FFT over the newest
    /// `SPEC_FFT_SIZE` samples. Used by the time-frequency displays.
    fn compute_spec_fft(&mut self, tap: &SampleBuffer) {
        if self.spec_samples.len() < SPEC_FFT_SIZE {
            self.spec_samples.resize(SPEC_FFT_SIZE, 0.0);
        }
        let _ = tap.latest_mono(&mut self.spec_samples[..SPEC_FFT_SIZE]);
        for i in 0..SPEC_FFT_SIZE {
            let s = self.spec_samples[i] * self.spec_window[i];
            self.spec_buf[i] = Complex32::new(s, 0.0);
        }
        self.spec_fft.process(&mut self.spec_buf);
        let half = SPEC_FFT_SIZE / 2;
        let scale = 2.0 / self.spec_window_sum;
        for i in 0..half {
            let c = self.spec_buf[i];
            self.spec_mags[i] = (c.re * c.re + c.im * c.im).sqrt() * scale;
        }
    }

    /// Reduce the half-spectrum into `bins` log-spaced bands, returning each as
    /// a normalized 0..1 value mapped from `db_floor..(db_floor + db_range)` dBFS.
    fn log_bin_db(&self, bins: usize, sample_rate: u32, db_floor: f32, db_range: f32) -> Vec<f32> {
        log_bin_db_from(&self.mags, bins, sample_rate, db_floor, db_range)
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
            // Hi-res FFT for finer low-frequency detail than the bar displays.
            self.compute_spec_fft(tap);
            // Slightly tighter dynamic range than the bars so colors saturate nicely.
            let col = log_bin_db_from(&self.spec_mags, height, sample_rate, -70.0, 65.0);
            self.spectrogram_cols.push_back(col);
        }
        let cap = width.max(2);
        while self.spectrogram_cols.len() > cap {
            self.spectrogram_cols.pop_front();
        }
        self.spectrogram_cols.iter().cloned().collect()
    }

    /// Advance the waterfall: append new FFT rows for the time elapsed since the
    /// last one (steady ~30 rows/s, independent of redraw rate) and rebuild the
    /// RGB image. Returns the image dimensions `(width, height)` in pixels; the
    /// pixels themselves are read via [`Self::waterfall_pixels`]. `now_secs` is
    /// the platform wall clock.
    pub fn waterfall(
        &mut self,
        tap: &SampleBuffer,
        sample_rate: u32,
        active: bool,
        now_secs: f64,
    ) -> (u32, u32) {
        match self.waterfall_last_secs {
            None => self.waterfall_last_secs = Some(now_secs),
            Some(last) if active => {
                // Add one row per elapsed WATERFALL_ROW_DT, capped so a long
                // stall (or a clock jump) can't spew a huge burst of rows.
                let mut t = last;
                let mut added = 0;
                while now_secs - t >= WATERFALL_ROW_DT && added < WATERFALL_ROWS {
                    // Hi-res FFT for finer low-frequency detail.
                    self.compute_spec_fft(tap);
                    let row = log_bin_db_from(
                        &self.spec_mags,
                        WATERFALL_BINS,
                        sample_rate,
                        WATERFALL_DB_FLOOR,
                        WATERFALL_DB_RANGE,
                    );
                    self.waterfall_rows.push_back(row);
                    t += WATERFALL_ROW_DT;
                    added += 1;
                }
                // If we fell far behind, resync rather than chase forever.
                self.waterfall_last_secs = Some(if added == WATERFALL_ROWS { now_secs } else { t });
                while self.waterfall_rows.len() > WATERFALL_ROWS {
                    self.waterfall_rows.pop_front();
                }
            }
            // Paused: hold the timeline so it doesn't lurch on resume.
            Some(_) => self.waterfall_last_secs = Some(now_secs),
        }
        self.rebuild_waterfall_image();
        (WATERFALL_BINS as u32, WATERFALL_ROWS as u32)
    }

    /// The current waterfall image as row-major RGB8, `WATERFALL_BINS` wide.
    pub fn waterfall_pixels(&self) -> &[u8] {
        &self.waterfall_img
    }

    /// The raw magnitude rows (oldest first), for the cell-based fallback used
    /// on terminals without pixel graphics.
    pub fn waterfall_rows(&self) -> &VecDeque<Vec<f32>> {
        &self.waterfall_rows
    }

    /// Repaint `waterfall_img` from `waterfall_rows`, newest row on top. Rows
    /// not yet filled show the palette's floor color (dark blue).
    fn rebuild_waterfall_image(&mut self) {
        let rows = &self.waterfall_rows;
        let n = rows.len();
        for y in 0..WATERFALL_ROWS {
            // y = 0 is the top = newest row.
            let row = if y < n { Some(&rows[n - 1 - y]) } else { None };
            for x in 0..WATERFALL_BINS {
                let mag = row.and_then(|r| r.get(x)).copied().unwrap_or(0.0);
                let (r, g, b) = waterfall_color(mag);
                let idx = (y * WATERFALL_BINS + x) * 3;
                self.waterfall_img[idx] = r;
                self.waterfall_img[idx + 1] = g;
                self.waterfall_img[idx + 2] = b;
            }
        }
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
            // Hi-res FFT for finer low-frequency detail (like the spectrogram
            // and waterfall); this scrolling view absorbs the longer window.
            self.compute_spec_fft(tap);
            let col = log_bin_db_from(&self.spec_mags, bins, sample_rate, -75.0, 70.0);
            self.spectrum_3d_rows.push_back(col);
        }
        let cap = depth.max(2);
        while self.spectrum_3d_rows.len() > cap {
            self.spectrum_3d_rows.pop_front();
        }
        self.spectrum_3d_rows.iter().cloned().collect()
    }

    /// Smoothed spectrum bars plus falling peak-hold caps, for the mirrored
    /// visualizer. Caps snap up to the live bar then sag at a fixed rate.
    pub fn mirror_bars(
        &mut self,
        tap: &SampleBuffer,
        bins: usize,
        sample_rate: u32,
        active: bool,
    ) -> (Vec<f32>, Vec<f32>) {
        let bars = self.spectrum(tap, bins, sample_rate, active);
        if self.mirror_peaks.len() != bins {
            self.mirror_peaks = bars.clone();
        }
        let dt = Self::advance_dt(&mut self.last_consumed_for_mirror, tap, active);
        let decay = dt * 0.9;
        for (peak, &bar) in self.mirror_peaks.iter_mut().zip(bars.iter()) {
            *peak = if bar >= *peak {
                bar
            } else {
                (*peak - decay).max(bar)
            };
        }
        (bars, self.mirror_peaks.clone())
    }

    /// Stereo RMS / peak levels and correlation for the VU meters, with falling
    /// peak-hold markers (frozen on pause).
    pub fn levels(&mut self, tap: &SampleBuffer, active: bool) -> Levels {
        const N: usize = 2048;
        if self.stereo_samples.len() < N {
            self.stereo_samples.resize(N, (0.0, 0.0));
        }
        let frames = tap.latest_stereo(&mut self.stereo_samples[..N]);
        let used = frames.min(N);
        let measured = compute_levels(&self.stereo_samples[..used]);
        let dt = Self::advance_dt(&mut self.last_consumed_for_vu, tap, active);
        let decay = dt * 0.6;
        for c in 0..2 {
            if measured.peak[c] >= self.vu_peak_hold[c] {
                self.vu_peak_hold[c] = measured.peak[c];
            } else {
                self.vu_peak_hold[c] = (self.vu_peak_hold[c] - decay).max(measured.peak[c]);
            }
        }
        Levels {
            rms: measured.rms,
            peak: self.vu_peak_hold,
            correlation: measured.correlation,
        }
    }

    /// Advance the plasma field and return its drift `phase`, a slowly rotating
    /// `hue` offset (0..1) for morphing the palette, and coarse bass/mid/treble
    /// energies. Phase drift speeds up with overall energy; all freeze on pause.
    pub fn plasma_state(
        &mut self,
        tap: &SampleBuffer,
        sample_rate: u32,
        active: bool,
    ) -> (f32, f32, [f32; 3]) {
        let bands = if active {
            self.compute_fft(tap);
            let b = self.log_bin_db(3, sample_rate, -70.0, 65.0);
            [b[0], b[1], b[2]]
        } else {
            [0.0, 0.0, 0.0]
        };
        let dt = Self::advance_dt(&mut self.last_consumed_for_plasma, tap, active);
        let energy = (bands[0] + bands[1] + bands[2]) / 3.0;
        // The field churns much faster when the music is loud, so beats visibly
        // drive the motion rather than just a steady drift.
        self.plasma_phase += dt * (0.4 + energy * 7.0);
        let tau = std::f32::consts::TAU;
        if self.plasma_phase > tau * 1024.0 {
            self.plasma_phase = self.plasma_phase.rem_euclid(tau);
        }
        // Rotate the palette roughly once every ~40 seconds of playback.
        self.plasma_hue = (self.plasma_hue + dt * 0.025).rem_euclid(1.0);
        (self.plasma_phase, self.plasma_hue, bands)
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.cycle();
    }
    pub fn toggle_mode_back(&mut self) {
        self.mode = self.mode.cycle_back();
    }
}

/// Per-channel RMS and peak plus inter-channel correlation for a block of
/// stereo frames. Pure (no `self`) so it can be unit-tested directly.
fn compute_levels(frames: &[(f32, f32)]) -> Levels {
    if frames.is_empty() {
        return Levels::default();
    }
    let n = frames.len() as f32;
    let mut sum_l2 = 0.0f32;
    let mut sum_r2 = 0.0f32;
    let mut sum_lr = 0.0f32;
    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    for &(l, r) in frames {
        sum_l2 += l * l;
        sum_r2 += r * r;
        sum_lr += l * r;
        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
    }
    let denom = (sum_l2 * sum_r2).sqrt();
    let correlation = if denom > 1e-9 {
        (sum_lr / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    Levels {
        rms: [(sum_l2 / n).sqrt().min(1.0), (sum_r2 / n).sqrt().min(1.0)],
        peak: [peak_l.min(1.0), peak_r.min(1.0)],
        correlation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_in_phase_is_one() {
        let frames: Vec<(f32, f32)> = (0..512)
            .map(|i| {
                let s = (i as f32 * 0.1).sin();
                (s, s)
            })
            .collect();
        let lv = compute_levels(&frames);
        assert!((lv.correlation - 1.0).abs() < 1e-3, "{}", lv.correlation);
    }

    #[test]
    fn correlation_out_of_phase_is_minus_one() {
        let frames: Vec<(f32, f32)> = (0..512)
            .map(|i| {
                let s = (i as f32 * 0.1).sin();
                (s, -s)
            })
            .collect();
        let lv = compute_levels(&frames);
        assert!((lv.correlation + 1.0).abs() < 1e-3, "{}", lv.correlation);
    }

    #[test]
    fn silence_has_zero_levels() {
        let lv = compute_levels(&[(0.0, 0.0); 256]);
        assert_eq!(lv.rms, [0.0, 0.0]);
        assert_eq!(lv.peak, [0.0, 0.0]);
        assert_eq!(lv.correlation, 0.0);
    }

    #[test]
    fn rms_and_peak_of_full_scale_tone() {
        let frames: Vec<(f32, f32)> = (0..1024)
            .map(|i| {
                let s = (i as f32 * 0.2).sin();
                (s, s * 0.5)
            })
            .collect();
        let lv = compute_levels(&frames);
        // RMS of a unit sine ≈ 0.707; right channel is half amplitude.
        assert!((lv.rms[0] - 0.707).abs() < 0.05, "{}", lv.rms[0]);
        assert!((lv.rms[1] - 0.354).abs() < 0.05, "{}", lv.rms[1]);
        assert!(lv.peak[0] <= 1.0 && lv.peak[0] > 0.9);
    }
}
