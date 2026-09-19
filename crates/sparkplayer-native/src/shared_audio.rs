//! Optional shared-memory audio bridge for external visualizers.
//!
//! The bridge is deliberately best-effort: callers should disable it if
//! `open` fails rather than making player startup depend on an external
//! consumer. Samples are published as interleaved stereo frames into a fixed
//! ring buffer. `write_frame` is the release/acquire publication cursor;
//! `generation` only guards stream and format transitions.

use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, Result};

const MAGIC: u32 = u32::from_le_bytes(*b"SPRK");
const VERSION: u32 = 2;
const OUTPUT_CHANNELS: u32 = 2;
const DEFAULT_CAPACITY_FRAMES: u32 = 48_000 * 10;

const METADATA_LEN: usize = 128;
const _: () = assert!(size_of::<SparkAudioHeader>() == 600);

pub struct SharedAudioControl {
    pub playback: Option<bool>,
    pub next_track: bool,
    pub previous_track: bool,
    pub visualizer_delta: i32,
}

#[repr(C)]
struct SparkAudioHeader {
    magic: u32,
    version: u32,
    header_size: u32,
    capacity_frames: u32,
    channels: u32,
    sample_rate: u32,
    write_frame: u64,
    total_frames: u64,
    generation: u64,
    active: u32,
    transport_sequence: u32,
    transport_state: u32,
    visualizer_sequence: u32,
    visualizer_delta: i32,
    track_sequence: u32,
    track_action: u32,
    reserved: u32,
    metadata_sequence: u32,
    title: [u8; METADATA_LEN],
    artist: [u8; METADATA_LEN],
    album: [u8; METADATA_LEN],
    info: [u8; METADATA_LEN],
}

/// Per-source channel assembler. This stays owned by the playback source so the
/// audio callback does not need to lock shared state for every sample.
pub(crate) struct SharedAudioFramePacker {
    source_channels: usize,
    pending_frame: Vec<f32>,
}

impl SharedAudioFramePacker {
    pub(crate) fn new(channels: u16) -> Self {
        let source_channels = channels.max(1) as usize;
        Self {
            source_channels,
            pending_frame: Vec::with_capacity(source_channels.min(8)),
        }
    }

    pub(crate) fn push_sample(&mut self, sample: f32) -> Option<(f32, f32)> {
        self.pending_frame.push(sample);
        if self.pending_frame.len() < self.source_channels {
            return None;
        }

        let frame = if self.source_channels == 1 {
            (self.pending_frame[0], self.pending_frame[0])
        } else {
            (self.pending_frame[0], self.pending_frame[1])
        };
        self.pending_frame.clear();
        Some(frame)
    }
}

#[derive(Clone)]
pub struct SharedAudioWriter {
    inner: Arc<SharedAudioInner>,
}

struct SharedAudioInner {
    _mapping: PlatformMapping,
    view: NonNull<u8>,
    capacity_frames: usize,
    next_frame: AtomicU64,
    generation: AtomicU64,
    last_transport_control: AtomicU32,
    last_visualizer_control: AtomicU32,
    last_track_control: AtomicU32,
}

unsafe impl Send for SharedAudioInner {}
unsafe impl Sync for SharedAudioInner {}

impl SharedAudioWriter {
    fn new(mapping: PlatformMapping) -> Self {
        let view = mapping.view();
        let capacity_frames = DEFAULT_CAPACITY_FRAMES as usize;
        let inner = SharedAudioInner {
            _mapping: mapping,
            view,
            capacity_frames,
            next_frame: AtomicU64::new(0),
            generation: AtomicU64::new(2),
            last_transport_control: AtomicU32::new(0),
            last_visualizer_control: AtomicU32::new(0),
            last_track_control: AtomicU32::new(0),
        };
        inner.initialize_header();
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Start a new stream epoch. Called by `TapSource` on the playback thread
    /// when its first sample actually reaches the output.
    pub fn begin_stream(&self, sample_rate: u32) {
        self.inner.begin_stream(sample_rate.max(1));
    }

    pub fn push_frame(&self, left: f32, right: f32) {
        self.inner.push_frame(left, right);
    }

    pub fn end_stream(&self) {
        self.inner.end_stream();
    }

    pub fn poll_control(&self) -> SharedAudioControl {
        self.inner.poll_control()
    }

    pub fn publish_metadata(&self, title: &str, artist: &str, album: &str, info: &str) {
        self.inner.publish_metadata(title, artist, album, info);
    }
}

impl SharedAudioInner {
    fn header(&self) -> *mut SparkAudioHeader {
        self.view.as_ptr().cast::<SparkAudioHeader>()
    }

    fn sample_ptr(&self) -> *mut f32 {
        unsafe {
            self.view
                .as_ptr()
                .add(size_of::<SparkAudioHeader>())
                .cast::<f32>()
        }
    }

    fn header_u32(&self, field: *mut u32) -> &AtomicU32 {
        // Header fields are naturally aligned and exclusively accessed with
        // matching-width atomics once the mapping has been initialized.
        unsafe { &*field.cast::<AtomicU32>() }
    }

    fn header_u64(&self, field: *mut u64) -> &AtomicU64 {
        unsafe { &*field.cast::<AtomicU64>() }
    }

    fn initialize_header(&self) {
        let header = self.header();
        let magic = self.header_u32(unsafe { std::ptr::addr_of_mut!((*header).magic) });
        let generation = self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).generation) });
        let previous_generation = if magic.load(Ordering::Acquire) == MAGIC {
            generation.load(Ordering::Acquire) & !1
        } else {
            0
        };
        let next_generation = previous_generation.wrapping_add(2).max(2);
        self.generation.store(next_generation, Ordering::Relaxed);

        // Hide the header while replacing a stale writer's session. Publishing
        // magic last prevents a reader from accepting a partially initialized
        // header even when the named mapping survived the previous process.
        magic.store(0, Ordering::Release);
        generation.store(next_generation.wrapping_sub(1), Ordering::Release);
        unsafe {
            std::ptr::addr_of_mut!((*header).version).write(VERSION);
            std::ptr::addr_of_mut!((*header).header_size)
                .write(size_of::<SparkAudioHeader>() as u32);
            std::ptr::addr_of_mut!((*header).capacity_frames).write(self.capacity_frames as u32);
            std::ptr::addr_of_mut!((*header).channels).write(OUTPUT_CHANNELS);
            std::ptr::addr_of_mut!((*header).sample_rate).write(44_100);
            std::ptr::addr_of_mut!((*header).write_frame).write(0);
            std::ptr::addr_of_mut!((*header).total_frames).write(0);
            std::ptr::addr_of_mut!((*header).active).write(0);
            std::ptr::addr_of_mut!((*header).transport_sequence).write(0);
            std::ptr::addr_of_mut!((*header).transport_state).write(0);
            std::ptr::addr_of_mut!((*header).visualizer_sequence).write(0);
            std::ptr::addr_of_mut!((*header).visualizer_delta).write(0);
            std::ptr::addr_of_mut!((*header).track_sequence).write(0);
            std::ptr::addr_of_mut!((*header).track_action).write(0);
            std::ptr::addr_of_mut!((*header).reserved).write(0);
            std::ptr::addr_of_mut!((*header).metadata_sequence).write(0);
            std::ptr::addr_of_mut!((*header).title).write([0; METADATA_LEN]);
            std::ptr::addr_of_mut!((*header).artist).write([0; METADATA_LEN]);
            std::ptr::addr_of_mut!((*header).album).write([0; METADATA_LEN]);
            std::ptr::addr_of_mut!((*header).info).write([0; METADATA_LEN]);
        }
        generation.store(next_generation, Ordering::Release);
        magic.store(MAGIC, Ordering::Release);
    }

    fn begin_stream(&self, sample_rate: u32) {
        let next_generation = self
            .generation
            .fetch_add(2, Ordering::AcqRel)
            .wrapping_add(2);
        self.next_frame.store(0, Ordering::Relaxed);
        let header = self.header();
        let generation = self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).generation) });
        generation.store(next_generation.wrapping_sub(1), Ordering::Release);
        self.header_u32(unsafe { std::ptr::addr_of_mut!((*header).sample_rate) })
            .store(sample_rate, Ordering::Relaxed);
        self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).total_frames) })
            .store(0, Ordering::Relaxed);
        self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).write_frame) })
            .store(0, Ordering::Release);
        self.header_u32(unsafe { std::ptr::addr_of_mut!((*header).active) })
            .store(1, Ordering::Relaxed);
        generation.store(next_generation, Ordering::Release);
    }

    fn end_stream(&self) {
        let header = self.header();
        self.header_u32(unsafe { std::ptr::addr_of_mut!((*header).active) })
            .store(0, Ordering::Release);
    }

    fn push_frame(&self, left: f32, right: f32) {
        let frame = self.next_frame.fetch_add(1, Ordering::Relaxed);
        let frame_index = (frame as usize) % self.capacity_frames;
        let sample_offset = frame_index * OUTPUT_CHANNELS as usize;
        unsafe {
            let sample_ptr = self.sample_ptr();
            sample_ptr.add(sample_offset).write(left);
            sample_ptr.add(sample_offset + 1).write(right);
        }
        let total = frame.wrapping_add(1);
        let header = self.header();
        self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).total_frames) })
            .store(total, Ordering::Relaxed);
        // Publishing the cursor last makes both samples visible to an
        // acquire-loading reader on weakly ordered CPUs such as ARM.
        self.header_u64(unsafe { std::ptr::addr_of_mut!((*header).write_frame) })
            .store(total, Ordering::Release);
    }

    fn poll_control(&self) -> SharedAudioControl {
        let header = self.header();
        let transport_generation = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).transport_sequence) })
            .load(Ordering::Acquire);
        let transport_state = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).transport_state) })
            .load(Ordering::Relaxed);
        let visualizer_generation = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).visualizer_sequence) })
            .load(Ordering::Acquire);
        let visualizer_delta = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).visualizer_delta).cast::<u32>() })
            .load(Ordering::Relaxed) as i32;
        let track_sequence = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).track_sequence) })
            .load(Ordering::Acquire);
        let track_action = self
            .header_u32(unsafe { std::ptr::addr_of_mut!((*header).track_action) })
            .load(Ordering::Relaxed);

        let playback = if transport_generation
            != self
                .last_transport_control
                .swap(transport_generation, Ordering::AcqRel)
        {
            match transport_state {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            }
        } else {
            None
        };

        let delta = if visualizer_generation
            != self
                .last_visualizer_control
                .swap(visualizer_generation, Ordering::AcqRel)
        {
            visualizer_delta.clamp(-16, 16)
        } else {
            0
        };
        let track_changed = track_sequence
            != self
                .last_track_control
                .swap(track_sequence, Ordering::AcqRel);

        SharedAudioControl {
            playback,
            next_track: track_changed && track_action == 1,
            previous_track: track_changed && track_action == 2,
            visualizer_delta: delta,
        }
    }

    fn publish_metadata(&self, title: &str, artist: &str, album: &str, info: &str) {
        let header = self.header();
        let sequence =
            self.header_u32(unsafe { std::ptr::addr_of_mut!((*header).metadata_sequence) });
        let next = sequence.load(Ordering::Relaxed).wrapping_add(2) & !1;
        sequence.store(next.wrapping_sub(1), Ordering::Release);
        unsafe {
            write_text(std::ptr::addr_of_mut!((*header).title), title);
            write_text(std::ptr::addr_of_mut!((*header).artist), artist);
            write_text(std::ptr::addr_of_mut!((*header).album), album);
            write_text(std::ptr::addr_of_mut!((*header).info), info);
        }
        sequence.store(next, Ordering::Release);
    }
}

unsafe fn write_text(destination: *mut [u8; METADATA_LEN], value: &str) {
    let output = unsafe { &mut *destination };
    output.fill(0);
    let bytes = value.as_bytes();
    let mut count = bytes.len().min(METADATA_LEN - 1);
    while !value.is_char_boundary(count) {
        count -= 1;
    }
    output[..count].copy_from_slice(&bytes[..count]);
}

impl Drop for SharedAudioInner {
    fn drop(&mut self) {
        self.end_stream();
    }
}

pub fn open(name: &str) -> Result<SharedAudioWriter> {
    let mapping = PlatformMapping::open(name)
        .with_context(|| format!("opening shared audio stream {name}"))?;
    Ok(SharedAudioWriter::new(mapping))
}

struct PlatformMapping {
    imp: PlatformMappingImp,
}

impl PlatformMapping {
    fn open(name: &str) -> Result<Self> {
        Ok(Self {
            imp: PlatformMappingImp::open(name)?,
        })
    }

    fn view(&self) -> NonNull<u8> {
        self.imp.view()
    }
}

#[cfg(windows)]
struct PlatformMappingImp {
    mapping: windows_sys::Win32::Foundation::HANDLE,
    writer_mutex: windows_sys::Win32::Foundation::HANDLE,
    view: NonNull<u8>,
}

#[cfg(windows)]
impl PlatformMappingImp {
    fn open(name: &str) -> Result<Self> {
        use std::ptr;
        use windows_sys::Win32::Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE,
        };
        use windows_sys::Win32::System::Threading::CreateMutexW;

        let byte_len = shared_byte_len();
        let mapping_name = normalize_windows_name(name);
        let mut wide = mapping_name.encode_utf16().collect::<Vec<_>>();
        wide.push(0);

        let mut mutex_wide = format!("{mapping_name}.writer")
            .encode_utf16()
            .collect::<Vec<_>>();
        mutex_wide.push(0);
        let writer_mutex = unsafe { CreateMutexW(ptr::null(), 1, mutex_wide.as_ptr()) };
        if writer_mutex.is_null() {
            anyhow::bail!("CreateMutexW failed for {mapping_name}");
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(writer_mutex) };
            anyhow::bail!("shared audio stream {mapping_name} is already in use");
        }

        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                ptr::null(),
                PAGE_READWRITE,
                (byte_len as u64 >> 32) as u32,
                byte_len as u32,
                wide.as_ptr(),
            )
        };
        if mapping.is_null() {
            unsafe { CloseHandle(writer_mutex) };
            anyhow::bail!("CreateFileMappingW failed for {name}");
        }

        let view_address = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, byte_len) };
        let view = NonNull::new(view_address.Value.cast::<u8>()).ok_or_else(|| {
            unsafe {
                CloseHandle(mapping);
                windows_sys::Win32::System::Threading::ReleaseMutex(writer_mutex);
                CloseHandle(writer_mutex);
            }
            anyhow::anyhow!("MapViewOfFile failed for {name}")
        })?;

        Ok(Self {
            mapping,
            writer_mutex,
            view,
        })
    }

    fn view(&self) -> NonNull<u8> {
        self.view
    }
}

#[cfg(windows)]
impl Drop for PlatformMappingImp {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        use windows_sys::Win32::System::Threading::ReleaseMutex;

        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view.as_ptr().cast(),
            });
            CloseHandle(self.mapping);
            ReleaseMutex(self.writer_mutex);
            CloseHandle(self.writer_mutex);
        }
    }
}

#[cfg(windows)]
fn normalize_windows_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.eq_ignore_ascii_case("Local\\SparkPlayerAudio")
        || trimmed.eq_ignore_ascii_case("Local\\sparkplayer_audio")
    {
        return "Local\\SparkPlayerAudio".to_string();
    }
    if trimmed.starts_with("Local\\") || trimmed.starts_with("Global\\") {
        return trimmed.to_string();
    }

    let raw = if trimmed.is_empty() {
        "sparkplayer_audio"
    } else if let Some(rest) = trimmed.strip_prefix('/') {
        rest
    } else {
        trimmed
            .rsplit(['\\', '/', ':'])
            .next()
            .unwrap_or("sparkplayer_audio")
    };

    if raw.eq_ignore_ascii_case("SparkPlayerAudio") || raw.eq_ignore_ascii_case("sparkplayer_audio")
    {
        return "Local\\SparkPlayerAudio".to_string();
    }

    let mut normalized = String::from("Local\\");
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            normalized.push(ch);
        } else {
            normalized.push('_');
        }
    }
    if normalized == "Local\\" {
        normalized.push_str("SparkPlayerAudio");
    }
    normalized
}

#[cfg(unix)]
struct PlatformMappingImp {
    fd: i32,
    _writer_lock: std::fs::File,
    view: NonNull<u8>,
    byte_len: usize,
}

#[cfg(unix)]
impl PlatformMappingImp {
    fn open(name: &str) -> Result<Self> {
        use std::fs::OpenOptions;
        use std::os::fd::AsRawFd;
        use std::ptr;

        use libc::{
            LOCK_EX, LOCK_NB, MAP_FAILED, MAP_SHARED, O_CREAT, O_RDWR, PROT_READ, PROT_WRITE,
            close, flock, ftruncate, mmap, shm_open,
        };

        let name = normalize_posix_name(name)?;
        let byte_len = shared_byte_len();
        let lock_name = name.to_string_lossy().trim_start_matches('/').to_string();
        let lock_path = std::env::temp_dir().join(format!("sparkplayer-{lock_name}.lock"));
        let writer_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .with_context(|| format!("opening writer lock {}", lock_path.display()))?;
        if unsafe { flock(writer_lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            anyhow::bail!(
                "shared audio stream {} is already in use: {error}",
                name.to_string_lossy()
            );
        }

        let fd = unsafe { shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o600) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            anyhow::bail!("shm_open failed for {}: {error}", name.to_string_lossy());
        }

        if unsafe { ftruncate(fd, byte_len as libc::off_t) } != 0 {
            let error = std::io::Error::last_os_error();
            unsafe { close(fd) };
            anyhow::bail!("ftruncate failed for {}: {error}", name.to_string_lossy());
        }

        let view = unsafe {
            mmap(
                ptr::null_mut(),
                byte_len,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd,
                0,
            )
        };
        if view == MAP_FAILED {
            let error = std::io::Error::last_os_error();
            unsafe { close(fd) };
            anyhow::bail!("mmap failed for {}: {error}", name.to_string_lossy());
        }

        let view = NonNull::new(view.cast::<u8>()).expect("mmap returned MAP_FAILED or non-null");
        Ok(Self {
            fd,
            _writer_lock: writer_lock,
            view,
            byte_len,
        })
    }

    fn view(&self) -> NonNull<u8> {
        self.view
    }
}

#[cfg(unix)]
impl Drop for PlatformMappingImp {
    fn drop(&mut self) {
        use libc::{c_void, close, munmap};

        unsafe {
            munmap(self.view.as_ptr().cast::<c_void>(), self.byte_len);
            close(self.fd);
        }
    }
}

#[cfg(unix)]
fn normalize_posix_name(name: &str) -> Result<std::ffi::CString> {
    let trimmed = name.trim();
    let raw = if trimmed.is_empty() {
        "sparkplayer_audio"
    } else if let Some(rest) = trimmed.strip_prefix('/') {
        rest
    } else {
        trimmed
            .rsplit(['\\', '/', ':'])
            .next()
            .unwrap_or("sparkplayer_audio")
    };
    let mut normalized = String::with_capacity(raw.len() + 1);
    normalized.push('/');
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            normalized.push(ch);
        } else {
            normalized.push('_');
        }
    }
    if normalized.len() == 1 {
        normalized.push_str("sparkplayer_audio");
    }
    std::ffi::CString::new(normalized).context("shared audio stream name contains a NUL byte")
}

#[cfg(not(any(windows, unix)))]
struct PlatformMappingImp {
    storage: Box<std::cell::UnsafeCell<Vec<u8>>>,
}

#[cfg(not(any(windows, unix)))]
impl PlatformMappingImp {
    fn open(_name: &str) -> Result<Self> {
        Ok(Self {
            storage: Box::new(std::cell::UnsafeCell::new(vec![0; shared_byte_len()])),
        })
    }

    fn view(&self) -> NonNull<u8> {
        let ptr = unsafe { (*self.storage.get()).as_mut_ptr() };
        NonNull::new(ptr).expect("Vec allocation returned null")
    }
}

fn shared_byte_len() -> usize {
    size_of::<SparkAudioHeader>() + DEFAULT_CAPACITY_FRAMES as usize * OUTPUT_CHANNELS as usize * 4
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{METADATA_LEN, SharedAudioFramePacker, open, write_text};

    fn test_name(label: &str) -> String {
        format!("/sparkplayer_{label}_{}", std::process::id())
    }

    #[cfg(unix)]
    fn remove_test_mapping(name: &str) {
        let name = super::normalize_posix_name(name).unwrap();
        unsafe { libc::shm_unlink(name.as_ptr()) };
    }

    #[cfg(not(unix))]
    fn remove_test_mapping(_name: &str) {}

    #[test]
    fn mono_samples_are_duplicated_to_stereo_frames() {
        let mut packer = SharedAudioFramePacker::new(1);
        assert_eq!(packer.push_sample(0.25), Some((0.25, 0.25)));
        assert_eq!(packer.push_sample(-0.5), Some((-0.5, -0.5)));
    }

    #[test]
    fn stereo_samples_are_packed_in_pairs() {
        let mut packer = SharedAudioFramePacker::new(2);
        assert_eq!(packer.push_sample(0.25), None);
        assert_eq!(packer.push_sample(-0.5), Some((0.25, -0.5)));
    }

    #[test]
    fn multichannel_sources_keep_the_first_stereo_pair() {
        let mut packer = SharedAudioFramePacker::new(6);
        for sample in [0.1, 0.2, 0.3, 0.4, 0.5] {
            assert_eq!(packer.push_sample(sample), None);
        }
        assert_eq!(packer.push_sample(0.6), Some((0.1, 0.2)));
    }

    #[test]
    fn stream_epoch_does_not_clear_the_ring_and_publishes_complete_frames() {
        let name = test_name("publication");
        let writer = open(&name).unwrap();
        let header = writer.inner.header();
        unsafe { writer.inner.sample_ptr().write(123.0) };

        writer.begin_stream(48_000);
        let generation = writer
            .inner
            .header_u64(unsafe { std::ptr::addr_of_mut!((*header).generation) });
        assert_eq!(generation.load(Ordering::Acquire) & 1, 0);
        assert_eq!(unsafe { writer.inner.sample_ptr().read() }, 123.0);

        writer.push_frame(0.25, -0.5);
        let write_frame = writer
            .inner
            .header_u64(unsafe { std::ptr::addr_of_mut!((*header).write_frame) });
        assert_eq!(write_frame.load(Ordering::Acquire), 1);
        assert_eq!(unsafe { writer.inner.sample_ptr().read() }, 0.25);
        assert_eq!(unsafe { writer.inner.sample_ptr().add(1).read() }, -0.5);

        drop(writer);
        remove_test_mapping(&name);
    }

    #[test]
    fn writer_lock_survives_the_mapping_and_recovers_after_exit() {
        let name = test_name("ownership");
        let first = open(&name).unwrap();
        assert!(open(&name).is_err());
        drop(first);
        let replacement = open(&name).unwrap();
        drop(replacement);
        remove_test_mapping(&name);
    }

    #[test]
    fn metadata_truncation_preserves_utf8_boundaries() {
        let mut output = [0; METADATA_LEN];
        let value = format!("{}é", "a".repeat(126));
        unsafe { write_text(&mut output, &value) };
        let end = output.iter().position(|&byte| byte == 0).unwrap();
        assert_eq!(end, 126);
        assert!(std::str::from_utf8(&output[..end]).is_ok());
    }
}
