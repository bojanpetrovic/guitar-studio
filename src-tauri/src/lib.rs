//! The desktop app: a Tauri window around the shared app layer (`gs-app`), plus the
//! phone remote. The UI reaches every command through `call` / `call_async`.

mod remote;

use gs_app::{App, Args, CmdResult, Shell, err, to_json};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct Desktop {
    app: Arc<App>,
    hub: Arc<remote::Hub>,
}

struct TauriShell(AppHandle);

impl Shell for TauriShell {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.0.emit(event, payload);
    }

    /// The app scans plugins by re-running itself (see `main.rs`).
    fn scanner(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}

/// Every UI command except the slow ones (`gs_app::runs_off_main`). Sync, so it runs
/// on the main thread, which plugin loading and editors need, in the order sent.
#[tauri::command]
fn call(
    handle: AppHandle,
    window: tauri::WebviewWindow,
    desktop: State<Desktop>,
    cmd: String,
    args: Value,
) -> CmdResult<Value> {
    let hub = &desktop.hub;
    let result = match cmd.as_str() {
        "set_window_buttons" => {
            set_window_buttons(&window, Args(args).get("visible")?).map(|()| Value::Null)
        }
        "remote_status" => to_json(hub.status(&handle)),
        "set_remote" => {
            let result = hub.set_enabled(&handle, Args(args).get("enabled")?);
            let mut status = hub.status(&handle);
            if let Err(e) = result {
                status.error = Some(e);
            }
            to_json(status)
        }
        // Make a new secret link; phones using the old one are disconnected.
        "renew_remote_link" => {
            hub.renew_token();
            to_json(hub.status(&handle))
        }
        _ => gs_app::dispatch(&desktop.app, &cmd, args),
    };
    // Desktop edits are mirrored to connected phones.
    if result.is_ok() && gs_app::changes_state(&cmd) {
        hub.desktop_changed();
    }
    result
}

/// The commands that must stay off the main thread (a plugin scan, quick reads that
/// shouldn't wait behind a plugin load).
#[tauri::command]
async fn call_async(desktop: State<'_, Desktop>, cmd: String, args: Value) -> CmdResult<Value> {
    let app = desktop.app.clone();
    tauri::async_runtime::spawn_blocking(move || gs_app::dispatch(&app, &cmd, args))
        .await
        .map_err(err)?
}

/// Show or hide the macOS close / minimise / zoom buttons. Stage mode hides them so the
/// window looks frameless. (We keep the native frame itself: taking decorations away
/// makes tao's traffic-light positioning panic on the next resize.) Windows has no
/// overlay title bar, so there the whole frame comes and goes.
fn set_window_buttons(window: &tauri::WebviewWindow, visible: bool) -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSWindow, NSWindowButton};
        let ptr = window.ns_window().map_err(err)?;
        // SAFETY: Tauri hands us the live NSWindow of this webview window, and sync
        // commands run on the main thread, where AppKit must be called.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = ns.standardWindowButton(kind) {
                button.setHidden(!visible);
            }
        }
        ns.invalidateShadow();
    }
    #[cfg(not(target_os = "macos"))]
    window.set_decorations(visible).map_err(err)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A release build on Windows has no console: log to a file there.
    let log = (cfg!(windows) && !cfg!(debug_assertions))
        .then(|| gs_app::local_dir().join("logs").join("app.log"));
    gs_app::logging::init(log.as_deref());
    // Plugins load and show their editors on this thread (see `gs_app::main_thread`).
    gs_app::main_thread::init();
    tauri::Builder::default()
        // Closing the main window quits, even with plugin editor windows still open
        // (otherwise audio and the phone server would run on, headless).
        .on_window_event(|window, event| {
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                window.app_handle().exit(0);
            }
        })
        .setup(|tauri_app| {
            let data_dir = tauri_app.path().app_data_dir()?;
            let handle = tauri_app.handle().clone();
            let app = App::new(
                data_dir.clone(),
                Box::new(TauriShell(handle.clone())),
                false,
            )?;
            let config = app.saved_audio_config();
            if let Err(e) = app.studio.lock().start_audio(config) {
                log::error!("audio failed to start: {e:#}");
            }
            let hub = Arc::new(remote::Hub::default());
            tauri_app.manage(Desktop {
                app: app.clone(),
                hub: hub.clone(),
            });
            hub.init(&handle, data_dir);

            // Meters go to connected phones too, and rig changes made on one side to
            // the other.
            gs_app::spawn_pump(&app, move |app, telemetry, tick| {
                hub.push_telemetry(telemetry);
                if tick.is_multiple_of(2) {
                    let (phones, desktop) = hub.take_stale();
                    if phones || desktop {
                        let sync = app.sync_state();
                        if phones {
                            hub.push_sync(&sync);
                        }
                        if desktop {
                            app.emit("sync", &sync);
                        }
                    }
                }
            })?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call, call_async])
        .run(tauri::generate_context!())
        .expect("error while running Guitar Studio");
}
