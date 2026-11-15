//! Running work on the main thread, from any thread. Plugins start, show their editors
//! and tear down there.
//!
//! On macOS this uses the system's main dispatch queue, so it works the same in the
//! desktop app and inside a plugin host (whose main thread we don't own). Windows has
//! no such queue: [`init`] (called on the main thread at startup) makes a hidden window
//! there, and jobs are posted to it; whoever runs that thread's message loop (the app,
//! or the DAW) runs them.

/// Make the main-thread queue on this thread. Call once from the main thread before
/// anything is queued (the app at startup, the plugin when the DAW creates it). Only
/// Windows needs it; elsewhere it does nothing.
pub fn init() {
    imp::init();
}

/// Run `f` on the main thread, soon (never inline, even when called from it).
pub fn run(f: impl FnOnce() + Send + 'static) {
    imp::run(Box::new(f));
}

/// Run `f` on the main thread and wait for its result (inline when already there).
/// `None` if it panicked.
pub fn run_and_wait<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    if is_main() {
        return Some(f());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    run(move || {
        let _ = tx.send(f());
    });
    rx.recv().ok()
}

pub fn is_main() -> bool {
    imp::is_main()
}

#[cfg(target_vendor = "apple")]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    struct DispatchQueue {
        _opaque: [u8; 0],
    }

    unsafe extern "C" {
        /// What `dispatch_get_main_queue()` (a C macro) returns.
        static _dispatch_main_q: DispatchQueue;
        fn dispatch_async_f(
            queue: *const DispatchQueue,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        fn pthread_main_np() -> i32;
    }

    type Job = Box<dyn FnOnce() + Send>;

    pub fn run(job: Job) {
        extern "C" fn trampoline(context: *mut c_void) {
            // SAFETY: `context` is the `Box<Job>` leaked below, handed to us exactly once.
            let job = unsafe { Box::from_raw(context.cast::<Job>()) };
            // A panic must not unwind into the dispatch queue (that aborts the process,
            // and inside a plugin host that's the user's DAW).
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                log::error!("a main-thread job panicked");
            }
        }
        let context = Box::into_raw(Box::new(job)).cast::<c_void>();
        // SAFETY: the main queue lives for the whole process; `trampoline` takes
        // ownership of `context`.
        unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, trampoline) };
    }

    pub fn is_main() -> bool {
        // SAFETY: no preconditions.
        unsafe { pthread_main_np() != 0 }
    }

    pub fn init() {}
}

#[cfg(windows)]
mod imp {
    use crate::win;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, HWND_MESSAGE, PostMessageW, WM_APP,
    };

    type Job = Box<dyn FnOnce() + Send>;

    /// The message that carries a job (its `Box<Job>` in `lParam`).
    const WM_JOB: u32 = WM_APP + 0x4753;

    /// The hidden window jobs are posted to, and the thread that owns it.
    static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static MAIN: AtomicU32 = AtomicU32::new(0);

    pub fn init() {
        if !WINDOW.load(Ordering::Acquire).is_null() {
            return;
        }
        win::pin_module();
        let class = win::wide("GuitarStudioMainThread");
        win::register_class(&class, Some(window_proc), std::ptr::null_mut());
        // SAFETY: a message-only window of a class registered just above.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                win::own_module(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return log::error!("couldn't set up the main-thread queue");
        }
        // SAFETY: no preconditions.
        MAIN.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        WINDOW.store(hwnd, Ordering::Release);
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_JOB {
            // SAFETY: `lparam` is the `Box<Job>` leaked by `run`, delivered exactly once.
            let job = unsafe { Box::from_raw(lparam as *mut Job) };
            // A panic must not unwind into the message loop (that's the DAW's).
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                log::error!("a main-thread job panicked");
            }
            return 0;
        }
        // SAFETY: the default handling for everything else.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub fn run(job: Job) {
        let hwnd = WINDOW.load(Ordering::Acquire);
        if hwnd.is_null() {
            log::warn!("main-thread queue not set up; running a job in place");
            return job();
        }
        let context = Box::into_raw(Box::new(job));
        // SAFETY: the window lives for the whole process (its module is pinned);
        // `window_proc` takes ownership of `context`.
        if unsafe { PostMessageW(hwnd, WM_JOB, 0, context as LPARAM) } == 0 {
            // SAFETY: not posted, so still ours.
            drop(unsafe { Box::from_raw(context) });
            log::error!("couldn't queue a main-thread job");
        }
    }

    pub fn is_main() -> bool {
        match MAIN.load(Ordering::Acquire) {
            0 => true,
            // SAFETY: no preconditions.
            main => main == unsafe { GetCurrentThreadId() },
        }
    }
}

#[cfg(not(any(target_vendor = "apple", windows)))]
mod imp {
    // Not a supported platform yet; run in place.
    pub fn init() {}

    pub fn run(job: Box<dyn FnOnce() + Send>) {
        job();
    }

    pub fn is_main() -> bool {
        true
    }
}
