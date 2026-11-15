//! Small Win32 helpers shared by the main-thread queue and the plugin editor windows.

use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HMODULE, HWND};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{RegisterClassW, WNDCLASSW, WNDPROC};

/// A NUL-terminated UTF-16 copy of `s`, for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The module (the app's .exe, or the plugin's DLL inside a DAW) this code lives in.
/// Window classes are registered against it, so two copies of the plugin (VST3 and
/// CLAP in one DAW) each get their own.
pub fn own_module() -> HMODULE {
    module_of(0)
}

/// Keep this module loaded until the process ends. A DAW may unload the plugin's DLL
/// once its last instance is gone, while a hidden window or a thread of ours still
/// points into it.
pub fn pin_module() {
    module_of(GET_MODULE_HANDLE_EX_FLAG_PIN);
}

fn module_of(extra: u32) -> HMODULE {
    let here = own_module as fn() -> HMODULE as *const c_void;
    let mut module: HMODULE = std::ptr::null_mut();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
        | if extra == 0 {
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT
        } else {
            extra
        };
    // SAFETY: `here` is an address inside this module; `module` is a valid out-pointer.
    unsafe { GetModuleHandleExW(flags, here.cast(), &mut module) };
    module
}

/// Where this module's file is.
pub fn module_path() -> Option<PathBuf> {
    let module = own_module();
    let mut buf = vec![0u16; 1024];
    loop {
        // SAFETY: `buf` is writable for `buf.len()` code units.
        let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
        }
        buf.resize(buf.len() * 2, 0);
    }
}

/// Register a window class of this module (again is fine: the first one stays).
pub fn register_class(name: &[u16], proc: WNDPROC, background: *mut c_void) {
    let class = WNDCLASSW {
        lpfnWndProc: proc,
        hInstance: own_module(),
        lpszClassName: name.as_ptr(),
        hbrBackground: background,
        // SAFETY: all-zero is a valid "unset" for the remaining fields.
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `class` and the name it points to are valid for the call.
    unsafe { RegisterClassW(&class) };
}

/// A window handle that may cross threads (only ever used on the thread that made it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hwnd(pub HWND);
// SAFETY: a plain handle value; every call through it happens on the main thread.
unsafe impl Send for Hwnd {}
// SAFETY: see above.
unsafe impl Sync for Hwnd {}
