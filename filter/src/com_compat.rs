//! ABI shims for DirectShow callers with stricter out-parameter behavior.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{HRESULT, Interface};
use windows::Win32::Media::DirectShow::{IPin, IPin_Vtbl};

use crate::state::{E_POINTER, E_UNEXPECTED_HR};

type ConnectedToFn =
    unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;

struct PatchedPinVtable {
    original_connected_to: ConnectedToFn,
    vtable: IPin_Vtbl,
}

static INPUT_VTABLE: OnceLock<PatchedPinVtable> = OnceLock::new();
static OUTPUT_VTABLE: OnceLock<PatchedPinVtable> = OnceLock::new();

unsafe extern "system" fn input_connected_to(
    this: *mut c_void,
    peer: *mut *mut c_void,
) -> HRESULT {
    connected_to_with_clean_out(this, peer, &INPUT_VTABLE)
}

unsafe extern "system" fn output_connected_to(
    this: *mut c_void,
    peer: *mut *mut c_void,
) -> HRESULT {
    connected_to_with_clean_out(this, peer, &OUTPUT_VTABLE)
}

unsafe fn connected_to_with_clean_out(
    this: *mut c_void,
    peer: *mut *mut c_void,
    patched: &'static OnceLock<PatchedPinVtable>,
) -> HRESULT {
    if peer.is_null() {
        return E_POINTER;
    }

    // The windows crate only writes this out parameter on success. Quartz on
    // Windows 10 can inspect it after VFW_E_NOT_CONNECTED, so establish the
    // COM-required null value before dispatching to the generated thunk.
    unsafe {
        peer.write(std::ptr::null_mut());
    }

    let Some(patched) = patched.get() else {
        return E_UNEXPECTED_HR;
    };
    unsafe { (patched.original_connected_to)(this, peer) }
}

fn patch_pin(
    pin: &IPin,
    storage: &'static OnceLock<PatchedPinVtable>,
    replacement: ConnectedToFn,
) {
    let interface = Interface::as_raw(pin) as *mut *const IPin_Vtbl;
    let original = unsafe { *interface };
    let patched = storage.get_or_init(|| {
        // COM vtables contain only function pointers. Copy the generated
        // table and replace one ABI entry while retaining its IUnknown and
        // all remaining IPin implementations verbatim.
        let mut vtable = unsafe { std::ptr::read(original) };
        let original_connected_to = vtable.ConnectedTo;
        vtable.ConnectedTo = replacement;
        PatchedPinVtable {
            original_connected_to,
            vtable,
        }
    });

    // A COM interface pointer addresses a writable vtable-pointer slot in
    // the generated object. The copied table itself is process-static.
    unsafe {
        interface.write(&patched.vtable);
    }
}

pub fn patch_input_pin(pin: &IPin) {
    patch_pin(pin, &INPUT_VTABLE, input_connected_to);
}

pub fn patch_output_pin(pin: &IPin) {
    patch_pin(pin, &OUTPUT_VTABLE, output_connected_to);
}
