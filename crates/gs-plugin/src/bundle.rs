//! Where the plugin is installed: its bundle holds the scanner (`Contents/MacOS`) and
//! the UI (`Contents/Resources/ui`).

use std::path::PathBuf;

/// The folder holding this plugin's binary (`<bundle>/Contents/MacOS`).
pub fn binary_dir() -> Option<PathBuf> {
    own_binary()?.parent().map(PathBuf::from)
}

/// The built UI (`npm run build`), copied into the bundle.
pub fn ui_dir() -> Option<PathBuf> {
    Some(binary_dir()?.parent()?.join("Resources").join("ui"))
}

/// The path of the binary this code was loaded from (the DAW's own executable is
/// `current_exe`, so ask the dynamic loader instead).
#[cfg(unix)]
fn own_binary() -> Option<PathBuf> {
    use std::ffi::{CStr, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }
    unsafe extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
    }

    let mut info = DlInfo {
        dli_fname: std::ptr::null(),
        dli_fbase: std::ptr::null_mut(),
        dli_sname: std::ptr::null(),
        dli_saddr: std::ptr::null_mut(),
    };
    let here = own_binary as fn() -> Option<PathBuf>;
    // SAFETY: `here` is an address inside this image; `info` is a valid out-pointer.
    let found = unsafe { dladdr(here as *const c_void, &mut info) };
    if found == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: on success `dli_fname` is a NUL-terminated path owned by the loader.
    let name = unsafe { CStr::from_ptr(info.dli_fname) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
}

#[cfg(not(unix))]
fn own_binary() -> Option<PathBuf> {
    None
}
