//! Shared state between the filter and its pins.

use std::collections::VecDeque;
use std::sync::Mutex;

use windows::core::{Error, HRESULT, Interface, Weak};
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::MediaFoundation::AM_MEDIA_TYPE;
use windows::Win32::Media::IReferenceClock;

use crate::encoder::Encoder;

#[derive(Clone)]
pub struct FormatInfo {
    pub width: i32,
    pub height: i32,
    pub bpp: u32,
    pub bottom_up: bool,
    pub pix_fmt: String,
    pub frame_dur: i64, // 100ns units
}

pub struct PinConn {
    pub peer: Option<IPin>,
    pub meminput: Option<IMemInputPin>,
    pub allocator: Option<IMemAllocator>,
    pub own_alloc: bool,
    pub mt: Option<AM_MEDIA_TYPE>,
}

impl PinConn {
    pub fn new() -> Self {
        PinConn {
            peer: None,
            meminput: None,
            allocator: None,
            own_alloc: false,
            mt: None,
        }
    }

    pub fn clear(&mut self) {
        self.peer = None;
        self.meminput = None;
        self.allocator = None;
        self.own_alloc = false;
        if let Some(mut mt) = self.mt.take() {
            unsafe { crate::mediatype::free_mt(&mut mt) };
        }
    }
}

pub struct Core {
    pub fmt: Option<FormatInfo>,
    pub fourcc: u32,
    pub out_mux: String,
    pub codec: String,
    pub started: bool,
    pub first_pkt: bool,
    pub ts_queue: VecDeque<i64>,
    pub last_ts: i64,
    pub state: FILTER_STATE,
    pub completed: bool,
    // Frames that arrive before the ffmpeg child is ready. They are written
    // as soon as the encoder starts, so no leading frames are lost.
    pub pending_frames: Vec<Vec<u8>>,
    // EndOfStream arrived before the encoder was started; propagate it to
    // the AVI Mux only after the buffered frames have been flushed.
    pub eos_pending: bool,
    pub avi_path: String,
    pub extra_path: String,
}

pub struct Shared {
    pub core: Mutex<Core>,
    pub encoder: Mutex<Option<Encoder>>,
    pub input: Mutex<PinConn>,
    pub output: Mutex<PinConn>,
    pub input_pin: Mutex<Option<IPin>>,
    pub output_pin: Mutex<Option<IPin>>,
    pub filter: Mutex<Option<Weak<IBaseFilter>>>,
    pub clock: Mutex<Option<IReferenceClock>>,
}

impl Shared {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Shared {
            core: Mutex::new(Core {
                fmt: None,
                fourcc: 0,
                out_mux: String::new(),
                codec: String::new(),
                started: false,
                first_pkt: true,
                ts_queue: VecDeque::new(),
                last_ts: 0,
                state: State_Stopped,
                completed: false,
                pending_frames: Vec::new(),
                eos_pending: false,
                avi_path: String::new(),
                extra_path: String::new(),
            }),
            encoder: Mutex::new(None),
            input: Mutex::new(PinConn::new()),
            output: Mutex::new(PinConn::new()),
            input_pin: Mutex::new(None),
            output_pin: Mutex::new(None),
            filter: Mutex::new(None),
            clock: Mutex::new(None),
        })
    }

    pub fn filter_iface(&self) -> Option<IBaseFilter> {
        self.filter
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|w| w.upgrade())
    }
}

pub fn err(hr: HRESULT) -> windows::core::Error {
    Error::from_hresult(hr)
}

pub fn debug_log(msg: &str) {
    use std::io::Write;
    if !debug_enabled() {
        return;
    }
    let log_path = std::env::temp_dir().join("ffmpeg_encoder_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(f, "{}", msg);
    }
}

static DEBUG_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn debug_enabled() -> bool {
    DEBUG_ON.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_debug(enabled: bool) {
    DEBUG_ON.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
pub const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
pub const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007000Eu32 as i32);
pub const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
pub const E_NOTIMPL_HR: HRESULT = HRESULT(0x80004001u32 as i32);
pub const E_UNEXPECTED_HR: HRESULT = HRESULT(0x8000FFFFu32 as i32);
pub const CLASS_E_NOAGGREGATION: HRESULT = HRESULT(0x80040110u32 as i32);
pub const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111u32 as i32);
pub const S_FALSE_HR: HRESULT = HRESULT(1);
