//! Process-level crash capture for the host (MMD).
//!
//! The recurring reports are "unknown module" access violations whose
//! faulting address is heap data (a corrupted return address), so a plain
//! log cannot tell us which of our COM calls corrupted the heap.  This
//! module installs an unhandled-exception filter once, writes a minidump
//! next to the debug log, records the exception code/address, and chains to
//! the previous filter so we do not change MMD's own crash behavior.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};
use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpWriteDump, SetUnhandledExceptionFilter, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    LPTOP_LEVEL_EXCEPTION_FILTER, MINIDUMP_EXCEPTION_INFORMATION, MINIDUMP_TYPE, MiniDumpNormal,
    MiniDumpWithDataSegs, MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo,
    MiniDumpWithUnloadedModules,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static DUMPED: AtomicBool = AtomicBool::new(false);
static PREV_FILTER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn prev_filter_from_raw(p: *mut c_void) -> LPTOP_LEVEL_EXCEPTION_FILTER {
    if p.is_null() {
        None
    } else {
        Some(unsafe {
            std::mem::transmute::<
                *mut c_void,
                unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32,
            >(p)
        })
    }
}

/// Install the crash handler exactly once per process. Safe to call from
/// DllGetClassObject as often as the host likes.
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let prev = SetUnhandledExceptionFilter(Some(handler));
        let raw = match prev {
            Some(f) => f as *const () as *mut c_void,
            None => std::ptr::null_mut(),
        };
        PREV_FILTER.store(raw, Ordering::SeqCst);
    }
    crate::state::always_log("crash dump handler installed");
}

unsafe extern "system" fn handler(ep: *const EXCEPTION_POINTERS) -> i32 {
    if !DUMPED.swap(true, Ordering::SeqCst) {
        if !ep.is_null() {
            let rec = (*ep).ExceptionRecord;
            if !rec.is_null() {
                let code = (*rec).ExceptionCode.0 as u32;
                let addr = (*rec).ExceptionAddress;
                let flags = (*rec).ExceptionFlags;
                crate::state::always_log(&format!(
                    "CRASH: unhandled exception code=0x{:08X} address=0x{:p} flags=0x{:08X}",
                    code, addr, flags
                ));
            }
        }
        let ok = write_minidump(ep);
        crate::state::always_log(&format!(
            "CRASH: minidump {} -> {}",
            if ok { "written" } else { "FAILED" },
            crate::state::crash_dump_path().to_string_lossy()
        ));
    }
    let raw = PREV_FILTER.load(Ordering::SeqCst);
    if let Some(f) = prev_filter_from_raw(raw) {
        f(ep)
    } else {
        EXCEPTION_CONTINUE_SEARCH
    }
}

fn write_minidump(ep: *const EXCEPTION_POINTERS) -> bool {
    let dir = crate::state::log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let path = crate::state::crash_dump_path();
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let file = match unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    } {
        Ok(h) => h,
        Err(_) => return false,
    };
    let mei = MINIDUMP_EXCEPTION_INFORMATION {
        ThreadId: unsafe { GetCurrentThreadId() },
        ExceptionPointers: ep as *mut EXCEPTION_POINTERS,
        ClientPointers: false.into(),
    };
    let dump_type: MINIDUMP_TYPE = MiniDumpNormal
        | MiniDumpWithDataSegs
        | MiniDumpWithIndirectlyReferencedMemory
        | MiniDumpWithUnloadedModules
        | MiniDumpWithThreadInfo;
    let ok = unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            file,
            dump_type,
            Some(&mei),
            None,
            None,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(file) };
    ok
}
