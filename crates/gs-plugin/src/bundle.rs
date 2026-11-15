//! Where the plugin is installed. Its bundle holds the scanner next to the plugin's
//! binary (`Contents/MacOS`, or `Contents/x86_64-win` for the Windows VST3) and the UI
//! in `Contents/Resources/ui`. On Windows the CLAP is a single file, so it's installed
//! in a folder of its own with the scanner and `Resources/ui` beside it.

use std::path::PathBuf;

/// The folder holding this plugin's binary (`<bundle>/Contents/MacOS`, …).
pub fn binary_dir() -> Option<PathBuf> {
    own_binary()?.parent().map(PathBuf::from)
}

/// The built UI (`npm run build`), copied into the bundle.
pub fn ui_dir() -> Option<PathBuf> {
    let dir = binary_dir()?;
    let bundled = dir.parent()?.join("Resources").join("ui");
    let beside = dir.join("Resources").join("ui");
    Some(if !bundled.is_dir() && beside.is_dir() {
        beside
    } else {
        bundled
    })
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

#[cfg(windows)]
fn own_binary() -> Option<PathBuf> {
    // gs-app is linked into this DLL, so its module is ours.
    gs_app::win::module_path()
}

#[cfg(not(any(unix, windows)))]
fn own_binary() -> Option<PathBuf> {
    None
}
