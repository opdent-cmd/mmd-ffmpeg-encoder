//! FFmpeg DirectShow video compressor filter (Rust implementation).

#![allow(non_snake_case)]
#![allow(linker_messages)]

mod alloc;
mod config;
mod crash;
mod encoder;
mod enums;
mod filter;
mod mediatype;
mod pins;
mod state;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use windows::core::*;
use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Registry::*;
use windows::Win32::Media::DirectShow::*;

use filter::Filter;
use state::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOINTERFACE, E_POINTER, Shared};

pub const CLSID_FFMPEG_ENCODER: GUID = GUID::from_u128(0xD79D43B2_F005_40A4_BE18_AFD19C03E6E6);

const FILTER_NAME: &str = "FFmpeg Video Encoder (H.264/HEVC/AV1)";
const CLSID_STR: &str = "{D79D43B2-F005-40A4-BE18-AFD19C03E6E6}";
const CAT_VIDEO_COMPRESSOR: &str = "{33d9a760-90c8-11d0-bd43-00a0c911ce86}";
const CAT_DIRECTSHOW_FILTERS: &str = "{860bb310-5d01-11d0-bd3b-00a0c911ce86}";

static ACTIVE: AtomicI32 = AtomicI32::new(0);
static PANIC_HOOK_SET: AtomicBool = AtomicBool::new(false);

fn ensure_panic_hook() {
    if !PANIC_HOOK_SET.swap(true, Ordering::SeqCst) {
        std::panic::set_hook(Box::new(|info| {
            crate::state::always_log(&format!("PANIC: {}", info));
            crate::state::show_error_log();
        }));
    }
}

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
        state::always_log(&format!(
            "CreateInstance: outer_null={} riid={:08X?}",
            punkouter.is_null(),
            unsafe { riid.as_ref().map(|g| g.data1) }
        ));
        // Never let an internal Rust panic take down the host (MMD). The
        // release profile used panic=abort, which killed the whole process
        // right here when something went wrong and left no trace beyond the
        // CreateInstance log line. Catch instead, log the panic, and return
        // E_FAIL so MMD shows an error instead of crashing.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.create_instance_inner(punkouter, riid, ppvobject)
        }));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic payload".to_string());
                state::always_log(&format!(
                    "CreateInstance PANIC caught: {}",
                    msg
                ));
                Err(state::err(crate::state::E_FAIL))
            }
        }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

impl ClassFactory_Impl {
    fn create_instance_inner(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if !punkouter.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let ppv = unsafe { ppvobject.as_mut() }.ok_or(E_POINTER)?;

        state::always_log("CreateInstance: Shared::new");
        let shared = Shared::new();
        state::always_log("CreateInstance: config::load");
        {
            // The output pin negotiates its media type before Run(), so the
            // codec/fourcc must be known from the start.
            let cfg = config::load();
            // Start the real-time log viewer as early as possible when
            // debug mode is enabled.
            state::set_debug(cfg.debug);
            let (fourcc, mux) = encoder::pick_fourcc_mux(&cfg.codec, &cfg.alpha_format);
            let mut core = shared.core.lock().unwrap();
            core.fourcc = fourcc;
            core.out_mux = mux.to_string();
            core.codec = cfg.codec.clone();
        }
        state::always_log("CreateInstance: Filter::new");
        let filter = Filter::new(shared.clone());
        let base: IBaseFilter = filter.into();
        let weak = base.downgrade()?;
        *shared.filter.lock().unwrap() = Some(weak);

        state::always_log("CreateInstance: make_pins");
        let (input, output) = filter::make_pins(shared.clone());
        *shared.input_pin.lock().unwrap() = Some(input);
        *shared.output_pin.lock().unwrap() = Some(output.clone());

        let raw = base.into_raw();
        ACTIVE.fetch_add(1, Ordering::SeqCst);

        let iid = unsafe { riid.as_ref() }.ok_or(E_POINTER)?;
        if *iid == IBaseFilter::IID || *iid == IUnknown::IID {
            state::always_log("CreateInstance: returning interface");
            *ppv = raw as *mut c_void;
            Ok(())
        } else {
            unsafe {
                let _ = IUnknown::from_raw(raw);
            }
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
            Err(E_NOINTERFACE.into())
        }
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
    ensure_panic_hook();
    crash::install();
    let Some(rclsid) = (unsafe { rclsid.as_ref() }) else {
        return HRESULT(0x80004003u32 as i32); // E_POINTER
    };
    state::always_log(&format!(
        "DllGetClassObject: rclsid={:08X}-{:04X}-{:04X} expected={:08X}",
        rclsid.data1, rclsid.data2, rclsid.data3, CLSID_FFMPEG_ENCODER.data1
    ));
    if *rclsid != CLSID_FFMPEG_ENCODER {
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
            let _ = RegSetValueExW(hk, w!("CLSID"), None, REG_SZ, Some(clsid_bytes));
            let _ = RegSetValueExW(hk, w!("FriendlyName"), None, REG_SZ, Some(name_bytes));
            let _ = RegCloseKey(hk);
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
        let _ = RegCloseKey(hk);
        return HRESULT(0x80004005u32 as i32);
    };
    reg_set_string(hk_isp, PCWSTR::null(), &path);
    reg_set_string(hk_isp, w!("ThreadingModel"), "Both");
    let _ = RegCloseKey(hk_isp);
    let _ = RegCloseKey(hk);
    register_categories(true)
}

#[no_mangle]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    let clsid_key = format!("Software\\Classes\\CLSID\\{}", CLSID_STR);
    let p = wide(&clsid_key);
    let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR::from_raw(p.as_ptr()));
    register_categories(false)
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    /// Reproduce the exact interrogation sequence seen in the Win10 crash
    /// log (CreateInstance -> EnumPins -> QueryDirection/QueryPinInfo/
    /// ConnectedTo/QueryInternalConnections -> QueryFilterInfo) without
    /// touching the registry.
    #[test]
    fn com_factory_and_pin_interrogation_smoke() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut factory_raw: *mut c_void = std::ptr::null_mut();
        let hr =
            unsafe { DllGetClassObject(&CLSID_FFMPEG_ENCODER, &IClassFactory::IID, &mut factory_raw) };
        assert_eq!(hr.0, 0, "DllGetClassObject failed");
        let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_raw as *mut _) };

        let obj: IUnknown =
            unsafe { factory.CreateInstance(None) }.expect("CreateInstance failed");
        let base: IBaseFilter = obj.cast().expect("QI IBaseFilter failed");

        // The graph builder sequence from the reporter's log.
        let pins = unsafe { base.EnumPins() }.expect("EnumPins failed");
        let mut pin_count = 0;
        loop {
            let mut arr = [None; 1];
            let hr = unsafe { pins.Next(&mut arr, None) };
            if hr.0 != 0 {
                break;
            }
            let pin = arr[0].take().expect("Next returned empty pin slot");
            pin_count += 1;

            let mut info = PIN_INFO::default();
            assert!(unsafe { pin.QueryPinInfo(&mut info) }.is_ok());
            let _ = unsafe { pin.QueryDirection() }.expect("QueryDirection failed");
            let _ = unsafe { pin.ConnectedTo() };
            let mut n = 0u32;
            let _ = unsafe { pin.QueryInternalConnections(None, &mut n) };
            if let Some(f) = core::mem::ManuallyDrop::into_inner(info.pFilter) {
                drop(f);
            }
            drop(pin);
        }
        assert!(pin_count >= 2, "expected input+output pins, got {}", pin_count);

        let mut fi = FILTER_INFO::default();
        assert!(unsafe { base.QueryFilterInfo(&mut fi) }.is_ok());
        if let Some(g) = core::mem::ManuallyDrop::into_inner(fi.pGraph) {
            drop(g);
        }
        unsafe { base.EnumPins() }.expect("second EnumPins failed");
        drop(base);
        drop(factory);
    }

    /// MMD's encoder list creates and releases our filter many times in a
    /// row (the reporter's log shows ~8 cycles before the crash). Repeated
    /// COM teardown is a classic way to expose a refcount/lifetime bug that
    /// corrupts the heap and only crashes on a later iteration.
    #[test]
    fn com_create_release_stress() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        for round in 0..3000 {
            let mut factory_raw: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                DllGetClassObject(&CLSID_FFMPEG_ENCODER, &IClassFactory::IID, &mut factory_raw)
            };
            assert_eq!(hr.0, 0);
            let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_raw as *mut _) };

            let obj: IUnknown = unsafe { factory.CreateInstance(None) }.expect("CreateInstance");
            let base: IBaseFilter = obj.cast().expect("QI IBaseFilter");

            let pins = unsafe { base.EnumPins() }.expect("EnumPins");
            let mut arr = [None; 1];
            let mut count = 0u32;
            while unsafe { pins.Next(&mut arr, Some(&mut count)) }.0 == 0 && count > 0 {
                let pin = arr[0].take().unwrap();
                let mut info = PIN_INFO::default();
                if unsafe { pin.QueryPinInfo(&mut info) }.is_ok() {
                    if let Some(f) = core::mem::ManuallyDrop::into_inner(info.pFilter) {
                        drop(f);
                    }
                }
                drop(pin);
            }
            drop(pins);
            drop(base);
            drop(factory);

            if round % 500 == 0 {
                crate::state::always_log(&format!("stress round {}", round));
            }
        }
    }
}
