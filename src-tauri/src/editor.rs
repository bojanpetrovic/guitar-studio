//! Plugin editor windows: a native Tauri window (no webview) whose content `NSView`
//! becomes the parent of the plugin's own GUI.

use crate::AppState;
use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, LogicalSize, Manager, Window, WindowEvent};
use truce_rack::core::WindowHandle;

fn label(id: u32) -> String {
    format!("plugin-editor-{id}")
}

pub fn open(app: &AppHandle, state: &AppState, id: u32) -> Result<()> {
    if let Some(window) = app.get_window(&label(id)) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    let plugin = state
        .studio
        .lock()
        .plugin(id)
        .ok_or_else(|| anyhow!("node {id} is not a plugin"))?;
    let title = plugin.lock().descriptor.name.clone();

    let window = tauri::window::WindowBuilder::new(app, label(id))
        .title(&title)
        .inner_size(640.0, 420.0)
        .resizable(false)
        .visible(false)
        .build()?;

    let attach = || -> Result<(Option<(u32, u32)>, bool)> {
        #[cfg(target_os = "macos")]
        let parent = WindowHandle::NSView(window.ns_view()?);
        #[cfg(not(target_os = "macos"))]
        let parent: WindowHandle =
            return Err(anyhow!("plugin editors are only wired up on macOS so far"));
        let scale = window.scale_factor()?;
        let mut slot = plugin.lock();
        let editor = slot
            .plugin
            .editor()
            .ok_or_else(|| anyhow!("{title} has no editor"))?;
        editor.open(parent, scale)?;
        Ok((editor.size(), editor.is_resizable()))
    };
    let (size, resizable) = match attach() {
        Ok(v) => v,
        Err(e) => {
            let _ = window.destroy();
            return Err(e);
        }
    };
    // Some editors (JUCE-based ones such as Neural DSP's) report 1×1 at first and size
    // themselves a moment later: keep the default size for those; `follow_view_size`
    // picks up the real one.
    if let Some((w, h)) = size.filter(|&(w, h)| w >= MIN_EDGE as u32 && h >= MIN_EDGE as u32) {
        window.set_size(fit_to_screen(&window, (w as f64, h as f64)))?;
    }
    window.set_resizable(resizable)?;
    window.center()?;
    window.show()?;
    window.set_focus()?;

    let closed = Arc::new(AtomicBool::new(false));
    window.on_window_event({
        let closed = closed.clone();
        move |event| match event {
            WindowEvent::CloseRequested { .. } => {
                closed.store(true, Ordering::Relaxed);
                if let Some(editor) = plugin.lock().plugin.editor() {
                    editor.close();
                }
            }
            WindowEvent::Destroyed => closed.store(true, Ordering::Relaxed),
            _ => {}
        }
    });
    follow_view_size(window, closed);
    Ok(())
}

/// Smallest plausible editor edge, in points; anything less is a view still setting up.
const MIN_EDGE: f64 = 20.0;

/// Resize the window whenever the plugin's view changes size: late first layouts, and
/// plugins with their own zoom setting. Reads the view's frame on the main thread
/// without touching the plugin lock (the audio thread needs that).
fn follow_view_size(window: Window, closed: Arc<AtomicBool>) {
    let read = |window: &Window| {
        let (tx, rx) = std::sync::mpsc::channel();
        let w = window.clone();
        window
            .run_on_main_thread(move || {
                let _ = tx.send(plugin_view_size(&w));
            })
            .ok()?;
        rx.recv().ok()
    };
    let spawned = std::thread::Builder::new()
        .name("gs-editor-size".into())
        .spawn(move || {
            // Only react to changes: some views (Apple's generic AU view) report a frame
            // smaller than what they draw, and the size set at open already covers that.
            let mut seen = read(&window).flatten();
            while !closed.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let Some(now) = read(&window) else { break };
                if now == seen || closed.load(Ordering::Relaxed) {
                    continue;
                }
                seen = now;
                if let Some((w, h)) = now.filter(|&(w, h)| w >= MIN_EDGE && h >= MIN_EDGE) {
                    let _ = window.set_size(fit_to_screen(&window, (w, h)));
                    // Queued after the resize, so it sees the new content size.
                    let w = window.clone();
                    let _ = window.run_on_main_thread(move || pin_view_top(&w));
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("editor size watcher didn't start: {e}");
    }
}

/// The plugin view's size: the first subview of the window's content view.
#[cfg(target_os = "macos")]
fn plugin_view_size(window: &Window) -> Option<(f64, f64)> {
    use objc2_app_kit::NSView;
    let ptr = window.ns_view().ok()?;
    // SAFETY: Tauri hands out the live content view of this window, and we're on the
    // main thread (`run_on_main_thread`), where AppKit must be called.
    let parent: &NSView = unsafe { &*(ptr as *const NSView) };
    let frame = parent.subviews().firstObject()?.frame();
    Some((frame.size.width, frame.size.height))
}

/// Hang the plugin view from the window's top edge. AppKit anchors views bottom-left,
/// so an editor taller than the (screen-limited) window would lose its top bar; this way
/// it's the bottom that's cut.
#[cfg(target_os = "macos")]
fn pin_view_top(window: &Window) {
    use objc2_app_kit::NSView;
    use objc2_foundation::NSPoint;
    let Ok(ptr) = window.ns_view() else { return };
    // SAFETY: as in `plugin_view_size`; called on the main thread.
    let parent: &NSView = unsafe { &*(ptr as *const NSView) };
    if let Some(view) = parent.subviews().firstObject() {
        let top = parent.bounds().size.height - view.frame().size.height;
        view.setFrameOrigin(NSPoint::new(0.0, top));
    }
}

#[cfg(not(target_os = "macos"))]
fn pin_view_top(_window: &Window) {}

#[cfg(not(target_os = "macos"))]
fn plugin_view_size(_window: &Window) -> Option<(f64, f64)> {
    None
}

/// Keep an editor window on screen (a few editors ask for more than a laptop has).
fn fit_to_screen(window: &Window, (w, h): (f64, f64)) -> LogicalSize<f64> {
    let screen = window.current_monitor().ok().flatten().map(|m| {
        let size = m.size().to_logical::<f64>(m.scale_factor());
        (size.width - 40.0, size.height - 80.0)
    });
    match screen {
        Some((sw, sh)) => LogicalSize::new(w.min(sw), h.min(sh)),
        None => LogicalSize::new(w, h),
    }
}

/// Close a node's editor window (if open) before the node goes away.
pub fn close(app: &AppHandle, state: &AppState, id: u32) {
    if let Some(window) = app.get_window(&label(id)) {
        if let Some(plugin) = state.studio.lock().plugin(id)
            && let Some(editor) = plugin.lock().plugin.editor()
        {
            editor.close();
        }
        let _ = window.destroy();
    }
}
