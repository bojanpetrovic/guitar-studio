mod editor;

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

// ---------------------------------------------------------------- metronome & tuner

#[tauri::command]
fn set_metronome(state: State<AppState>, settings: MetronomeSettings) {
    state.studio.lock().set_metronome(settings);
}

#[tauri::command]
fn set_tuner(state: State<AppState>, enabled: bool, reference_a4: Option<f32>) {
    state.studio.lock().set_tuner(enabled, reference_a4);
}

// ---------------------------------------------------------------- app

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
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
            app.manage(AppState {
                studio: studio.clone(),
                catalog: Mutex::new(catalog),
                graveyard: Mutex::new(Vec::new()),
                data_dir,
            });

            // Telemetry pump: meters, tuner and metronome beat at ~30 Hz.
            let handle = app.handle().clone();
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
                        let _ = handle.emit("telemetry", t);
                        tick = tick.wrapping_add(1);
                        if tick.is_multiple_of(30) {
                            let h = handle.clone();
                            let _ = handle.run_on_main_thread(move || h.state::<AppState>().reap());
                        }
                    }
                })?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_studio,
            list_devices,
            set_audio_config,
            get_catalog,
            scan_plugins,
            add_plugin_node,
            add_gain_node,
            remove_node,
            connect,
            disconnect,
            set_node_param,
            set_bypass,
            move_node,
            open_editor,
            set_metronome,
            set_tuner,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Guitar Studio");
}
