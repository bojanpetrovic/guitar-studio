mod editor;
mod presets;
mod remote;

use gs_engine::Studio;
use gs_engine::audio::{self, AudioConfig, DeviceList};
use gs_engine::metronome::MetronomeSettings;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{self, PluginDescriptor, SharedPlugin};
use gs_engine::studio::StudioView;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub plugins: Vec<PluginDescriptor>,
    pub errors: Vec<String>,
    pub scanned_at: Option<u64>,
    #[serde(default)]
    pub scanning: bool,
}

pub struct AppState {
    studio: Arc<Mutex<Studio>>,
    catalog: Mutex<Catalog>,
    /// Removed plugins wait here until the audio thread has let go of them, so the
    /// final drop happens on the main thread (AU / VST3 teardown expects that).
    graveyard: Mutex<Vec<SharedPlugin>>,
    data_dir: PathBuf,
    presets: gs_engine::presets::PresetStore,
    /// Name of the preset the current rig was loaded from / last saved as.
    current_preset: Mutex<Option<String>>,
    /// Bumped on every whole-rig swap (preset load / new), so other clients can tell a
    /// new rig from an edited one.
    rig: AtomicU64,
    remote: Arc<remote::Hub>,
}

impl AppState {
    fn catalog_path(&self) -> PathBuf {
        self.data_dir.join("plugins.json")
    }
    fn settings_path(&self) -> PathBuf {
        self.data_dir.join("audio.json")
    }
    fn reap(&self) {
        self.graveyard.lock().retain(|p| Arc::strong_count(p) > 1);
    }
}

// ---------------------------------------------------------------- studio & audio

#[tauri::command]
fn get_studio(state: State<AppState>) -> StudioView {
    state.studio.lock().view()
}

#[tauri::command]
fn list_devices() -> DeviceList {
    audio::list_devices()
}

#[tauri::command]
fn set_audio_config(state: State<AppState>, config: AudioConfig) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    let result = studio.start_audio(config.clone());
    if let Ok(json) = serde_json::to_vec_pretty(&config) {
        let _ = std::fs::write(state.settings_path(), json);
    }
    result.map_err(|e| format!("{e:#}"))?;
    Ok(studio.view())
}

// ---------------------------------------------------------------- plugins

#[tauri::command]
fn get_catalog(state: State<AppState>) -> Catalog {
    state.catalog.lock().clone()
}

#[tauri::command]
async fn scan_plugins(app: AppHandle) -> CmdResult<Catalog> {
    let state = app.state::<AppState>();
    {
        let mut c = state.catalog.lock();
        if c.scanning {
            return Ok(c.clone());
        }
        c.scanning = true;
    }
    let _ = app.emit("catalog", state.catalog.lock().clone());

    let exe = std::env::current_exe().map_err(err)?;
    let (plugins, errors) =
        tauri::async_runtime::spawn_blocking(move || plugins::scan_all_isolated(&exe))
            .await
            .map_err(err)?;
    let catalog = Catalog {
        plugins,
        errors,
        scanned_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        scanning: false,
    };
    if let Ok(json) = serde_json::to_vec(&catalog) {
        let _ = std::fs::write(state.catalog_path(), json);
    }
    *state.catalog.lock() = catalog.clone();
    let _ = app.emit("catalog", catalog.clone());
    Ok(catalog)
}

// ---------------------------------------------------------------- graph editing
// Non-async commands run on the main thread, which plugin loading and editors need.

#[tauri::command]
fn add_plugin_node(
    state: State<AppState>,
    key: String,
    x: f32,
    y: f32,
) -> CmdResult<(u32, StudioView)> {
    let desc = state
        .catalog
        .lock()
        .plugins
        .iter()
        .find(|p| p.key() == key)
        .cloned()
        .ok_or("plugin not found — try rescanning")?;
    let sample_rate = state.studio.lock().sample_rate();
    // Load without holding the studio lock: some plugins take seconds to start.
    let slot = plugins::load_plugin(&desc, sample_rate, MAX_BLOCK).map_err(|e| format!("{e:#}"))?;
    let mut studio = state.studio.lock();
    let id = studio.add_plugin_node(slot, x, y).map_err(err)?;
    Ok((id, studio.view()))
}

#[tauri::command]
fn add_gain_node(state: State<AppState>, x: f32, y: f32) -> CmdResult<(u32, StudioView)> {
    let mut studio = state.studio.lock();
    let id = studio.add_gain_node(x, y).map_err(err)?;
    Ok((id, studio.view()))
}

#[tauri::command]
fn add_mixer_node(state: State<AppState>, x: f32, y: f32) -> CmdResult<(u32, StudioView)> {
    let mut studio = state.studio.lock();
    let id = studio.add_mixer_node(x, y).map_err(err)?;
    Ok((id, studio.view()))
}

/// `field`: "gainDb" | "pan" | "mute".
#[tauri::command]
fn set_mixer_strip(
    state: State<AppState>,
    id: u32,
    source: u32,
    field: String,
    value: f32,
) -> CmdResult<()> {
    state
        .studio
        .lock()
        .set_mixer_strip(id, source, &field, value)
        .map_err(err)
}

#[tauri::command]
fn remove_node(app: AppHandle, state: State<AppState>, id: u32) -> CmdResult<StudioView> {
    editor::close(&app, &state, id);
    let mut studio = state.studio.lock();
    if let Some(plugin) = studio.remove_node(id).map_err(err)? {
        state.graveyard.lock().push(plugin);
    }
    Ok(studio.view())
}

#[tauri::command]
fn connect(state: State<AppState>, from: u32, to: u32) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.connect(from, to).map_err(err)?;
    Ok(studio.view())
}

#[tauri::command]
fn disconnect(state: State<AppState>, from: u32, to: u32) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.disconnect(from, to).map_err(err)?;
    Ok(studio.view())
}

#[tauri::command]
fn set_node_param(state: State<AppState>, id: u32, param: String, value: f32) -> CmdResult<()> {
    state
        .studio
        .lock()
        .set_param(id, &param, value)
        .map_err(err)
}

#[tauri::command]
fn set_bypass(state: State<AppState>, id: u32, bypass: bool) -> CmdResult<()> {
    state.studio.lock().set_bypass(id, bypass).map_err(err)
}

#[tauri::command]
fn move_node(state: State<AppState>, id: u32, x: f32, y: f32) {
    state.studio.lock().move_node(id, x, y);
}

#[tauri::command]
fn open_editor(app: AppHandle, state: State<AppState>, id: u32) -> CmdResult<()> {
    editor::open(&app, &state, id).map_err(|e| format!("{e:#}"))
}

/// Show or hide the macOS close / minimise / zoom buttons. Stage mode hides them so the
/// window looks frameless. (We keep the native frame itself: taking decorations away
/// makes tao's traffic-light positioning panic on the next resize.)
#[tauri::command]
fn set_window_buttons(window: tauri::WebviewWindow, visible: bool) -> CmdResult<()> {
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
    let _ = (window, visible);
    Ok(())
}

// ---------------------------------------------------------------- scenes

#[tauri::command]
fn add_scene(state: State<AppState>, name: Option<String>) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.add_scene(name.as_deref()).map_err(err)?;
    Ok(studio.view())
}

/// Sync (main thread): JUCE plugins apply parameter changes on the main run loop.
#[tauri::command]
fn select_scene(state: State<AppState>, index: usize) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.select_scene(index).map_err(err)?;
    Ok(studio.view())
}

#[tauri::command]
fn rename_scene(state: State<AppState>, index: usize, name: String) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.rename_scene(index, &name).map_err(err)?;
    Ok(studio.view())
}

#[tauri::command]
fn delete_scene(state: State<AppState>, index: usize) -> CmdResult<StudioView> {
    let mut studio = state.studio.lock();
    studio.delete_scene(index).map_err(err)?;
    Ok(studio.view())
}

// ---------------------------------------------------------------- pedalboard

#[tauri::command]
fn plugin_params(state: State<AppState>, id: u32) -> CmdResult<Vec<gs_engine::plugins::ParamInfo>> {
    state.studio.lock().plugin_params(id).map_err(err)
}

#[tauri::command]
fn set_plugin_param(state: State<AppState>, id: u32, param_id: u32, value: f64) -> CmdResult<()> {
    state
        .studio
        .lock()
        .set_plugin_param(id, param_id, value)
        .map_err(err)
}

/// `items` = `[node, paramId]` pairs; answers in the same order (`null` = gone).
#[tauri::command]
fn get_param_values(state: State<AppState>, items: Vec<(u32, u32)>) -> Vec<Option<f64>> {
    state.studio.lock().param_values(&items)
}

#[tauri::command]
fn set_pedalboard(state: State<AppState>, board: serde_json::Value) {
    state.studio.lock().set_pedalboard(board);
}

// ---------------------------------------------------------------- metronome & tuner

#[tauri::command]
fn set_metronome(state: State<AppState>, settings: MetronomeSettings) {
    state.studio.lock().set_metronome(settings);
}

#[tauri::command]
fn set_tuner(state: State<AppState>, enabled: bool, reference_a4: Option<f32>) {
    state.studio.lock().set_tuner(enabled, reference_a4);
}

// ---------------------------------------------------------------- phone remote

/// Everything a second screen needs to mirror the rig (pushed as the `sync` event).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    studio: StudioView,
    preset: Option<String>,
    rig: u64,
}

fn sync_state(state: &AppState) -> SyncState {
    SyncState {
        studio: state.studio.lock().view(),
        preset: state.current_preset.lock().clone(),
        rig: state.rig.load(Ordering::Relaxed),
    }
}

#[tauri::command]
fn remote_sync(state: State<AppState>) -> SyncState {
    sync_state(&state)
}

#[tauri::command]
fn remote_status(app: AppHandle, state: State<AppState>) -> remote::RemoteStatus {
    state.remote.status(&app)
}

#[tauri::command]
fn set_remote(app: AppHandle, state: State<AppState>, enabled: bool) -> remote::RemoteStatus {
    let result = state.remote.set_enabled(&app, enabled);
    let mut status = state.remote.status(&app);
    if let Err(e) = result {
        status.error = Some(e);
    }
    status
}

/// Make a new secret link; phones using the old one are disconnected.
#[tauri::command]
fn renew_remote_link(app: AppHandle, state: State<AppState>) -> CmdResult<remote::RemoteStatus> {
    state.remote.renew_token(&app)?;
    Ok(state.remote.status(&app))
}

// ---------------------------------------------------------------- app

/// Pins down the handler's type so it can be wrapped (see `invoke_handler` below).
fn desktop_commands<F: Fn(tauri::ipc::Invoke) -> bool>(f: F) -> F {
    f
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hub = Arc::new(remote::Hub::default());
    let commands = desktop_commands(tauri::generate_handler![
        get_studio,
        list_devices,
        set_audio_config,
        get_catalog,
        scan_plugins,
        add_plugin_node,
        add_gain_node,
        add_mixer_node,
        set_mixer_strip,
        remove_node,
        connect,
        disconnect,
        set_node_param,
        set_bypass,
        move_node,
        open_editor,
        set_window_buttons,
        add_scene,
        select_scene,
        rename_scene,
        delete_scene,
        set_metronome,
        set_tuner,
        plugin_params,
        set_plugin_param,
        get_param_values,
        set_pedalboard,
        remote_sync,
        remote_status,
        set_remote,
        renew_remote_link,
        presets::list_presets,
        presets::save_preset,
        presets::load_preset,
        presets::new_preset,
        presets::delete_preset,
        presets::rename_preset,
    ]);
    let desktop_hub = hub.clone();
    tauri::Builder::default()
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let catalog: Catalog = std::fs::read(data_dir.join("plugins.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .map(|c: Catalog| Catalog {
                    scanning: false,
                    ..c
                })
                .unwrap_or_default();
            let audio_config: AudioConfig = std::fs::read(data_dir.join("audio.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(AudioConfig {
                    buffer_size: 128,
                    ..Default::default()
                });

            let mut studio = Studio::new();
            if let Err(e) = studio.start_audio(audio_config) {
                log::error!("audio failed to start: {e:#}");
            }
            let studio = Arc::new(Mutex::new(studio));
            let presets = gs_engine::presets::PresetStore::new(data_dir.join("presets"))?;
            let data_dir_for_remote = data_dir.clone();
            app.manage(AppState {
                studio: studio.clone(),
                catalog: Mutex::new(catalog),
                graveyard: Mutex::new(Vec::new()),
                data_dir,
                presets,
                current_preset: Mutex::new(None),
                rig: AtomicU64::new(0),
                remote: hub.clone(),
            });
            hub.init(app.handle(), data_dir_for_remote);

            // Telemetry pump: meters, tuner and metronome beat at ~30 Hz, to the window
            // and to connected phones; plus rig changes made on one side for the other.
            let handle = app.handle().clone();
            let hub = hub.clone();
            std::thread::Builder::new()
                .name("gs-telemetry".into())
                .spawn(move || {
                    let mut tick = 0u32;
                    loop {
                        std::thread::sleep(Duration::from_millis(33));
                        let t = {
                            let mut s = studio.lock();
                            s.collect_garbage();
                            s.telemetry()
                        };
                        let _ = handle.emit("telemetry", &t);
                        hub.push_telemetry(&t);
                        tick = tick.wrapping_add(1);
                        if tick.is_multiple_of(2) {
                            let (phones, desktop) = hub.take_stale();
                            if phones || desktop {
                                let sync = sync_state(&handle.state::<AppState>());
                                if phones {
                                    hub.push_sync(&sync);
                                }
                                if desktop {
                                    let _ = handle.emit("sync", &sync);
                                }
                            }
                        }
                        if tick.is_multiple_of(30) {
                            let h = handle.clone();
                            let _ = handle.run_on_main_thread(move || h.state::<AppState>().reap());
                        }
                    }
                })?;
            Ok(())
        })
        // Desktop edits are mirrored to connected phones.
        .invoke_handler(move |invoke| {
            let notify = remote::changes_state(invoke.message.command());
            let handled = commands(invoke);
            if notify {
                desktop_hub.desktop_changed();
            }
            handled
        })
        .run(tauri::generate_context!())
        .expect("error while running Guitar Studio");
}
