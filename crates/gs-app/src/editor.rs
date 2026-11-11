//! Plugin editor windows: a plain AppKit window whose content `NSView` becomes the
//! parent of the plugin's own GUI. Plain AppKit (not the front end's toolkit) so the
//! same windows work in the desktop app and inside a plugin host.
//!
//! Main thread only.

use crate::App;
use anyhow::Result;

/// Smallest plausible editor edge, in points; anything less is a view still setting up.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MIN_EDGE: f64 = 20.0;

#[cfg(target_os = "macos")]
pub use mac::{any_open, close, close_all, open};

#[cfg(not(target_os = "macos"))]
pub fn open(_app: &App, _id: u32) -> Result<()> {
    Err(anyhow::anyhow!(
        "plugin editors are only wired up on macOS so far"
    ))
}
#[cfg(not(target_os = "macos"))]
pub fn close(_app: &App, _id: u32) {}
#[cfg(not(target_os = "macos"))]
pub fn close_all(_app: &App) {}
#[cfg(not(target_os = "macos"))]
pub fn any_open(_app: &App) -> bool {
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{App, MIN_EDGE, Result};
    use crate::main_thread;
    use anyhow::anyhow;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSFloatingWindowLevel, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::Duration;
    use truce_rack::core::WindowHandle;

    /// One open editor: its window and the plugin it shows.
    struct Editor {
        window: Retained<NSWindow>,
        plugin: gs_engine::plugins::SharedPlugin,
        /// Last size the plugin's view reported (to react only to changes).
        seen: Option<(f64, f64)>,
    }

    /// Open editors by (app instance, node). Several apps exist when the plugin host has
    /// more than one Guitar Studio loaded.
    type Key = (usize, u32);

    thread_local! {
        static EDITORS: RefCell<HashMap<Key, Editor>> = RefCell::new(HashMap::new());
    }

    fn key(app: &App, id: u32) -> Key {
        (std::ptr::from_ref(app) as usize, id)
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
        follow(k);
        Ok(())
    }

    /// Every 250 ms, on the main thread: close the plugin's editor once its window was
    /// closed, and resize the window when the plugin's view changes size (late first
    /// layouts, plugins with their own zoom setting).
    fn follow(k: Key) {
        let spawned = std::thread::Builder::new()
            .name("gs-editor-watch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    let open = main_thread::run_and_wait(move || check(k)).unwrap_or(false);
                    if !open {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("editor watcher didn't start: {e}");
        }
    }

    /// Whether the editor is still open after this check.
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
