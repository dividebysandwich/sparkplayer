//! Web Audio backend. A single `HTMLMediaElement` (a `<video>`, shared with the
//! video overlay) decodes and plays the media; its output is routed through
//! `MediaElementAudioSourceNode → AnalyserNode → GainNode → destination`. Each
//! frame we copy the analyser's time-domain data into the shared `SampleBuffer`
//! so every visualizer mode works unchanged.

use std::time::Duration;

use wasm_bindgen::JsValue;
use web_sys::{
    AnalyserNode, AudioContext, AudioContextState, GainNode, HtmlVideoElement,
    MediaElementAudioSourceNode,
};

use sparkplayer_core::backend::AudioBackend;
use sparkplayer_core::{SampleBuffer, TrackRef};

pub struct WebAudioBackend {
    ctx: AudioContext,
    element: HtmlVideoElement,
    analyser: AnalyserNode,
    gain: GainNode,
    tap: SampleBuffer,
    volume: f32,
    scratch: Vec<f32>,
    // Keep the source node alive (dropping it would tear down the graph edge).
    _source: MediaElementAudioSourceNode,
}

impl WebAudioBackend {
    pub fn new(element: HtmlVideoElement) -> Result<Self, JsValue> {
        let ctx = AudioContext::new()?;
        let source = ctx.create_media_element_source(&element)?;
        let analyser = ctx.create_analyser()?;
        analyser.set_fft_size(2048);
        let gain = ctx.create_gain()?;
        // source → analyser → gain → destination
        source.connect_with_audio_node(&analyser)?;
        analyser.connect_with_audio_node(&gain)?;
        gain.connect_with_audio_node(&ctx.destination())?;
        gain.gain().set_value(0.8);

        let fft = analyser.fft_size() as usize;
        Ok(Self {
            ctx,
            element,
            analyser,
            gain,
            tap: SampleBuffer::new(),
            volume: 0.8,
            scratch: vec![0.0; fft],
            _source: source,
        })
    }
}

impl AudioBackend for WebAudioBackend {
    fn play(&mut self, source: &TrackRef) -> anyhow::Result<Option<Duration>> {
        let url = source.locator();
        // Allow the AnalyserNode to read cross-origin media (the host must send
        // CORS headers). Without this, getFloatTimeDomainData yields silence.
        self.element.set_cross_origin(Some("anonymous"));
        self.element.set_src(&url);
        self.tap.reset();
        let _ = self.element.play();
        Ok(None)
    }

    fn toggle_pause(&self) {
        if self.element.paused() {
            let _ = self.element.play();
        } else {
            let _ = self.element.pause();
        }
    }

    fn is_paused(&self) -> bool {
        self.element.paused()
    }

    fn is_finished(&self) -> bool {
        self.element.ended()
    }

    fn stop(&mut self) {
        let _ = self.element.pause();
        self.element.set_src("");
        self.tap.reset();
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        // Route gain through the GainNode (the element's own volume caps at
        // 1.0, but we want to allow the existing 1.5 boost).
        self.gain.gain().set_value(self.volume);
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn seek_relative(&mut self, delta: f64, total: Option<Duration>) -> anyhow::Result<()> {
        let cur = self.element.current_time();
        let mut target = (cur + delta).max(0.0);
        if let Some(t) = total {
            let max = t.as_secs_f64();
            if max > 0.0 && target > max - 0.05 {
                target = (max - 0.05).max(0.0);
            }
        }
        self.element.set_current_time(target);
        Ok(())
    }

    fn position(&self) -> Duration {
        Duration::from_secs_f64(self.element.current_time().max(0.0))
    }

    fn tap(&self) -> &SampleBuffer {
        &self.tap
    }

    fn output_buffer_latency(&self) -> Duration {
        Duration::ZERO
    }

    fn pump(&mut self) {
        // Analyser data is the post-mix mono signal.
        self.tap.set_format(1, self.ctx.sample_rate() as u32);
        self.analyser.get_float_time_domain_data(&mut self.scratch);
        self.tap.push_slice(&self.scratch);
    }

    fn on_user_gesture(&mut self) {
        if self.ctx.state() == AudioContextState::Suspended {
            let _ = self.ctx.resume();
        }
    }

    fn duration(&self) -> Option<Duration> {
        let d = self.element.duration();
        if d.is_finite() && d > 0.0 {
            Some(Duration::from_secs_f64(d))
        } else {
            None
        }
    }
}
