//! FFmpeg DirectShow video compressor filter (Rust implementation).

#![allow(non_snake_case)]

mod alloc;
mod config;
mod encoder;
mod enums;
mod filter;
mod mediatype;
mod pins;
mod state;

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};

use windows::core::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Registry::*;
use windows::Win32::Media::DirectShow::*;

use filter::Filter;
use state::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOINTERFACE, E_POINTER, Shared};

pub const CLSID_FFmpegEncoder: GUID = GUID::from_u128(0xD79D43B2_F005_40A4_BE18_AFD19C03E6E6);

const FILTER_NAME: &str = "FFmpeg Video Encoder (H.264/HEVC/AV1)";
const CLSID_STR: &str = "{D79D43B2-F005-40A4-BE18-AFD19C03E6E6}";
const CAT_VIDEO_COMPRESSOR: &str = "{33d9a760-90c8-11d0-bd43-00a0c911ce86}";
const CAT_DIRECTSHOW_FILTERS: &str = "{860bb310-5d01-11d0-bd3b-00a0c911ce86}";

static ACTIVE: AtomicI32 = AtomicI32::new(0);

// ---------------------------------------------------------------------------
// Class factory
// ---------------------------------------------------------------------------

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        state::debug_log(&format!(
            "CreateInstance: outer_null={} riid={:08X?}",
            punkouter.is_null(),
            unsafe { riid.as_ref().map(|g| g.data1) }
        ));
        if !punkouter.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let ppv = unsafe { ppvobject.as_mut() }.ok_or(E_POINTER)?;

        let shared = Shared::new();
        {
            // The output pin negotiates its media type before Run(), so the
            // codec/fourcc must be known from the start.
            let cfg = config::load();
            let (fourcc, mux) = encoder::pick_fourcc_mux(&cfg.codec);
            let mut core = shared.core.lock().unwrap();
            core.fourcc = fourcc;
            core.out_mux = mux.to_string();
            core.codec = cfg.codec.clone();
        }
        let filter = Filter::new(shared.clone());
        let base: IBaseFilter = filter.into();
        let weak = base.downgrade()?;
        *shared.filter.lock().unwrap() = Some(weak);

        let (input, output) = filter::make_pins(shared.clone());
        *shared.input_pin.lock().unwrap() = Some(input);
        *shared.output_pin.lock().unwrap() = Some(output.clone());

        let raw = base.into_raw();
        ACTIVE.fetch_add(1, Ordering::SeqCst);

        let iid = unsafe { riid.as_ref() }.ok_or(E_POINTER)?;
        if *iid == IBaseFilter::IID || *iid == IUnknown::IID {
            unsafe {
                *ppv = raw as *mut c_void;
            }
            Ok(())
        } else {
            unsafe {
                let _ = IUnknown::from_raw(raw);
            }
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
            Err(E_NOINTERFACE.into())
        }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DLL exports
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let Some(rclsid) = (unsafe { rclsid.as_ref() }) else {
        return HRESULT(0x80004003u32 as i32); // E_POINTER
    };
    state::debug_log(&format!(
        "DllGetClassObject: rclsid={:08X}-{:04X}-{:04X} expected={:08X}",
        rclsid.data1, rclsid.data2, rclsid.data3, CLSID_FFmpegEncoder.data1
    ));
    if *rclsid != CLSID_FFmpegEncoder {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = ClassFactory.into();
    let raw = factory.into_raw();
    match unsafe { riid.as_ref() } {
        Some(iid) if *iid == IClassFactory::IID || *iid == IUnknown::IID => {
            unsafe {
                ppv.write(raw as *mut _);
            }
            HRESULT(0)
        }
        _ => {
            unsafe {
                let _ = IUnknown::from_raw(raw);
            }
            E_NOINTERFACE
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    if ACTIVE.load(Ordering::SeqCst) == 0 {
        HRESULT(0)
    } else {
        HRESULT(1) // S_FALSE
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn reg_set_string(hkey: HKEY, name: PCWSTR, value: &str) -> bool {
    let v = wide(value);
    let r = RegSetValueExW(
        hkey,
        name,
        None,
        REG_SZ,
        Some(unsafe {
            std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2)
        }),
    );
    r == ERROR_SUCCESS
}

unsafe fn reg_create(hkey: HKEY, path: &str) -> Option<HKEY> {
    let p = wide(path);
    let mut hk = HKEY::default();
    let r = RegCreateKeyExW(
        hkey,
        PCWSTR::from_raw(p.as_ptr()),
        None,
        PCWSTR::null(),
        REG_OPTION_NON_VOLATILE,
        KEY_WRITE,
        None,
        &mut hk,
        None,
    );
    if r == ERROR_SUCCESS {
        Some(hk)
    } else {
        None
    }
}

fn dll_path() -> String {
    let mut hmod = HMODULE::default();
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(DllRegisterServer as *const u8 as *const u16),
            &mut hmod,
        )
    }
    .is_ok();
    if !ok {
        return String::new();
    }
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(Some(hmod), &mut buf) };
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

unsafe fn register_categories(b_register: bool) -> HRESULT {
    for cat in [CAT_VIDEO_COMPRESSOR, CAT_DIRECTSHOW_FILTERS] {
        let instance_key = format!(
            "Software\\Classes\\CLSID\\{}\\Instance\\{}",
            cat, CLSID_STR
        );
        if b_register {
            let Some(hk) = reg_create(HKEY_LOCAL_MACHINE, &instance_key) else {
                return HRESULT(0x80004005u32 as i32); // E_FAIL
            };
            let clsid_wide = wide(CLSID_STR);
            let name_wide = wide(FILTER_NAME);
            let clsid_bytes =
                unsafe { std::slice::from_raw_parts(clsid_wide.as_ptr() as *const u8, clsid_wide.len() * 2) };
            let name_bytes =
                unsafe { std::slice::from_raw_parts(name_wide.as_ptr() as *const u8, name_wide.len() * 2) };
            RegSetValueExW(hk, w!("CLSID"), None, REG_SZ, Some(clsid_bytes));
            RegSetValueExW(hk, w!("FriendlyName"), None, REG_SZ, Some(name_bytes));
            RegCloseKey(hk);
        } else {
            let p = wide(&instance_key);
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR::from_raw(p.as_ptr()));
        }
    }
    HRESULT(0)
}

#[no_mangle]
pub unsafe extern "system" fn DllRegisterServer() -> HRESULT {
    let clsid_key = format!("Software\\Classes\\CLSID\\{}", CLSID_STR);
    let Some(hk) = reg_create(HKEY_LOCAL_MACHINE, &clsid_key) else {
        return HRESULT(0x80004005u32 as i32);
    };
    reg_set_string(hk, PCWSTR::null(), FILTER_NAME);
    let path = dll_path();
    let Some(hk_isp) = reg_create(hk, "InprocServer32") else {
        RegCloseKey(hk);
        return HRESULT(0x80004005u32 as i32);
    };
    reg_set_string(hk_isp, PCWSTR::null(), &path);
    reg_set_string(hk_isp, w!("ThreadingModel"), "Both");
    RegCloseKey(hk_isp);
    RegCloseKey(hk);
    register_categories(true)
}

#[no_mangle]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    let clsid_key = format!("Software\\Classes\\CLSID\\{}", CLSID_STR);
    let p = wide(&clsid_key);
    let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR::from_raw(p.as_ptr()));
    register_categories(false)
}
