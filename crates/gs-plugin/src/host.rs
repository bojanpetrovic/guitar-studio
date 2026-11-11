//! The app side of one plugin instance: the shared app layer (`gs-app`) running inside
//! the DAW, its link to the plugin window, and the rig saved with the DAW's project.

use crate::bundle;
use gs_app::{App, Shell, main_thread, presets};
use gs_engine::presets::Patch;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use truce::core::snapshot::SnapshotSlot;

/// How often (in telemetry ticks of ~33 ms) a changed rig is re-saved for the DAW.
const PUBLISH_EVERY: u32 = 6;
/// While a hosted plugin's own editor is open its settings can change without us
/// hearing about it: re-save this often (ticks) then.
const PUBLISH_WHILE_EDITING: u32 = 60;

/// The plugin window's webview, as far as the app is concerned: where events go.
#[derive(Default)]
pub struct Ui {
    /// The plugin window is open (events are worth sending).
    pub open: AtomicBool,
    /// Latest telemetry not yet handed to the webview (sent at most once per main-loop
    /// turn, so a busy main thread never builds up a backlog of meter frames).
    telemetry: Mutex<Option<String>>,
    telemetry_queued: AtomicBool,
    /// The window's size, kept for the next time it opens (`None` = the default).
    pub size: Mutex<Option<(u32, u32)>>,
}

/// Events go to the plugin window's webview (see `editor.rs`).
struct PluginShell {
    id: usize,
    ui: Arc<Ui>,
}

impl Shell for PluginShell {
    fn emit(&self, event: &str, payload: Value) {
        if !self.ui.open.load(Ordering::Relaxed) {
            return;
        }
        let script = format!(
            "window.__gs&&window.__gs.event({},{})",
            Value::String(event.into()),
            payload
        );
        if event != "telemetry" {
            let id = self.id;
            main_thread::run(move || crate::editor::run_script(id, &script));
            return;
        }
        *self.ui.telemetry.lock() = Some(script);
        if !self.ui.telemetry_queued.swap(true, Ordering::AcqRel) {
            let (id, ui) = (self.id, self.ui.clone());
            main_thread::run(move || {
                ui.telemetry_queued.store(false, Ordering::Release);
                if let Some(script) = ui.telemetry.lock().take() {
                    crate::editor::run_script(id, &script);
                }
            });
        }
    }

    /// `gs-scan`, shipped inside the plugin bundle (the DAW can't scan for us).
    fn scanner(&self) -> std::io::Result<PathBuf> {
        let path = bundle::binary_dir()
            .ok_or_else(|| std::io::Error::other("couldn't find the plugin's folder"))?
            .join("gs-scan");
        if path.is_file() {
            Ok(path)
        } else {
            Err(std::io::Error::other(format!(
                "{} is missing",
                path.display()
            )))
        }
    }
}

/// The rig as saved in the DAW's project.
#[derive(Serialize, Deserialize)]
struct SavedRig {
    /// What this blob is; anything else is ignored.
    format: String,
    version: u32,
    /// The preset the rig came from (`None` = untitled).
    preset: Option<String>,
    patch: Patch,
}

const SAVED_FORMAT: &str = "guitar-studio-rig";

/// One plugin instance's app and its links to the plugin wrapper (cheap to clone).
#[derive(Clone)]
pub struct Instance {
    pub id: usize,
    pub app: Arc<App>,
    pub ui: Arc<Ui>,
    pub link: Arc<Link>,
}

/// What the plugin wrapper and the app exchange about the saved rig. Separate from
/// `App` so the telemetry pump can hold it without keeping the app alive.
pub struct Link {
    /// The DAW reads saved state from here (we publish; it never blocks audio).
    pub snapshot: Arc<SnapshotSlot>,
    /// State the DAW restored, waiting to be set up on the main thread.
    restored: Mutex<Option<Vec<u8>>>,
    has_restored: AtomicBool,
    /// The rig changed since it was last published.
    pub dirty: AtomicBool,
}

impl Instance {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let ui = Arc::new(Ui::default());
        let shell = || Box::new(PluginShell { id, ui: ui.clone() });
        // The desktop app's folder: same presets, same plugin list.
        let app = App::new(gs_app::default_data_dir(), shell(), true)
            .or_else(|e| {
                log::error!("can't use the Guitar Studio folder ({e:#}); using a temporary one");
                App::new(std::env::temp_dir().join("guitar-studio"), shell(), true)
            })
            .expect("a temporary folder for Guitar Studio");
        let link = Arc::new(Link {
            snapshot: SnapshotSlot::new(),
            restored: Mutex::new(None),
            has_restored: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
        });
        // Something to save even before the first change (an empty rig).
        publish(&app, &link);

        let pump_link = link.clone();
        let pumped = gs_app::spawn_pump(&app, move |app, _telemetry, tick| {
            pump(app, &pump_link, tick);
        });
        if let Err(e) = pumped {
            log::error!("the telemetry thread didn't start: {e}");
        }
        Self { id, app, ui, link }
    }
}

impl Link {
    /// The DAW restored a project (or a preset of its own): set the rig up soon, on the
    /// main thread. Called from the audio thread or the DAW's thread.
    pub fn restore(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        *self.restored.lock() = Some(data.to_vec());
        self.has_restored.store(true, Ordering::Release);
    }
}

/// Runs on every telemetry tick (off the main thread).
fn pump(app: &Arc<App>, link: &Arc<Link>, tick: u32) {
    if link.has_restored.swap(false, Ordering::AcqRel)
        && let Some(data) = link.restored.lock().take()
    {
        // Saving right after a restore must give back what was restored, even before
        // its plugins are up.
        link.snapshot.store_arc(data.clone());
        let (app, link) = (app.clone(), link.clone());
        main_thread::run(move || apply_restored(&app, &link, data));
        return;
    }
    if tick.is_multiple_of(PUBLISH_EVERY) {
        let check_editors = tick.is_multiple_of(PUBLISH_WHILE_EDITING);
        if link.dirty.load(Ordering::Relaxed) || check_editors {
            let (app, link) = (app.clone(), link.clone());
            main_thread::run(move || {
                let editing = check_editors && gs_app::editor::any_open(&app);
                if link.dirty.swap(false, Ordering::AcqRel) || editing {
                    publish(&app, &link);
                }
            });
        }
    }
}

/// Main thread: set up a rig the DAW restored.
fn apply_restored(app: &Arc<App>, link: &Arc<Link>, data: Vec<u8>) {
    let saved: SavedRig = match serde_json::from_slice(&data) {
        Ok(s) if is_ours(&s) => s,
        Ok(_) => return log::warn!("ignoring saved state that isn't a Guitar Studio rig"),
        Err(e) => return log::warn!("couldn't read the saved rig: {e}"),
    };
    match presets::load_patch(app, &saved.patch, saved.preset) {
        Ok(load) => {
            for w in &load.warnings {
                log::warn!("restoring the rig: {w}");
            }
            log::info!("restored the rig ({} nodes)", load.studio.nodes.len());
            app.emit("sync", &app.sync_state());
            // Save it again from the live rig (plugins may have adjusted their state).
            link.dirty.store(true, Ordering::Relaxed);
        }
        Err(e) if e.starts_with("Still loading") => {
            // A rig change is under way; try again on the next tick.
            *link.restored.lock() = Some(data);
            link.has_restored.store(true, Ordering::Release);
        }
        Err(e) => log::error!("couldn't restore the rig: {e}"),
    }
}

fn is_ours(s: &SavedRig) -> bool {
    s.format == SAVED_FORMAT && s.version == 1
}

/// Main thread: save the rig where the DAW will find it.
pub fn publish(app: &App, link: &Link) {
    let preset = app.current_preset.lock().clone();
    let (patch, _) = app
        .studio
        .lock()
        .to_patch(preset.as_deref().unwrap_or("Untitled"));
    let saved = SavedRig {
        format: SAVED_FORMAT.into(),
        version: 1,
        preset,
        patch,
    };
    match serde_json::to_vec(&saved) {
        Ok(bytes) => link.snapshot.store_arc(bytes),
        Err(e) => log::error!("couldn't save the rig: {e}"),
    }
}
