//! Start on the CPU where Windows has no Vulkan loader (P18 stage 3).
//!
//! ggml-vulkan hands `vkGetInstanceProcAddr` to vulkan-hpp's dispatcher, and
//! that one import makes `vulkan-1.dll` a hard dependency of the sidecar: on a
//! machine without the loader (a VM, a server on the basic display adapter)
//! Windows would refuse to start the process at all. ggml registers its
//! backends before whisper ever looks at `use_gpu`, so `--no-gpu` does not
//! help either.
//!
//! `build.rs` therefore links `vulkan-1.dll` delay-loaded, and this module is
//! the failure hook the MSVC delay-load helper calls when binding fails. A
//! missing loader gets a stand-in `vkGetInstanceProcAddr` whose only function
//! reports `VK_ERROR_INITIALIZATION_FAILED` from `vkEnumerateInstanceVersion`
//! — the first thing ggml asks. vulkan-hpp turns that into an exception,
//! `ggml_backend_vk_reg` catches it and registers no Vulkan device, and
//! whisper falls back to the CPU as it does wherever no GPU is found.
//!
//! A loader that is present but finds no driver needs none of this:
//! `vkCreateInstance` fails, and ggml takes the same exception path.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

type Farproc = *const c_void;
type DliHook = unsafe extern "system" fn(u32, *const DelayLoadInfo) -> Farproc;

/// `DelayLoadInfo` from `delayimp.h`, as far as this hook reads it.
#[repr(C)]
struct DelayLoadInfo {
    cb: u32,
    pidd: *const c_void,
    ppfn: *mut Farproc,
    sz_dll: *const c_char,
    dlp: DelayLoadProc,
    hmod_cur: *mut c_void,
    pfn_cur: Farproc,
    dw_last_error: u32,
}

#[repr(C)]
struct DelayLoadProc {
    f_import_by_name: i32,
    /// `szProcName` when `f_import_by_name`, else the ordinal.
    sz_proc_name: *const c_char,
}

const DLI_FAIL_LOAD_LIB: u32 = 3;
const DLI_FAIL_GET_PROC: u32 = 4;
const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;

/// Picked up by delayimp.lib in place of its own null default, because the
/// linker resolves it from this object before it searches the library.
#[no_mangle]
#[used]
static __pfnDliFailureHook2: Option<DliHook> = Some(on_failure);

/// Set once the stand-in was handed out, for the log line in [`report`].
static MISSING: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

unsafe extern "system" fn on_failure(notify: u32, info: *const DelayLoadInfo) -> Farproc {
    let info = &*info;
    if !is_vulkan(info.sz_dll) {
        return std::ptr::null();
    }
    match notify {
        // Answer with this executable's own module: it exports no Vulkan
        // functions, so the helper comes back below for each one it binds.
        DLI_FAIL_LOAD_LIB => {
            MISSING.store(true, Ordering::Relaxed);
            GetModuleHandleW(std::ptr::null()) as Farproc
        }
        DLI_FAIL_GET_PROC
            if info.dlp.f_import_by_name != 0
                && CStr::from_ptr(info.dlp.sz_proc_name) == c"vkGetInstanceProcAddr" =>
        {
            stub_get_instance_proc_addr as Farproc
        }
        // Any other import stays unbound and raises if it is ever called —
        // which ggml does not do without an instance.
        _ => std::ptr::null(),
    }
}

unsafe fn is_vulkan(dll: *const c_char) -> bool {
    !dll.is_null()
        && CStr::from_ptr(dll)
            .to_bytes()
            .eq_ignore_ascii_case(b"vulkan-1.dll")
}

unsafe extern "system" fn stub_get_instance_proc_addr(
    _instance: *mut c_void,
    name: *const c_char,
) -> Farproc {
    if !name.is_null() && CStr::from_ptr(name) == c"vkEnumerateInstanceVersion" {
        stub_enumerate_instance_version as Farproc
    } else {
        std::ptr::null()
    }
}

unsafe extern "system" fn stub_enumerate_instance_version(_version: *mut u32) -> i32 {
    VK_ERROR_INITIALIZATION_FAILED
}

/// Log the fallback once the model is loaded, i.e. after ggml went looking.
pub fn report() {
    if MISSING.load(Ordering::Relaxed) {
        tracing::warn!(
            "vulkan-1.dll not found: no Vulkan driver installed, transcribing on the CPU"
        );
    }
}
