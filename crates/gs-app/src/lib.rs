//! Guitar Studio's app layer: everything between the UI and the engine that doesn't
//! depend on the window toolkit — the app state, the commands the UI sends (one
//! [`dispatch`] for all of them), presets, fast switching and plugin editor windows.
//!
//! Two front ends use it: the desktop app (Tauri, `src-tauri`) and the plugin that
//! runs Guitar Studio inside a DAW (`crates/gs-plugin`). Each provides a [`Shell`],
//! which carries events to its UI and finds the plugin scanner.
//!
//! Threading: commands that start plugins, open editors or swap the rig must run on
//! the main thread (see [`runs_off_main`] for the few that mustn't). Background work
//! that needs the main thread goes through [`main_thread`].

pub mod editor;
pub mod logging;
pub mod main_thread;
pub mod presets;
pub mod warm;

use gs_engine::Studio;
use gs_engine::audio::{self, AudioConfig};
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{self, PluginDescriptor, SharedPlugin};
use gs_engine::presets::PresetStore;
use gs_engine::studio::{StudioView, TelemetryView};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

pub type CmdResult<T> = Result<T, String>;

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Errors with their whole cause chain (`anyhow`'s `{:#}`).
fn chain(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// What a front end provides.
pub trait Shell: Send + Sync + 'static {
    /// Send an event to the UI (`catalog`, `warming`, `telemetry`, `sync`).
    fn emit(&self, event: &str, payload: Value);
    /// The program that scans plugins in a child process: it is run as
    /// `<program> --scan-plugins <format>` (see `gs_engine::plugins::run_scan_child`).
    fn scanner(&self) -> std::io::Result<PathBuf>;
}

/// The app's data folder (presets, plugin list, settings), shared by the desktop app
/// and the plugin. Matches the Tauri identifier in `src-tauri/tauri.conf.json`.
pub fn default_data_dir() -> PathBuf {
    const IDENTIFIER: &str = "dev.guitarstudio.app";
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support").join(IDENTIFIER)
    } else {
        home.join(".local/share").join(IDENTIFIER)
    }
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

/// Everything a second screen needs to mirror the rig (pushed as the `sync` event).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub studio: StudioView,
    pub preset: Option<String>,
    pub rig: u64,
}

pub struct App {
    pub studio: Arc<Mutex<Studio>>,
    pub catalog: Mutex<Catalog>,
    /// Removed plugins wait here until the audio thread has let go of them, so the
    /// final drop happens on the main thread (AU / VST3 teardown expects that).
    graveyard: Mutex<Vec<SharedPlugin>>,
    pub data_dir: PathBuf,
    pub presets: PresetStore,
    /// Name of the preset the current rig was loaded from / last saved as.
    pub current_preset: Mutex<Option<String>>,
    /// Bumped on every whole-rig swap (preset load / new), so other clients can tell a
    /// new rig from an edited one.
    pub rig: AtomicU64,
    /// Spare plugin instances for fast preset switching.
    pub warm: warm::Warm,
    pub shell: Box<dyn Shell>,
    /// Running inside a plugin host: the host drives the audio, and the rig is saved
    /// with the host's project rather than remembered as "the last preset".
    pub hosted: bool,
}

impl App {
    /// Set up the app over `data_dir` (created if missing). Audio isn't started.
    pub fn new(
        data_dir: PathBuf,
        shell: Box<dyn Shell>,
        hosted: bool,
    ) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        let catalog: Catalog = read_json(&data_dir.join(CATALOG_FILE))
            .map(|c: Catalog| Catalog {
                scanning: false,
                ..c
            })
            .unwrap_or_default();
        let presets = PresetStore::new(data_dir.join("presets"))?;
        Ok(Arc::new(Self {
            studio: Arc::new(Mutex::new(Studio::new())),
            catalog: Mutex::new(catalog),
            graveyard: Mutex::new(Vec::new()),
            data_dir,
            presets,
            current_preset: Mutex::new(None),
            rig: AtomicU64::new(0),
            warm: warm::Warm::new(!hosted),
            shell,
            hosted,
        }))
    }

    pub fn settings_path(&self) -> PathBuf {
        self.data_dir.join("audio.json")
    }

    /// The saved audio device settings (desktop app).
    pub fn saved_audio_config(&self) -> AudioConfig {
        read_json(&self.settings_path()).unwrap_or(AudioConfig {
            buffer_size: 128,
            ..Default::default()
        })
    }

    pub fn emit<T: Serialize>(&self, event: &str, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(v) => self.shell.emit(event, v),
            Err(e) => log::warn!("couldn't send the {event} event: {e}"),
        }
    }

    /// Drop removed plugins the audio thread has let go of. Main thread only.
    pub fn reap(&self) {
        self.graveyard.lock().retain(|p| Arc::strong_count(p) > 1);
    }

    pub fn bury(&self, plugins: impl IntoIterator<Item = SharedPlugin>) {
        self.graveyard.lock().extend(plugins);
    }

    pub fn sync_state(&self) -> SyncState {
        SyncState {
            studio: self.studio.lock().view(),
            preset: self.current_preset.lock().clone(),
            rig: self.rig.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn next_rig(&self) -> u64 {
        self.rig.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn find_plugin(&self, key: &str) -> CmdResult<PluginDescriptor> {
        self.catalog
            .lock()
            .plugins
            .iter()
            .find(|p| p.key() == key)
            .cloned()
            .ok_or_else(|| "plugin not found — try rescanning".into())
    }

    /// Release every plugin (closing editors first). Main thread only: the plugin host
    /// is unloading us, and plugins must be torn down on the main thread.
    pub fn shut_down(&self) {
        self.warm.stop();
        editor::close_all(self);
        let old = {
            let mut studio = self.studio.lock();
            studio.stop_audio();
            studio.reset_graph().unwrap_or_default()
        };
        drop(old);
        drop(self.warm.take());
        self.graveyard.lock().clear();
    }
}

const CATALOG_FILE: &str = "plugins.json";

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

/// The telemetry pump: meters, the tuner and the metronome beat at ~30 Hz to the UI
/// (`telemetry` event). `each` runs after every tick, with its number. Stops when the
/// app is dropped.
pub fn spawn_pump(
    app: &Arc<App>,
    each: impl Fn(&Arc<App>, &TelemetryView, u32) + Send + 'static,
) -> std::io::Result<()> {
    let app: Weak<App> = Arc::downgrade(app);
    std::thread::Builder::new()
        .name("gs-telemetry".into())
        .spawn(move || {
            let mut tick = 0u32;
            loop {
                std::thread::sleep(Duration::from_millis(33));
                let Some(app) = app.upgrade() else { break };
                let t = {
                    let mut s = app.studio.lock();
                    s.collect_garbage();
                    s.telemetry()
                };
                app.emit("telemetry", &t);
                each(&app, &t, tick);
                tick = tick.wrapping_add(1);
                if tick.is_multiple_of(30) {
                    let app = app.clone();
                    main_thread::run(move || app.reap());
                }
            }
        })
        .map(|_| ())
}

// ---------------------------------------------------------------- rig guard

thread_local! {
    static RIG_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Held (on the main thread) while a command starts plugins or swaps the rig. Some
/// plugins pump the main run loop while they start (AU v3 async instantiation), which
/// can run another command in the middle of this one; a second rig change arriving
/// then is refused instead of interleaving with the first.
pub struct RigGuard(());

impl RigGuard {
    pub fn take() -> CmdResult<Self> {
        if RIG_BUSY.with(|busy| busy.replace(true)) {
            return Err("Still loading the last change — try again in a moment".into());
        }
        Ok(Self(()))
    }
}

impl Drop for RigGuard {
    fn drop(&mut self) {
        RIG_BUSY.with(|busy| busy.set(false));
    }
}

// ---------------------------------------------------------------- commands

/// Commands that must *not* run on the main thread: they take long without needing
/// it (a plugin scan), or only read and shouldn't queue behind a plugin load.
pub fn runs_off_main(cmd: &str) -> bool {
    matches!(cmd, "scan_plugins" | "preset_ready")
}

/// Whether a command can change what [`App::sync_state`] returns (so other screens need
/// telling). Plugin parameters aren't in it: boards poll those.
pub fn changes_state(cmd: &str) -> bool {
    !(cmd.starts_with("get_")
        || cmd.starts_with("list_")
        || cmd.starts_with("remote_")
        || matches!(
            cmd,
            "plugin_params"
                | "preset_ready"
                | "set_plugin_param"
                | "open_editor"
                | "set_window_buttons"
                | "scan_plugins"
                | "set_remote"
                | "renew_remote_link"
        ))
}

/// The UI's arguments: a JSON object with camelCase names.
pub struct Args(pub Value);

impl Args {
    /// A named argument. Missing = null, so `Option<T>` arguments may be left out.
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> CmdResult<T> {
        let v = self.0.get(name).cloned().unwrap_or(Value::Null);
        serde_json::from_value(v).map_err(|e| format!("argument `{name}`: {e}"))
    }
}

pub fn to_json<T: Serialize>(v: T) -> CmdResult<Value> {
    serde_json::to_value(v).map_err(err)
}

/// Run one UI command. See [`runs_off_main`] for which thread to call it on.
pub fn dispatch(app: &Arc<App>, cmd: &str, args: Value) -> CmdResult<Value> {
    let a = Args(args);
    let studio = || app.studio.lock();
    match cmd {
        // studio & audio
        "get_studio" => to_json(studio().view()),
        "remote_sync" => to_json(app.sync_state()),
        "list_devices" => to_json(audio::list_devices()),
        "set_audio_config" => to_json(set_audio_config(app, a.get("config")?)?),

        // plugins
        "get_catalog" => to_json(app.catalog.lock().clone()),
        "scan_plugins" => to_json(scan_plugins(app)?),

        // graph editing
        "add_plugin_node" => to_json(add_plugin_node(
            app,
            &a.get::<String>("key")?,
            a.get("x")?,
            a.get("y")?,
        )?),
        "add_gain_node" => {
            let mut s = studio();
            let id = s.add_gain_node(a.get("x")?, a.get("y")?).map_err(err)?;
            to_json((id, s.view()))
        }
        "add_mixer_node" => {
            let mut s = studio();
            let id = s.add_mixer_node(a.get("x")?, a.get("y")?).map_err(err)?;
            to_json((id, s.view()))
        }
        "set_mixer_strip" => to_json(
            studio()
                .set_mixer_strip(
                    a.get("id")?,
                    a.get("source")?,
                    &a.get::<String>("field")?,
                    a.get("value")?,
                )
                .map_err(err)?,
        ),
        "replace_plugin" => to_json(replace_plugin(app, a.get("id")?, &a.get::<String>("key")?)?),
        "remove_node" => to_json(remove_node(app, a.get("id")?)?),
        "connect" => {
            let mut s = studio();
            s.connect(a.get("from")?, a.get("to")?).map_err(err)?;
            to_json(s.view())
        }
        "disconnect" => {
            let mut s = studio();
            s.disconnect(a.get("from")?, a.get("to")?).map_err(err)?;
            to_json(s.view())
        }
        "set_node_param" => to_json(
            studio()
                .set_param(a.get("id")?, &a.get::<String>("param")?, a.get("value")?)
                .map_err(err)?,
        ),
        "set_bypass" => to_json(
            studio()
                .set_bypass(a.get("id")?, a.get("bypass")?)
                .map_err(err)?,
        ),
        "move_node" => {
            studio().move_node(a.get("id")?, a.get("x")?, a.get("y")?);
            Ok(Value::Null)
        }
        "open_editor" => to_json(editor::open(app, a.get("id")?).map_err(chain)?),

        // scenes (selecting is main-thread: JUCE plugins apply parameter changes there)
        "add_scene" => {
            let mut s = studio();
            s.add_scene(a.get::<Option<String>>("name")?.as_deref())
                .map_err(err)?;
            to_json(s.view())
        }
        "select_scene" => {
            let mut s = studio();
            s.select_scene(a.get("index")?).map_err(err)?;
            to_json(s.view())
        }
        "rename_scene" => {
            let mut s = studio();
            s.rename_scene(a.get("index")?, &a.get::<String>("name")?)
                .map_err(err)?;
            to_json(s.view())
        }
        "delete_scene" => {
            let mut s = studio();
            s.delete_scene(a.get("index")?).map_err(err)?;
            to_json(s.view())
        }

        // pedalboard
        "plugin_params" => to_json(studio().plugin_params(a.get("id")?).map_err(err)?),
        "set_plugin_param" => to_json(
            studio()
                .set_plugin_param(a.get("id")?, a.get("paramId")?, a.get("value")?)
                .map_err(err)?,
        ),
        // `items` = `[node, paramId]` pairs; answers in the same order (`null` = gone).
        "get_param_values" => to_json(studio().param_values(&a.get::<Vec<(u32, u32)>>("items")?)),
        "set_pedalboard" => {
            studio().set_pedalboard(a.get("board")?);
            Ok(Value::Null)
        }

        // metronome & tuner
        "set_metronome" => {
            studio().set_metronome(a.get("settings")?);
            Ok(Value::Null)
        }
        "set_tuner" => {
            studio().set_tuner(a.get("enabled")?, a.get("referenceA4")?);
            Ok(Value::Null)
        }

        // presets
        "preset_ready" => to_json(preset_is_ready(app, &a.get::<String>("name")?)),
        "list_presets" => to_json(presets::list(app)),
        "save_preset" => to_json(presets::save(
            app,
            &a.get::<String>("name")?,
            a.get("overwrite")?,
        )?),
        "load_preset" => to_json(presets::load(app, &a.get::<String>("name")?)?),
        "new_preset" => to_json(presets::new(app)?),
        "delete_preset" => to_json(presets::delete(app, &a.get::<String>("name")?)?),
        "rename_preset" => to_json(presets::rename(
            app,
            &a.get::<String>("from")?,
            &a.get::<String>("to")?,
        )?),

        _ => Err(format!("unknown command “{cmd}”")),
    }
}

fn set_audio_config(app: &Arc<App>, config: AudioConfig) -> CmdResult<StudioView> {
    if app.hosted {
        return Err("The plugin host handles audio — change devices in its settings".into());
    }
    let mut studio = app.studio.lock();
    let rate = studio.sample_rate();
    let result = studio.start_audio(config.clone());
    if (studio.sample_rate() - rate).abs() > 0.5 {
        // Spares would still run at the old rate (the rig's plugins were reactivated):
        // drop them and preload fresh ones.
        app.warm.clear(app);
        warm::refresh(app);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&config) {
        let _ = std::fs::write(app.settings_path(), json);
    }
    result.map_err(chain)?;
    Ok(studio.view())
}

/// Blocking (minutes for a big plugin folder): call off the main thread.
fn scan_plugins(app: &Arc<App>) -> CmdResult<Catalog> {
    {
        let mut c = app.catalog.lock();
        if c.scanning {
            return Ok(c.clone());
        }
        c.scanning = true;
    }
    app.emit("catalog", &*app.catalog.lock());

    let scanned = app
        .shell
        .scanner()
        .map(|exe| plugins::scan_all_isolated(&exe))
        .map_err(|e| format!("couldn't start the plugin scanner: {e}"));
    let (mut found, errors) = match scanned {
        Ok(found) => found,
        Err(e) => {
            // Keep the old list; never leave the catalog stuck "scanning".
            let catalog = {
                let mut c = app.catalog.lock();
                c.scanning = false;
                c.errors = vec![e.clone()];
                c.clone()
            };
            app.emit("catalog", &catalog);
            return Err(e);
        }
    };
    // Guitar Studio's own plugin would host itself.
    found.retain(|p| !is_guitar_studio(p));
    let catalog = Catalog {
        plugins: found,
        errors,
        scanned_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        scanning: false,
    };
    if let Ok(json) = serde_json::to_vec(&catalog) {
        let _ = std::fs::write(app.data_dir.join(CATALOG_FILE), json);
    }
    *app.catalog.lock() = catalog.clone();
    app.emit("catalog", &catalog);
    Ok(catalog)
}

/// Our own plugin, as it shows up in a scan.
fn is_guitar_studio(p: &PluginDescriptor) -> bool {
    p.name.eq_ignore_ascii_case("Guitar Studio")
}

fn add_plugin_node(app: &Arc<App>, key: &str, x: f32, y: f32) -> CmdResult<(u32, StudioView)> {
    let _busy = RigGuard::take()?;
    let desc = app.find_plugin(key)?;
    let sample_rate = app.studio.lock().sample_rate();
    // Load without holding the studio lock: some plugins take seconds to start.
    let slot = plugins::load_plugin(&desc, sample_rate, MAX_BLOCK).map_err(chain)?;
    let mut studio = app.studio.lock();
    let id = studio.add_plugin_node(slot, x, y).map_err(err)?;
    Ok((id, studio.view()))
}

/// Swap the plugin in node `id` for another (`key` from the catalog), keeping the
/// node's cables and place.
fn replace_plugin(app: &Arc<App>, id: u32, key: &str) -> CmdResult<StudioView> {
    let _busy = RigGuard::take()?;
    let desc = app.find_plugin(key)?;
    let sample_rate = {
        let studio = app.studio.lock();
        if !studio.is_plugin(id) {
            return Err("only plugins can be replaced".into());
        }
        studio.sample_rate()
    };
    editor::close(app, id);
    // Load without holding the studio lock: some plugins take seconds to start.
    let slot = plugins::load_plugin(&desc, sample_rate, MAX_BLOCK).map_err(chain)?;
    let mut studio = app.studio.lock();
    let old = studio
        .replace_plugin(id, Arc::new(Mutex::new(slot)))
        .map_err(err)?;
    // The old instance may still be wanted by a neighbouring preset.
    app.warm.put_back(vec![old]);
    warm::refresh(app);
    Ok(studio.view())
}

fn remove_node(app: &Arc<App>, id: u32) -> CmdResult<StudioView> {
    editor::close(app, id);
    let mut studio = app.studio.lock();
    if let Some(plugin) = studio.remove_node(id).map_err(err)? {
        app.bury([plugin]);
    }
    Ok(studio.view())
}

/// Would loading this preset need to start any plugin? (If not it's instant, and the UI
/// skips the "Loading…" notice.)
pub fn preset_is_ready(app: &App, name: &str) -> bool {
    app.presets
        .plugins_of(name)
        .is_ok_and(|plugins| app.warm.covers(app, &plugins))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_state_changing_commands_notify() {
        assert!(changes_state("select_scene"));
        assert!(changes_state("set_mixer_strip"));
        assert!(changes_state("load_preset"));
        assert!(!changes_state("get_param_values"));
        assert!(!changes_state("set_plugin_param"));
        assert!(!changes_state("remote_sync"));
        assert!(!changes_state("preset_ready"));
    }

    #[test]
    fn missing_arguments_read_as_null() {
        let a = Args(serde_json::json!({ "id": 3 }));
        assert_eq!(a.get::<u32>("id"), Ok(3));
        assert_eq!(a.get::<Option<String>>("name"), Ok(None));
        assert!(a.get::<u32>("x").is_err());
    }
}
