use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg::util::frame::video::Video;

/// A decoded video frame in RGB8, ready to be wrapped in an `image::DynamicImage`.
pub struct VideoFrame {
    pub pts_secs: f64,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

struct SharedQueue {
    frames: VecDeque<VideoFrame>,
    seek_target_ms: Option<i64>,
    eof: bool,
}

/// Threaded video decoder. Frames are decoded ahead of time and queued in PTS
/// order; the UI samples them by audio time.
pub struct VideoStream {
    inner: Arc<Mutex<SharedQueue>>,
    stop: Arc<AtomicBool>,
    last_drawn_pts_ms: AtomicI64,
    handle: Option<JoinHandle<()>>,
    pub width: u32,
    pub height: u32,
    #[allow(dead_code)]
    pub frame_rate: f64,
    #[allow(dead_code)]
    pub duration: Option<Duration>,
    #[allow(dead_code)]
    pub path: PathBuf,
}

const MAX_QUEUED_FRAMES: usize = 12;

impl VideoStream {
    pub fn open(path: &Path) -> Result<Self> {
        ffmpeg::init().ok();
        // Mute libav warnings — they corrupt the TUI when written to stderr.
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Fatal);
        let ictx = ffmpeg::format::input(&path.to_path_buf())
            .with_context(|| format!("opening video {}", path.display()))?;
        let stream = ictx
            .streams()
            .best(Type::Video)
            .context("no video stream in file")?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let avg_frame_rate = stream.avg_frame_rate();
        let frame_rate = if avg_frame_rate.denominator() != 0 {
            avg_frame_rate.numerator() as f64 / avg_frame_rate.denominator() as f64
        } else {
            25.0
        };
        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = codec_ctx.decoder().video()?;
        let width = decoder.width();
        let height = decoder.height();
        let format = decoder.format();

        let duration = {
            let dur = stream.duration();
            if dur > 0 {
                Some(Duration::from_secs_f64(
                    dur as f64 * time_base.numerator() as f64 / time_base.denominator() as f64,
                ))
            } else {
                let d = ictx.duration();
                if d > 0 {
                    Some(Duration::from_secs_f64(d as f64 / ffmpeg::ffi::AV_TIME_BASE as f64))
                } else {
                    None
                }
            }
        };

        let inner = Arc::new(Mutex::new(SharedQueue {
            frames: VecDeque::new(),
            seek_target_ms: None,
            eof: false,
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let inner_t = Arc::clone(&inner);
        let stop_t = Arc::clone(&stop);
        let path_t = path.to_path_buf();

        let handle = thread::Builder::new()
            .name("sparkplayer-video".into())
            .spawn(move || {
                if let Err(e) = decode_loop(
                    path_t,
                    stream_index,
                    width,
                    height,
                    format,
                    time_base,
                    inner_t,
                    stop_t,
                ) {
                    eprintln!("video decode thread exited: {e}");
                }
            })
            .context("spawning video decoder thread")?;

        // Move the decoder ownership into the thread; reopen there. The above
        // decoder/ictx exist only to probe metadata.
        drop(decoder);
        drop(ictx);

        Ok(Self {
            inner,
            stop,
            last_drawn_pts_ms: AtomicI64::new(i64::MIN),
            handle: Some(handle),
            width,
            height,
            frame_rate,
            duration,
            path: path.to_path_buf(),
        })
    }

    /// Tell the decoder thread to seek to `target` and discard buffered frames.
    pub fn seek(&self, target: Duration) {
        if let Ok(mut q) = self.inner.lock() {
            q.frames.clear();
            q.seek_target_ms = Some(target.as_millis() as i64);
            q.eof = false;
        }
        self.last_drawn_pts_ms.store(i64::MIN, Ordering::Relaxed);
    }

    /// Pop the most-recent frame whose PTS is at or before `target_secs`.
    /// Older frames are discarded. Returns None if no such frame is ready
    /// (decoder still warming up) or if the same frame was already returned.
    pub fn frame_at(&self, target_secs: f64) -> Option<VideoFrame> {
        let target_ms = (target_secs * 1000.0) as i64;
        let mut q = self.inner.lock().ok()?;
        // Drop frames that are stale (older than target) but keep the most
        // recent stale one in case the queue tail is still in the future.
        let mut selected: Option<VideoFrame> = None;
        while let Some(front) = q.frames.front() {
            let front_ms = (front.pts_secs * 1000.0) as i64;
            if front_ms <= target_ms {
                selected = q.frames.pop_front();
            } else {
                break;
            }
        }
        let frame = selected?;
        let frame_ms = (frame.pts_secs * 1000.0) as i64;
        let last = self.last_drawn_pts_ms.load(Ordering::Relaxed);
        if frame_ms == last {
            return None;
        }
        self.last_drawn_pts_ms.store(frame_ms, Ordering::Relaxed);
        Some(frame)
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the decoder thread by clearing the queue so the bounded-queue
        // sleep loop exits promptly.
        if let Ok(mut q) = self.inner.lock() {
            q.frames.clear();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn decode_loop(
    path: PathBuf,
    stream_index: usize,
    width: u32,
    height: u32,
    src_format: Pixel,
    time_base: ffmpeg::Rational,
    inner: Arc<Mutex<SharedQueue>>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut ictx = ffmpeg::format::input(&path)?;
    let codec_ctx = ffmpeg::codec::context::Context::from_parameters(
        ictx.stream(stream_index).context("stream gone")?.parameters(),
    )?;
    let mut decoder = codec_ctx.decoder().video()?;
    let mut scaler = Scaler::get(
        src_format,
        width,
        height,
        Pixel::RGB24,
        width,
        height,
        Flags::BILINEAR,
    )?;

    let tb_num = time_base.numerator() as f64;
    let tb_den = time_base.denominator() as f64;
    let pts_to_secs = |pts: i64| -> f64 { pts as f64 * tb_num / tb_den };

    let mut packet = ffmpeg::Packet::empty();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Honor a pending seek request from the UI thread.
        let seek_to = inner.lock().ok().and_then(|mut q| q.seek_target_ms.take());
        if let Some(ms) = seek_to {
            let ts = (ms as i64 * ffmpeg::ffi::AV_TIME_BASE as i64) / 1000;
            let _ = ictx.seek(ts, ..ts);
            decoder.flush();
        }

        match packet.read(&mut ictx) {
            Ok(()) => {
                if packet.stream() != stream_index {
                    continue;
                }
                if decoder.send_packet(&packet).is_err() {
                    continue;
                }
                let mut decoded = Video::empty();
                while decoder.receive_frame(&mut decoded).is_ok() {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let pts = decoded.pts().unwrap_or(0);
                    let secs = pts_to_secs(pts);

                    let mut rgb = Video::empty();
                    if scaler.run(&decoded, &mut rgb).is_err() {
                        continue;
                    }
                    let stride = rgb.stride(0);
                    let plane = rgb.data(0);
                    let w = rgb.width() as usize;
                    let h = rgb.height() as usize;
                    let mut buf = Vec::with_capacity(w * h * 3);
                    for y in 0..h {
                        let row_start = y * stride;
                        buf.extend_from_slice(&plane[row_start..row_start + w * 3]);
                    }

                    let mut pending = Some(VideoFrame {
                        pts_secs: secs,
                        width: w as u32,
                        height: h as u32,
                        data: buf,
                    });

                    // Backpressure: wait until queue has room.
                    while pending.is_some() {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        {
                            let mut q = match inner.lock() {
                                Ok(q) => q,
                                Err(_) => return Ok(()),
                            };
                            if q.seek_target_ms.is_some() {
                                // Stale frame from before the seek — drop it.
                                drop(pending.take());
                                break;
                            }
                            if q.frames.len() < MAX_QUEUED_FRAMES {
                                if let Some(f) = pending.take() {
                                    q.frames.push_back(f);
                                }
                            }
                        }
                        if pending.is_some() {
                            thread::sleep(Duration::from_millis(8));
                        }
                    }
                }
            }
            Err(_) => {
                // EOF or read error — flush decoder and mark eof.
                let _ = decoder.send_eof();
                let mut decoded = Video::empty();
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let pts = decoded.pts().unwrap_or(0);
                    let secs = pts_to_secs(pts);
                    let mut rgb = Video::empty();
                    if scaler.run(&decoded, &mut rgb).is_err() {
                        continue;
                    }
                    let stride = rgb.stride(0);
                    let plane = rgb.data(0);
                    let w = rgb.width() as usize;
                    let h = rgb.height() as usize;
                    let mut buf = Vec::with_capacity(w * h * 3);
                    for y in 0..h {
                        let row_start = y * stride;
                        buf.extend_from_slice(&plane[row_start..row_start + w * 3]);
                    }
                    if let Ok(mut q) = inner.lock() {
                        q.frames.push_back(VideoFrame {
                            pts_secs: secs,
                            width: w as u32,
                            height: h as u32,
                            data: buf,
                        });
                    }
                }
                if let Ok(mut q) = inner.lock() {
                    q.eof = true;
                }
                // Idle wait — keep thread alive until dropped, occasionally
                // checking for a seek request that would resume decoding.
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                    let resume = inner
                        .lock()
                        .ok()
                        .and_then(|q| q.seek_target_ms);
                    if resume.is_some() {
                        break;
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                // Reopen the input for the seek.
                ictx = ffmpeg::format::input(&path)?;
            }
        }
    }
    Ok(())
}

