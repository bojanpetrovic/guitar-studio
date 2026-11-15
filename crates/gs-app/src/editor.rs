//! Plugin editor windows: a plain native window (AppKit on macOS, Win32 on Windows)
//! that becomes the parent of the plugin's own GUI. Plain native windows (not the front
//! end's toolkit) so the same windows work in the desktop app and inside a plugin host.
//!
//! Main thread only.

use crate::App;
use anyhow::Result;

/// Smallest plausible editor edge, in points; anything less is a view still setting up.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
const MIN_EDGE: f64 = 20.0;

#[cfg(target_os = "macos")]
pub use mac::{any_open, close, close_all, open};
#[cfg(windows)]
pub use windows::{any_open, close, close_all, open};

#[cfg(not(any(target_os = "macos", windows)))]
pub fn open(_app: &App, _id: u32) -> Result<()> {
    Err(anyhow::anyhow!(
        "plugin editors aren't wired up on this platform yet"
    ))
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn close(_app: &App, _id: u32) {}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn close_all(_app: &App) {}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn any_open(_app: &App) -> bool {
    false
}

/// Open editors are kept by (app instance, node). Several apps exist when the plugin
/// host has more than one Guitar Studio loaded.
#[cfg(any(target_os = "macos", windows))]
type Key = (usize, u32);

#[cfg(any(target_os = "macos", windows))]
fn key(app: &App, id: u32) -> Key {
    (std::ptr::from_ref(app) as usize, id)
}

/// Every 250 ms, on the main thread, run `check` until it says the editor is gone.
#[cfg(any(target_os = "macos", windows))]
fn follow(k: Key, check: fn(Key) -> bool) {
    use std::time::Duration;
    let spawned = std::thread::Builder::new()
        .name("gs-editor-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let open = crate::main_thread::run_and_wait(move || check(k)).unwrap_or(false);
                if !open {
                    break;
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("editor watcher didn't start: {e}");
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{App, Key, MIN_EDGE, Result, key};
    use anyhow::anyhow;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSFloatingWindowLevel, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use truce_rack::core::WindowHandle;

    /// One open editor: its window and the plugin it shows.
    struct Editor {
        window: Retained<NSWindow>,
        plugin: gs_engine::plugins::SharedPlugin,
        /// Last size the plugin's view reported (to react only to changes).
        seen: Option<(f64, f64)>,
    }

    thread_local! {
        static EDITORS: RefCell<HashMap<Key, Editor>> = RefCell::new(HashMap::new());
    }

    fn mtm() -> Result<MainThreadMarker> {
        MainThreadMarker::new().ok_or_else(|| anyhow!("plugin editors open on the main thread"))
    }

    pub fn open(app: &App, id: u32) -> Result<()> {
        let mtm = mtm()?;
        let k = key(app, id);
        let shown =
            EDITORS.with_borrow(|eds| eds.get(&k).map(|ed| ed.window.makeKeyAndOrderFront(None)));
        if shown.is_some() {
            return Ok(());
        }
        let plugin = app
            .studio
            .lock()
            .plugin(id)
            .ok_or_else(|| anyhow!("node {id} is not a plugin"))?;
        let title = plugin.lock().descriptor.name.clone();

        // SAFETY: plain window creation on the main thread (`mtm`).
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(640.0, 420.0)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // We keep our own reference; closing only hides it until we let go.
        // SAFETY: called before anything else retains or closes the window.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&title));
        if app.hosted {
            // Over the DAW's windows while it's in front, like its own plugin windows.
            window.setLevel(NSFloatingWindowLevel);
            window.setHidesOnDeactivate(true);
        }
        let view = window
            .contentView()
            .ok_or_else(|| anyhow!("the editor window has no content view"))?;

        let attached = {
            let mut slot = plugin.lock();
            match slot.plugin.editor() {
                Some(editor) => {
                    let parent = WindowHandle::NSView(Retained::as_ptr(&view) as *mut _);
                    editor
                        .open(parent, window.backingScaleFactor())
                        .map(|()| (editor.size(), editor.is_resizable()))
                        .map_err(anyhow::Error::from)
                }
                None => Err(anyhow!("{title} has no editor")),
            }
        };
        let (size, resizable) = match attached {
            Ok(v) => v,
            Err(e) => {
                window.close();
                return Err(e);
            }
        };
        // Some editors (JUCE-based ones such as Neural DSP's) report 1×1 at first and size
        // themselves a moment later: keep the default size for those; `follow` picks up
        // the real one.
        if let Some((w, h)) = size.filter(|&(w, h)| w as f64 >= MIN_EDGE && h as f64 >= MIN_EDGE) {
            window.setContentSize(fit_to_screen(&window, (w as f64, h as f64)));
        }
        if resizable {
            window.setStyleMask(window.styleMask() | NSWindowStyleMask::Resizable);
        }
        window.center();
        window.makeKeyAndOrderFront(None);
        let seen = plugin_view_size(&window);
        EDITORS.with_borrow_mut(|eds| {
            eds.insert(
                k,
                Editor {
                    window,
                    plugin,
                    seen,
                },
            )
        });
        super::follow(k, check);
        Ok(())
    }

    /// Close the plugin's editor once its window was closed, and resize the window when
    /// the plugin's view changes size (late first layouts, plugins with their own zoom
    /// setting). Whether the editor is still open after this check.
    fn check(k: Key) -> bool {
        let closed = EDITORS.with_borrow_mut(|eds| {
            let ed = eds.get_mut(&k)?;
            if !ed.window.isVisible() && !ed.window.isMiniaturized() {
                return eds.remove(&k);
            }
            let now = plugin_view_size(&ed.window);
            // Only react to changes: some views (Apple's generic AU view) report a frame
            // smaller than what they draw, and the size set at open already covers that.
            if now != ed.seen {
                ed.seen = now;
                if let Some((w, h)) = now.filter(|&(w, h)| w >= MIN_EDGE && h >= MIN_EDGE) {
                    ed.window.setContentSize(fit_to_screen(&ed.window, (w, h)));
                    pin_view_top(&ed.window);
                }
            }
            None
        });
        match closed {
            Some(ed) => {
                // The user closed the window.
                tear_down(ed);
                false
            }
            None => EDITORS.with_borrow(|eds| eds.contains_key(&k)),
        }
    }

    /// Detach the plugin's GUI, then let the window go. Outside the `EDITORS` borrow: a
    /// plugin may run the main loop while it closes.
    fn tear_down(ed: Editor) {
        if let Some(editor) = ed.plugin.lock().plugin.editor() {
            editor.close();
        }
        ed.window.close();
    }

    /// The plugin view's size: the first subview of the window's content view.
    fn plugin_view_size(window: &NSWindow) -> Option<(f64, f64)> {
        let frame = window.contentView()?.subviews().firstObject()?.frame();
        Some((frame.size.width, frame.size.height))
    }

    /// Hang the plugin view from the window's top edge. AppKit anchors views bottom-left,
    /// so an editor taller than the (screen-limited) window would lose its top bar; this
    /// way it's the bottom that's cut.
    fn pin_view_top(window: &NSWindow) {
        let Some(parent) = window.contentView() else {
            return;
        };
        let view: Option<Retained<NSView>> = parent.subviews().firstObject();
        if let Some(view) = view {
            let top = parent.bounds().size.height - view.frame().size.height;
            view.setFrameOrigin(NSPoint::new(0.0, top));
        }
    }

    /// Keep an editor window on screen (a few editors ask for more than a laptop has).
    fn fit_to_screen(window: &NSWindow, (w, h): (f64, f64)) -> NSSize {
        match window.screen() {
            Some(screen) => {
                let area = screen.visibleFrame().size;
                NSSize::new(w.min(area.width - 40.0), h.min(area.height - 80.0))
            }
            None => NSSize::new(w, h),
        }
    }

    /// Close a node's editor (if open) before the node goes away.
    pub fn close(app: &App, id: u32) {
        let k = key(app, id);
        if let Some(ed) = EDITORS.with_borrow_mut(|eds| eds.remove(&k)) {
            tear_down(ed);
        }
    }

    /// Whether any of this app's plugin editors is open.
    pub fn any_open(app: &App) -> bool {
        let me = std::ptr::from_ref(app) as usize;
        EDITORS.with_borrow(|eds| eds.keys().any(|k| k.0 == me))
    }

    /// Close every editor of this app (it's shutting down).
    pub fn close_all(app: &App) {
        let me = std::ptr::from_ref(app) as usize;
        let mine: Vec<Editor> = EDITORS.with_borrow_mut(|eds| {
            let keys: Vec<Key> = eds.keys().filter(|k| k.0 == me).copied().collect();
            keys.into_iter().filter_map(|k| eds.remove(&k)).collect()
        });
        mine.into_iter().for_each(tear_down);
    }
}

#[cfg(windows)]
mod windows {
    use super::{App, Key, MIN_EDGE, Result, key};
    use crate::win;
    use anyhow::anyhow;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use truce_rack::core::WindowHandle;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BLACK_BRUSH, GetMonitorInfoW, GetStockObject, MONITOR_DEFAULTTONEAREST, MONITORINFO,
        MonitorFromPoint,
    };
    use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GA_ROOTOWNER, GW_CHILD,
        GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow,
        GetWindow, GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE,
        SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        SetForegroundWindow, SetWindowLongW, SetWindowPos, ShowWindow, WINDOW_STYLE, WM_CLOSE,
        WS_CAPTION, WS_CLIPCHILDREN, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
        WS_THICKFRAME,
    };

    /// One open editor: its window and the plugin it shows.
    struct Editor {
        hwnd: HWND,
        plugin: gs_engine::plugins::SharedPlugin,
        resizable: bool,
        /// Last size of the plugin's own window (to react only to changes).
        seen: Option<(i32, i32)>,
        /// Our window's inside, as last set or seen (a change is the user resizing).
        client: (i32, i32),
    }

    thread_local! {
        static EDITORS: RefCell<HashMap<Key, Editor>> = RefCell::new(HashMap::new());
    }

    const CLASS: &str = "GuitarStudioPluginEditor";

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            // Only hide it: the watcher notices and detaches the plugin before the
            // window goes (`check`).
            // SAFETY: `hwnd` is the window this message is for.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
        // SAFETY: the default handling for everything else.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn style(resizable: bool) -> WINDOW_STYLE {
        let base = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        if resizable {
            base | WS_THICKFRAME | WS_MAXIMIZEBOX
        } else {
            base
        }
    }

    fn show(hwnd: HWND) {
        // SAFETY: `hwnd` is one of our live editor windows.
        unsafe {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(hwnd);
        }
    }

    pub fn open(app: &App, id: u32) -> Result<()> {
        let k = key(app, id);
        if EDITORS
            .with_borrow(|eds| eds.get(&k).map(|ed| show(ed.hwnd)))
            .is_some()
        {
            return Ok(());
        }
        let plugin = app
            .studio
            .lock()
            .plugin(id)
            .ok_or_else(|| anyhow!("node {id} is not a plugin"))?;
        let title = plugin.lock().descriptor.name.clone();

        let class = win::wide(CLASS);
        // SAFETY: a stock object; no preconditions.
        win::register_class(&class, Some(window_proc), unsafe {
            GetStockObject(BLACK_BRUSH)
        });
        // Inside a DAW: owned by its main window, so it stays over it and goes with it,
        // like the DAW's own plugin windows.
        let owner = if app.hosted {
            // SAFETY: no preconditions; a null result means no owner.
            unsafe { GetAncestor(GetForegroundWindow(), GA_ROOTOWNER) }
        } else {
            std::ptr::null_mut()
        };
        let name = win::wide(&title);
        // SAFETY: a top-level window of the class registered above.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                style(false),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                640,
                420,
                owner,
                std::ptr::null_mut(),
                win::own_module(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(anyhow!("couldn't make a window for {title}"));
        }
        // SAFETY: `hwnd` is live.
        let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.0;

        let attached = {
            let mut slot = plugin.lock();
            match slot.plugin.editor() {
                Some(editor) => editor
                    .open(WindowHandle::HWND(hwnd.cast()), scale)
                    .map(|()| (editor.size(), editor.is_resizable()))
                    .map_err(anyhow::Error::from),
                None => Err(anyhow!("{title} has no editor")),
            }
        };
        let (size, resizable) = match attached {
            Ok(v) => v,
            Err(e) => {
                // SAFETY: our window, not shown yet.
                unsafe { DestroyWindow(hwnd) };
                return Err(e);
            }
        };
        if resizable {
            // SAFETY: `hwnd` is live; SWP_FRAMECHANGED applies the new frame.
            unsafe {
                SetWindowLongW(hwnd, GWL_STYLE, style(true) as i32);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }
        // The plugin's own window knows its size best; some report 1×1 at first and
        // size themselves later (`check` picks that up). Sizes are pixels on Windows.
        let big = |&(w, h): &(i32, i32)| f64::from(w) >= MIN_EDGE && f64::from(h) >= MIN_EDGE;
        let fit = plugin_view_size(hwnd)
            .filter(big)
            .or(size.map(|(w, h)| (w as i32, h as i32)).filter(big));
        if let Some((w, h)) = fit {
            set_client_size(hwnd, (w, h));
        }
        center(hwnd);
        show(hwnd);
        let seen = plugin_view_size(hwnd);
        EDITORS.with_borrow_mut(|eds| {
            eds.insert(
                k,
                Editor {
                    hwnd,
                    plugin,
                    resizable,
                    seen,
                    client: client_size(hwnd),
                },
            )
        });
        super::follow(k, check);
        Ok(())
    }

    enum Step {
        Closed(Editor),
        Resize(gs_engine::plugins::SharedPlugin, (i32, i32)),
        Keep,
    }

    /// Close the plugin's editor once its window was closed; fit the window to the
    /// plugin's when that changes size, or tell a resizable plugin when the user resized
    /// the window. Whether the editor is still open after this check.
    fn check(k: Key) -> bool {
        let step = EDITORS.with_borrow_mut(|eds| {
            let Some(ed) = eds.get_mut(&k) else {
                return Step::Keep;
            };
            // SAFETY: `ed.hwnd` is live until `tear_down`.
            let (visible, minimised) =
                unsafe { (IsWindowVisible(ed.hwnd) != 0, IsIconic(ed.hwnd) != 0) };
            if !visible && !minimised {
                return eds.remove(&k).map_or(Step::Keep, Step::Closed);
            }
            if minimised {
                return Step::Keep;
            }
            let now = plugin_view_size(ed.hwnd);
            if now != ed.seen {
                ed.seen = now;
                if let Some((w, h)) =
                    now.filter(|&(w, h)| f64::from(w) >= MIN_EDGE && f64::from(h) >= MIN_EDGE)
                {
                    set_client_size(ed.hwnd, (w, h));
                }
                ed.client = client_size(ed.hwnd);
                return Step::Keep;
            }
            let client = client_size(ed.hwnd);
            if ed.resizable && client != ed.client {
                ed.client = client;
                return Step::Resize(ed.plugin.clone(), client);
            }
            Step::Keep
        });
        match step {
            Step::Closed(ed) => {
                // The user closed the window.
                tear_down(ed);
                false
            }
            Step::Resize(plugin, (w, h)) => {
                // Outside the `EDITORS` borrow: the plugin may pump messages meanwhile.
                // It may pick a nearby size; the next check fits the window to it.
                if let Some(editor) = plugin.lock().plugin.editor() {
                    editor.set_size(w.max(1) as u32, h.max(1) as u32);
                }
                true
            }
            Step::Keep => EDITORS.with_borrow(|eds| eds.contains_key(&k)),
        }
    }

    /// Detach the plugin's GUI, then destroy the window. Outside the `EDITORS` borrow:
    /// a plugin may run the message loop while it closes.
    fn tear_down(ed: Editor) {
        if let Some(editor) = ed.plugin.lock().plugin.editor() {
            editor.close();
        }
        // SAFETY: our window; nothing uses it after this.
        unsafe { DestroyWindow(ed.hwnd) };
    }

    /// The size of the plugin's own window: the first child of ours.
    fn plugin_view_size(hwnd: HWND) -> Option<(i32, i32)> {
        // SAFETY: `hwnd` is live; `r` is a valid out-pointer.
        unsafe {
            let child = GetWindow(hwnd, GW_CHILD);
            if child.is_null() {
                return None;
            }
            let mut r: RECT = std::mem::zeroed();
            (GetWindowRect(child, &mut r) != 0).then(|| (r.right - r.left, r.bottom - r.top))
        }
    }

    fn client_size(hwnd: HWND) -> (i32, i32) {
        // SAFETY: `hwnd` is live; `r` is a valid out-pointer.
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut r);
            (r.right - r.left, r.bottom - r.top)
        }
    }

    /// The usable part of the screen under the mouse (or the window's).
    fn work_area() -> RECT {
        // SAFETY: valid out-pointers; `MonitorFromPoint` always returns a monitor here.
        unsafe {
            let mut cursor = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cursor);
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(monitor, &mut info);
            info.rcWork
        }
    }

    /// Make the window's inside `w × h` pixels, keeping it on screen (a few editors ask
    /// for more than a laptop has).
    fn set_client_size(hwnd: HWND, (w, h): (i32, i32)) {
        let area = work_area();
        // SAFETY: `hwnd` is live; `r` is a valid in/out pointer.
        unsafe {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            AdjustWindowRectExForDpi(&mut r, style, 0, ex, GetDpiForWindow(hwnd));
            let width = (r.right - r.left).min(area.right - area.left - 40);
            let height = (r.bottom - r.top).min(area.bottom - area.top - 40);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn center(hwnd: HWND) {
        let area = work_area();
        // SAFETY: `hwnd` is live; `r` is a valid out-pointer.
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let x = area.left + ((area.right - area.left - w) / 2).max(0);
            let y = area.top + ((area.bottom - area.top - h) / 2).max(0);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Close a node's editor (if open) before the node goes away.
    pub fn close(app: &App, id: u32) {
        let k = key(app, id);
        if let Some(ed) = EDITORS.with_borrow_mut(|eds| eds.remove(&k)) {
            tear_down(ed);
        }
    }

    /// Whether any of this app's plugin editors is open.
    pub fn any_open(app: &App) -> bool {
        let me = std::ptr::from_ref(app) as usize;
        EDITORS.with_borrow(|eds| eds.keys().any(|k| k.0 == me))
    }

    /// Close every editor of this app (it's shutting down).
    pub fn close_all(app: &App) {
        let me = std::ptr::from_ref(app) as usize;
        let mine: Vec<Editor> = EDITORS.with_borrow_mut(|eds| {
            let keys: Vec<Key> = eds.keys().filter(|k| k.0 == me).copied().collect();
            keys.into_iter().filter_map(|k| eds.remove(&k)).collect()
        });
        mine.into_iter().for_each(tear_down);
    }
}
