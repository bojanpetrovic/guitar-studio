//! Running work on the main thread, from any thread. Plugins start, show their editors
//! and tear down there.
//!
//! This uses the system's main dispatch queue, so it works the same in the desktop app
//! and inside a plugin host (whose main thread we don't own).

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
}

#[cfg(not(target_vendor = "apple"))]
mod imp {
    // Guitar Studio is macOS-only so far; elsewhere, run in place.
    pub fn run(job: Box<dyn FnOnce() + Send>) {
        job();
    }

    pub fn is_main() -> bool {
        true
    }
}
