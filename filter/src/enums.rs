//! IEnumPins / IEnumMediaTypes implementations.

use std::sync::Mutex;

use windows::core::*;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::MediaFoundation::AM_MEDIA_TYPE;
use windows::Win32::System::Com::CoTaskMemFree;

use crate::mediatype::{alloc_mt_ptr, free_mt};
use crate::state::{err, E_OUTOFMEMORY, E_POINTER, S_FALSE_HR};

#[implement(IEnumPins)]
pub struct EnumPins {
    list: Mutex<Vec<IPin>>,
    pos: Mutex<usize>,
}

impl EnumPins {
    pub fn new(pins: Vec<IPin>) -> Self {
        EnumPins {
            list: Mutex::new(pins),
            pos: Mutex::new(0),
        }
    }
}

impl IEnumPins_Impl for EnumPins_Impl {
    fn Next(&self, cpins: u32, pppins: *mut Option<IPin>, pcfetched: *mut u32) -> HRESULT {
        if cpins > 0 && pppins.is_null() {
            return E_POINTER;
        }
        let mut pos = self.pos.lock().unwrap();
        let list = self.list.lock().unwrap();
        let mut fetched = 0u32;
        for i in 0..cpins as usize {
            if *pos >= list.len() {
                break;
            }
            unsafe {
                pppins.add(i).write(Some(list[*pos].clone()));
            }
            *pos += 1;
            fetched += 1;
        }
        if !pcfetched.is_null() {
            unsafe {
                *pcfetched = fetched;
            }
        }
        if fetched == cpins {
            HRESULT(0)
        } else {
            S_FALSE_HR
        }
    }

    fn Skip(&self, cmediatypes: u32) -> Result<()> {
        let mut pos = self.pos.lock().unwrap();
        let len = self.list.lock().unwrap().len();
        *pos = (*pos + cmediatypes as usize).min(len);
        if cmediatypes > 0 && *pos >= len {
            Err(err(S_FALSE_HR))
        } else {
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        *self.pos.lock().unwrap() = 0;
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumPins> {
        let e = EnumPins::new(self.list.lock().unwrap().clone());
        *e.pos.lock().unwrap() = *self.pos.lock().unwrap();
        Ok(e.into())
    }
}

#[implement(IEnumMediaTypes)]
pub struct EnumMediaTypes {
    list: Mutex<Vec<AM_MEDIA_TYPE>>,
    pos: Mutex<usize>,
}

impl EnumMediaTypes {
    pub fn new(mts: Vec<AM_MEDIA_TYPE>) -> Self {
        EnumMediaTypes {
            list: Mutex::new(mts),
            pos: Mutex::new(0),
        }
    }
}

impl Drop for EnumMediaTypes {
    fn drop(&mut self) {
        let mut list = self.list.lock().unwrap();
        for mt in list.iter_mut() {
            unsafe { free_mt(mt) };
        }
    }
}

impl IEnumMediaTypes_Impl for EnumMediaTypes_Impl {
    fn Next(
        &self,
        cmediatypes: u32,
        ppmediatypes: *mut *mut AM_MEDIA_TYPE,
        pcfetched: *mut u32,
    ) -> HRESULT {
        if cmediatypes > 0 && ppmediatypes.is_null() {
            return E_POINTER;
        }
        let mut pos = self.pos.lock().unwrap();
        let list = self.list.lock().unwrap();
        let mut fetched = 0u32;
        for i in 0..cmediatypes as usize {
            if *pos >= list.len() {
                break;
            }
            let p = unsafe { alloc_mt_ptr(&list[*pos]) };
            if p.is_null() {
                return E_OUTOFMEMORY;
            }
            unsafe {
                ppmediatypes.add(i).write(p);
            }
            *pos += 1;
            fetched += 1;
        }
        if !pcfetched.is_null() {
            unsafe {
                *pcfetched = fetched;
            }
        }
        if fetched == cmediatypes {
            HRESULT(0)
        } else {
            S_FALSE_HR
        }
    }

    fn Skip(&self, cmediatypes: u32) -> Result<()> {
        let mut pos = self.pos.lock().unwrap();
        let len = self.list.lock().unwrap().len();
        *pos = (*pos + cmediatypes as usize).min(len);
        if cmediatypes > 0 && *pos >= len {
            Err(err(S_FALSE_HR))
        } else {
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        *self.pos.lock().unwrap() = 0;
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumMediaTypes> {
        let list = self.list.lock().unwrap();
        let mut cloned = Vec::new();
        for mt in list.iter() {
            cloned.push(crate::mediatype::clone_mt(mt));
        }
        let e = EnumMediaTypes::new(cloned);
        *e.pos.lock().unwrap() = *self.pos.lock().unwrap();
        Ok(e.into())
    }
}

#[allow(unused)]
fn _free_enum_mt(p: *mut AM_MEDIA_TYPE) {
    if !p.is_null() {
        unsafe {
            free_mt(&mut *p);
            CoTaskMemFree(Some(p as *const _));
        }
    }
}
