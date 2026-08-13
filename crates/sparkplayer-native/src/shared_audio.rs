const MAGIC: u32 = u32::from_le_bytes(*b"SPRK");
const VERSION: u32 = 1;
const OUTPUT_CHANNELS: u32 = 2;
const DEFAULT_CAPACITY_FRAMES: u32 = 48_000 * 10;

pub struct SharedAudioControl {
    pub playback: Option<bool>,
    pub visualizer_delta: i32,
}

#[allow(dead_code)]
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
    reserved: [u32; 7],
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::{Arc, Mutex};

    use anyhow::{Context, Result};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
        PAGE_READWRITE, UnmapViewOfFile,
    };

    use super::{DEFAULT_CAPACITY_FRAMES, MAGIC, OUTPUT_CHANNELS, SparkAudioHeader, VERSION};

    pub struct SharedAudioWriter {
        inner: Arc<Mutex<SharedAudioWriterInner>>,
    }

    struct SharedAudioWriterInner {
        mapping: HANDLE,
        view: *mut u8,
        capacity_frames: usize,
        source_channels: usize,
        sample_rate: u32,
        write_frame: u64,
        generation: u64,
        pending_frame: Vec<f32>,
        last_transport_control: u32,
        last_visualizer_control: u32,
    }

    unsafe impl Send for SharedAudioWriterInner {}

    impl SharedAudioWriter {
        pub fn open(name: &str) -> Result<Self> {
            let capacity_frames = DEFAULT_CAPACITY_FRAMES as usize;
            let header_size = std::mem::size_of::<SparkAudioHeader>();
            let byte_len =
                header_size + capacity_frames * OUTPUT_CHANNELS as usize * size_of::<f32>();
            let mapping_name = normalize_windows_name(name);
            let mut wide = mapping_name.encode_utf16().collect::<Vec<_>>();
            wide.push(0);

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
                anyhow::bail!("CreateFileMappingW failed for {name}");
            }

            let view_address =
                unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, byte_len) };
            let view = view_address.Value.cast::<u8>();
            if view.is_null() {
                unsafe {
                    CloseHandle(mapping);
                }
                anyhow::bail!("MapViewOfFile failed for {name}");
            }

            let mut inner = SharedAudioWriterInner {
                mapping,
                view,
                capacity_frames,
                source_channels: 2,
                sample_rate: 44_100,
                write_frame: 0,
                generation: 1,
                pending_frame: Vec::with_capacity(8),
                last_transport_control: 0,
                last_visualizer_control: 0,
            };
            inner.initialize_header();

            Ok(Self {
                inner: Arc::new(Mutex::new(inner)),
            })
        }

        pub fn set_format(&self, channels: u16, sample_rate: u32) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.set_format(channels, sample_rate);
            }
        }

        pub fn push_sample(&self, sample: f32) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.push_sample(sample);
            }
        }

        pub fn reset(&self) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.reset();
            }
        }

        pub fn poll_control(&self) -> super::SharedAudioControl {
            self.inner
                .lock()
                .map(|mut inner| inner.poll_control())
                .unwrap_or(super::SharedAudioControl {
                    playback: None,
                    visualizer_delta: 0,
                })
        }
    }

    impl Clone for SharedAudioWriter {
        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
            }
        }
    }


    fn normalize_windows_name(name: &str) -> String {
        let trimmed = name.trim();
        if trimmed.eq_ignore_ascii_case("Local\\SparkPlayerAudio") || trimmed.eq_ignore_ascii_case("Local\\sparkplayer_audio") {
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

        if raw.eq_ignore_ascii_case("SparkPlayerAudio") || raw.eq_ignore_ascii_case("sparkplayer_audio") {
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
    impl SharedAudioWriterInner {
        fn header(&self) -> *mut SparkAudioHeader {
            self.view.cast::<SparkAudioHeader>()
        }

        fn poll_control(&mut self) -> super::SharedAudioControl {
            let header = self.header();
            let transport_generation = unsafe { ptr::addr_of!((*header).reserved[0]).read_volatile() };
            let transport_state = unsafe { ptr::addr_of!((*header).reserved[1]).read_volatile() };
            let visualizer_generation = unsafe { ptr::addr_of!((*header).reserved[2]).read_volatile() };
            let visualizer_delta = unsafe { ptr::addr_of!((*header).reserved[3]).read_volatile() as i32 };

            let playback = if transport_generation != self.last_transport_control {
                self.last_transport_control = transport_generation;
                match transport_state {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                }
            } else {
                None
            };

            let delta = if visualizer_generation != self.last_visualizer_control {
                self.last_visualizer_control = visualizer_generation;
                visualizer_delta.clamp(-16, 16)
            } else {
                0
            };

            super::SharedAudioControl {
                playback,
                visualizer_delta: delta,
            }
        }
        fn initialize_header(&mut self) {
            let header = self.header();
            unsafe {
                ptr::write(
                    header,
                    SparkAudioHeader {
                        magic: MAGIC,
                        version: VERSION,
                        header_size: size_of::<SparkAudioHeader>() as u32,
                        capacity_frames: self.capacity_frames as u32,
                        channels: OUTPUT_CHANNELS,
                        sample_rate: self.sample_rate,
                        write_frame: 0,
                        total_frames: 0,
                        generation: self.generation,
                        active: 1,
                        reserved: [0; 7],
                    },
                );
            }
            self.clear_audio();
        }

        fn set_format(&mut self, channels: u16, sample_rate: u32) {
            let channels = channels.max(1) as usize;
            let sample_rate = sample_rate.max(1);
            if self.source_channels == channels && self.sample_rate == sample_rate {
                return;
            }
            self.source_channels = channels;
            self.sample_rate = sample_rate;
            self.pending_frame.clear();
            self.reset();
        }

        fn reset(&mut self) {
            self.write_frame = 0;
            self.generation = self.generation.wrapping_add(1).max(1);
            self.pending_frame.clear();
            self.clear_audio();
            let header = self.header();
            unsafe {
                ptr::addr_of_mut!((*header).sample_rate).write_volatile(self.sample_rate);
                ptr::addr_of_mut!((*header).write_frame).write_volatile(0);
                ptr::addr_of_mut!((*header).total_frames).write_volatile(0);
                ptr::addr_of_mut!((*header).generation).write_volatile(self.generation);
                ptr::addr_of_mut!((*header).active).write_volatile(1);
            }
        }

        fn clear_audio(&mut self) {
            let sample_ptr = unsafe { self.view.add(size_of::<SparkAudioHeader>()).cast::<f32>() };
            let sample_count = self.capacity_frames * OUTPUT_CHANNELS as usize;
            unsafe {
                ptr::write_bytes(sample_ptr, 0, sample_count);
            }
        }

        fn push_sample(&mut self, sample: f32) {
            self.pending_frame.push(sample);
            if self.pending_frame.len() < self.source_channels {
                return;
            }

            let (left, right) = if self.source_channels == 1 {
                (self.pending_frame[0], self.pending_frame[0])
            } else {
                (self.pending_frame[0], self.pending_frame[1])
            };
            self.pending_frame.clear();
            self.push_stereo_frame(left, right);
        }

        fn push_stereo_frame(&mut self, left: f32, right: f32) {
            let frame_index = (self.write_frame as usize) % self.capacity_frames;
            let sample_offset = frame_index * OUTPUT_CHANNELS as usize;
            let sample_ptr = unsafe { self.view.add(size_of::<SparkAudioHeader>()).cast::<f32>() };
            unsafe {
                sample_ptr.add(sample_offset).write(left);
                sample_ptr.add(sample_offset + 1).write(right);
            }

            self.write_frame = self.write_frame.wrapping_add(1);
            let header = self.header();
            unsafe {
                ptr::addr_of_mut!((*header).write_frame).write_volatile(self.write_frame);
                ptr::addr_of_mut!((*header).total_frames).write_volatile(self.write_frame);
            }
        }
    }

    impl Drop for SharedAudioWriterInner {
        fn drop(&mut self) {
            if !self.view.is_null() {
                let header = self.header();
                unsafe {
                    ptr::addr_of_mut!((*header).active).write_volatile(0);
                    UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.view.cast::<c_void>(),
                    });
                }
                self.view = ptr::null_mut();
            }
            if !self.mapping.is_null() {
                unsafe {
                    CloseHandle(self.mapping);
                }
                self.mapping = ptr::null_mut();
            }
        }
    }

    pub fn open(name: &str) -> Result<SharedAudioWriter> {
        SharedAudioWriter::open(name).with_context(|| format!("opening shared audio stream {name}"))
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::ptr;
    use std::sync::{Arc, Mutex};

    use anyhow::{Context, Result};
    use libc::{
        MAP_FAILED, MAP_SHARED, O_CREAT, O_RDWR, PROT_READ, PROT_WRITE, c_void, close, ftruncate,
        mmap, munmap, shm_open, shm_unlink,
    };

    use super::{DEFAULT_CAPACITY_FRAMES, MAGIC, OUTPUT_CHANNELS, SparkAudioHeader, VERSION};

    pub struct SharedAudioWriter {
        inner: Arc<Mutex<SharedAudioWriterInner>>,
    }

    struct SharedAudioWriterInner {
        fd: i32,
        name: CString,
        view: *mut u8,
        byte_len: usize,
        capacity_frames: usize,
        source_channels: usize,
        sample_rate: u32,
        write_frame: u64,
        generation: u64,
        pending_frame: Vec<f32>,
        last_transport_control: u32,
        last_visualizer_control: u32,
    }

    unsafe impl Send for SharedAudioWriterInner {}

    impl SharedAudioWriter {
        pub fn open(name: &str) -> Result<Self> {
            let name = normalize_posix_name(name)?;
            let capacity_frames = DEFAULT_CAPACITY_FRAMES as usize;
            let header_size = std::mem::size_of::<SparkAudioHeader>();
            let byte_len =
                header_size + capacity_frames * OUTPUT_CHANNELS as usize * size_of::<f32>();

            unsafe {
                shm_unlink(name.as_ptr());
            }
            let fd = unsafe { shm_open(name.as_ptr(), O_CREAT | O_RDWR, 0o600) };
            if fd < 0 {
                anyhow::bail!("shm_open failed for {}", name.to_string_lossy());
            }

            if unsafe { ftruncate(fd, byte_len as libc::off_t) } != 0 {
                unsafe {
                    close(fd);
                    shm_unlink(name.as_ptr());
                }
                anyhow::bail!("ftruncate failed for {}", name.to_string_lossy());
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
                unsafe {
                    close(fd);
                    shm_unlink(name.as_ptr());
                }
                anyhow::bail!("mmap failed for {}", name.to_string_lossy());
            }

            let mut inner = SharedAudioWriterInner {
                fd,
                name,
                view: view.cast::<u8>(),
                byte_len,
                capacity_frames,
                source_channels: 2,
                sample_rate: 44_100,
                write_frame: 0,
                generation: 1,
                pending_frame: Vec::with_capacity(8),
                last_transport_control: 0,
                last_visualizer_control: 0,
            };
            inner.initialize_header();

            Ok(Self {
                inner: Arc::new(Mutex::new(inner)),
            })
        }

        pub fn set_format(&self, channels: u16, sample_rate: u32) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.set_format(channels, sample_rate);
            }
        }

        pub fn push_sample(&self, sample: f32) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.push_sample(sample);
            }
        }

        pub fn reset(&self) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.reset();
            }
        }

        pub fn poll_control(&self) -> super::SharedAudioControl {
            self.inner
                .lock()
                .map(|mut inner| inner.poll_control())
                .unwrap_or(super::SharedAudioControl {
                    playback: None,
                    visualizer_delta: 0,
                })
        }
    }

    impl Clone for SharedAudioWriter {
        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
            }
        }
    }

    fn normalize_posix_name(name: &str) -> Result<CString> {
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
            normalized.push_str("SparkPlayerAudio");
        }
        CString::new(normalized).context("shared audio stream name contains a NUL byte")
    }

    impl SharedAudioWriterInner {
        fn header(&self) -> *mut SparkAudioHeader {
            self.view.cast::<SparkAudioHeader>()
        }

        fn poll_control(&mut self) -> super::SharedAudioControl {
            let header = self.header();
            let transport_generation = unsafe { ptr::addr_of!((*header).reserved[0]).read_volatile() };
            let transport_state = unsafe { ptr::addr_of!((*header).reserved[1]).read_volatile() };
            let visualizer_generation = unsafe { ptr::addr_of!((*header).reserved[2]).read_volatile() };
            let visualizer_delta = unsafe { ptr::addr_of!((*header).reserved[3]).read_volatile() as i32 };

            let playback = if transport_generation != self.last_transport_control {
                self.last_transport_control = transport_generation;
                match transport_state {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                }
            } else {
                None
            };

            let delta = if visualizer_generation != self.last_visualizer_control {
                self.last_visualizer_control = visualizer_generation;
                visualizer_delta.clamp(-16, 16)
            } else {
                0
            };

            super::SharedAudioControl {
                playback,
                visualizer_delta: delta,
            }
        }
        fn initialize_header(&mut self) {
            let header = self.header();
            unsafe {
                ptr::write(
                    header,
                    SparkAudioHeader {
                        magic: MAGIC,
                        version: VERSION,
                        header_size: size_of::<SparkAudioHeader>() as u32,
                        capacity_frames: self.capacity_frames as u32,
                        channels: OUTPUT_CHANNELS,
                        sample_rate: self.sample_rate,
                        write_frame: 0,
                        total_frames: 0,
                        generation: self.generation,
                        active: 1,
                        reserved: [0; 7],
                    },
                );
            }
            self.clear_audio();
        }

        fn set_format(&mut self, channels: u16, sample_rate: u32) {
            let channels = channels.max(1) as usize;
            let sample_rate = sample_rate.max(1);
            if self.source_channels == channels && self.sample_rate == sample_rate {
                return;
            }
            self.source_channels = channels;
            self.sample_rate = sample_rate;
            self.pending_frame.clear();
            self.reset();
        }

        fn reset(&mut self) {
            self.write_frame = 0;
            self.generation = self.generation.wrapping_add(1).max(1);
            self.pending_frame.clear();
            self.clear_audio();
            let header = self.header();
            unsafe {
                ptr::addr_of_mut!((*header).sample_rate).write_volatile(self.sample_rate);
                ptr::addr_of_mut!((*header).write_frame).write_volatile(0);
                ptr::addr_of_mut!((*header).total_frames).write_volatile(0);
                ptr::addr_of_mut!((*header).generation).write_volatile(self.generation);
                ptr::addr_of_mut!((*header).active).write_volatile(1);
            }
        }

        fn clear_audio(&mut self) {
            let sample_ptr = unsafe { self.view.add(size_of::<SparkAudioHeader>()).cast::<f32>() };
            let sample_count = self.capacity_frames * OUTPUT_CHANNELS as usize;
            unsafe {
                ptr::write_bytes(sample_ptr, 0, sample_count);
            }
        }

        fn push_sample(&mut self, sample: f32) {
            self.pending_frame.push(sample);
            if self.pending_frame.len() < self.source_channels {
                return;
            }

            let (left, right) = if self.source_channels == 1 {
                (self.pending_frame[0], self.pending_frame[0])
            } else {
                (self.pending_frame[0], self.pending_frame[1])
            };
            self.pending_frame.clear();
            self.push_stereo_frame(left, right);
        }

        fn push_stereo_frame(&mut self, left: f32, right: f32) {
            let frame_index = (self.write_frame as usize) % self.capacity_frames;
            let sample_offset = frame_index * OUTPUT_CHANNELS as usize;
            let sample_ptr = unsafe { self.view.add(size_of::<SparkAudioHeader>()).cast::<f32>() };
            unsafe {
                sample_ptr.add(sample_offset).write(left);
                sample_ptr.add(sample_offset + 1).write(right);
            }

            self.write_frame = self.write_frame.wrapping_add(1);
            let header = self.header();
            unsafe {
                ptr::addr_of_mut!((*header).write_frame).write_volatile(self.write_frame);
                ptr::addr_of_mut!((*header).total_frames).write_volatile(self.write_frame);
            }
        }
    }

    impl Drop for SharedAudioWriterInner {
        fn drop(&mut self) {
            if !self.view.is_null() {
                let header = self.header();
                unsafe {
                    ptr::addr_of_mut!((*header).active).write_volatile(0);
                    munmap(self.view.cast::<c_void>(), self.byte_len);
                }
                self.view = ptr::null_mut();
            }
            if self.fd >= 0 {
                unsafe {
                    close(self.fd);
                    shm_unlink(self.name.as_ptr());
                }
                self.fd = -1;
            }
        }
    }

    pub fn open(name: &str) -> Result<SharedAudioWriter> {
        SharedAudioWriter::open(name).with_context(|| format!("opening shared audio stream {name}"))
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use anyhow::Result;

    #[derive(Clone)]
    pub struct SharedAudioWriter;

    impl SharedAudioWriter {
        pub fn set_format(&self, _channels: u16, _sample_rate: u32) {}
        pub fn push_sample(&self, _sample: f32) {}
        pub fn reset(&self) {}
        pub fn poll_control(&self) -> super::SharedAudioControl {
            super::SharedAudioControl {
                playback: None,
                visualizer_delta: 0,
            }
        }
    }

    pub fn open(_name: &str) -> Result<SharedAudioWriter> {
        Ok(SharedAudioWriter)
    }
}

pub use imp::{SharedAudioWriter, open};
