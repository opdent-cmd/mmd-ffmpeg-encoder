//! Minimal IMemAllocator + IMediaSample implementations for our output pin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::MediaFoundation::AM_MEDIA_TYPE;

use crate::state::{err, E_POINTER, S_FALSE_HR};

#[implement(IMemAllocator)]
pub struct MemAllocator {
    props: Mutex<ALLOCATOR_PROPERTIES>,
    committed: AtomicBool,
}

impl MemAllocator {
    pub fn new() -> Self {
        MemAllocator {
            props: Mutex::new(ALLOCATOR_PROPERTIES {
                cBuffers: 1,
                cbBuffer: 0,
                cbAlign: 1,
                cbPrefix: 0,
            }),
            committed: AtomicBool::new(false),
        }
    }
}

impl IMemAllocator_Impl for MemAllocator_Impl {
    fn SetProperties(&self, prequest: *const ALLOCATOR_PROPERTIES) -> Result<ALLOCATOR_PROPERTIES> {
        let req = unsafe { prequest.as_ref() }.ok_or(E_POINTER)?;
        let mut p = *req;
        if p.cbAlign < 1 {
            p.cbAlign = 1;
        }
        if p.cBuffers < 1 {
            p.cBuffers = 1;
        }
        if p.cbBuffer < 1 {
            p.cbBuffer = 1;
        }
        let aligned = (p.cbBuffer + p.cbAlign - 1) / p.cbAlign * p.cbAlign;
        p.cbBuffer = aligned;
        *self.props.lock().unwrap() = p;
        Ok(p)
    }

    fn GetProperties(&self) -> Result<ALLOCATOR_PROPERTIES> {
        Ok(*self.props.lock().unwrap())
    }

    fn Commit(&self) -> Result<()> {
        self.committed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn Decommit(&self) -> Result<()> {
        self.committed.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn GetBuffer(
        &self,
        ppbuffer: OutRef<'_, IMediaSample>,
        pstarttime: *const i64,
        pendtime: *const i64,
        dwflags: u32,
    ) -> Result<()> {
        if !self.committed.load(Ordering::SeqCst) {
            return Err(err(VFW_E_NOT_COMMITTED));
        }
        let props = *self.props.lock().unwrap();
        let sample: IMediaSample =
            MediaSample::new(props.cbBuffer as usize, props.cbPrefix as usize).into();
        if !pstarttime.is_null() && !pendtime.is_null() {
            unsafe {
                sample.SetTime(Some(pstarttime), Some(pendtime))?;
            }
        }
        if dwflags & AM_GBF_NOTASYNCPOINT != 0 {
            unsafe {
                sample.SetSyncPoint(false)?;
            }
        }
        ppbuffer.write(Some(sample))
    }

    fn ReleaseBuffer(&self, _pbuffer: Ref<'_, IMediaSample>) -> Result<()> {
        Ok(())
    }
}

#[implement(IMediaSample)]
pub struct MediaSample {
    data: Box<[u8]>,
    prefix: usize,
    actual: Mutex<i32>,
    start: Mutex<Option<i64>>,
    stop: Mutex<Option<i64>>,
    sync: AtomicBool,
    preroll: AtomicBool,
    discontinuity: AtomicBool,
}

impl MediaSample {
    pub fn new(total: usize, prefix: usize) -> Self {
        MediaSample {
            data: vec![0u8; total].into_boxed_slice(),
            prefix,
            actual: Mutex::new(0),
            start: Mutex::new(None),
            stop: Mutex::new(None),
            sync: AtomicBool::new(true),
            preroll: AtomicBool::new(false),
            discontinuity: AtomicBool::new(false),
        }
    }
}

impl IMediaSample_Impl for MediaSample_Impl {
    fn GetPointer(&self) -> Result<*mut u8> {
        Ok(unsafe { self.data.as_ptr().add(self.prefix) as *mut u8 })
    }

    fn GetSize(&self) -> i32 {
        (self.data.len() - self.prefix) as i32
    }

    fn GetTime(&self, ptimestart: *mut i64, ptimeend: *mut i64) -> Result<()> {
        if ptimestart.is_null() || ptimeend.is_null() {
            return Err(err(E_POINTER));
        }
        let s = *self.start.lock().unwrap();
        let e = *self.stop.lock().unwrap();
        match (s, e) {
            (Some(s), Some(e)) => {
                unsafe {
                    *ptimestart = s;
                    *ptimeend = e;
                }
                Ok(())
            }
            _ => Err(err(S_FALSE_HR)),
        }
    }

    fn SetTime(&self, ptimestart: *const i64, ptimeend: *const i64) -> Result<()> {
        if ptimestart.is_null() {
            *self.start.lock().unwrap() = None;
            *self.stop.lock().unwrap() = None;
            return Ok(());
        }
        *self.start.lock().unwrap() = Some(unsafe { *ptimestart });
        *self.stop.lock().unwrap() = if ptimeend.is_null() {
            None
        } else {
            Some(unsafe { *ptimeend })
        };
        Ok(())
    }

    fn IsSyncPoint(&self) -> HRESULT {
        if self.sync.load(Ordering::SeqCst) {
            HRESULT(0)
        } else {
            S_FALSE_HR
        }
    }

    fn SetSyncPoint(&self, bissyncpoint: BOOL) -> Result<()> {
        self.sync.store(bissyncpoint.as_bool(), Ordering::SeqCst);
        Ok(())
    }

    fn IsPreroll(&self) -> HRESULT {
        if self.preroll.load(Ordering::SeqCst) {
            HRESULT(0)
        } else {
            S_FALSE_HR
        }
    }

    fn SetPreroll(&self, bispreroll: BOOL) -> Result<()> {
        self.preroll.store(bispreroll.as_bool(), Ordering::SeqCst);
        Ok(())
    }

    fn GetActualDataLength(&self) -> i32 {
        *self.actual.lock().unwrap()
    }

    fn SetActualDataLength(&self, len: i32) -> Result<()> {
        let size = (self.data.len() - self.prefix) as i32;
        if len < 0 || len > size {
            return Err(err(crate::state::E_INVALIDARG));
        }
        *self.actual.lock().unwrap() = len;
        Ok(())
    }

    fn GetMediaType(&self) -> Result<*mut AM_MEDIA_TYPE> {
        Ok(std::ptr::null_mut())
    }

    fn SetMediaType(&self, _pmediatype: *const AM_MEDIA_TYPE) -> Result<()> {
        Ok(())
    }

    fn IsDiscontinuity(&self) -> HRESULT {
        if self.discontinuity.load(Ordering::SeqCst) {
            HRESULT(0)
        } else {
            S_FALSE_HR
        }
    }

    fn SetDiscontinuity(&self, bdiscontinuity: BOOL) -> Result<()> {
        self.discontinuity
            .store(bdiscontinuity.as_bool(), Ordering::SeqCst);
        Ok(())
    }

    fn GetMediaTime(&self, _ptimestart: *mut i64, _ptimeend: *mut i64) -> Result<()> {
        Ok(())
    }

    fn SetMediaTime(&self, _ptimestart: *const i64, _ptimeend: *const i64) -> Result<()> {
        Ok(())
    }
}
