//! Plugin editor windows: a native Tauri window (no webview) whose content `NSView`
//! becomes the parent of the plugin's own GUI.

use crate::AppState;
use anyhow::{Result, anyhow};
use tauri::{AppHandle, LogicalSize, Manager, WindowEvent};
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
    if let Some((w, h)) = size {
        window.set_size(LogicalSize::new(w as f64, h as f64))?;
    }
    window.set_resizable(resizable)?;
    window.center()?;
    window.show()?;
    window.set_focus()?;

    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { .. } = event
            && let Some(editor) = plugin.lock().plugin.editor()
        {
            editor.close();
        }
    });
    Ok(())
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
