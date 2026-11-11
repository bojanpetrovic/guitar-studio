//! Preset commands. Loading runs on the main thread because it instantiates plugins;
//! plugins are loaded *before* taking the studio lock so the meters keep running while
//! a heavy amp sim boots.

use crate::{App, CmdResult, RigGuard, chain, editor, warm};
use gs_engine::presets::{self, Patch, PresetInfo, PresetStore};
use gs_engine::studio::StudioView;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Returned when saving would overwrite a different preset; the UI asks first.
const EXISTS: &str = "PRESET_EXISTS";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetList {
    pub presets: Vec<PresetInfo>,
    pub current: Option<String>,
    /// Preset that was active when the app last quit (for auto-restore on launch).
    /// Never set inside a plugin host: there the project restores the rig.
    pub last: Option<String>,
    pub dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetLoad {
    pub studio: StudioView,
    pub presets: PresetList,
    pub warnings: Vec<String>,
    /// Rig generation (see `App::rig`).
    pub rig: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    last_preset: Option<String>,
}

fn session_path(app: &App) -> std::path::PathBuf {
    app.data_dir.join("session.json")
}

fn read_session(app: &App) -> Session {
    if app.hosted {
        return Session::default();
    }
    crate::read_json(&session_path(app)).unwrap_or_default()
}

pub(crate) fn set_current(app: &App, name: Option<String>) {
    *app.current_preset.lock() = name.clone();
    // The desktop app reopens with the last preset; a plugin's rig is in the project.
    if app.hosted {
        return;
    }
    let session = Session { last_preset: name };
    if let Ok(json) = serde_json::to_vec_pretty(&session) {
        let _ = std::fs::write(session_path(app), json);
    }
}

pub fn list(app: &App) -> PresetList {
    PresetList {
        presets: app.presets.list(),
        current: app.current_preset.lock().clone(),
        last: read_session(app).last_preset,
        dir: app.presets.dir().display().to_string(),
    }
}

/// Close every open plugin editor before the graph is swapped out.
fn close_all_editors(app: &App) {
    let ids = app.studio.lock().plugin_node_ids();
    for id in ids {
        editor::close(app, id);
    }
}

/// Save the current rig. `overwrite` must be true to replace a *different* existing
/// preset (re-saving the current one never needs it).
pub fn save(app: &Arc<App>, name: &str, overwrite: bool) -> CmdResult<(PresetList, Vec<String>)> {
    let name = PresetStore::clean_name(name).map_err(|e| e.to_string())?;
    let is_current = app
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(&name));
    if !overwrite && !is_current && app.presets.exists(&name) {
        return Err(EXISTS.into());
    }
    let (patch, warnings) = app.studio.lock().to_patch(&name);
    app.presets.save(&patch).map_err(chain)?;
    set_current(app, Some(name));
    // The neighbours of the current preset may have changed.
    warm::refresh(app);
    Ok((list(app), warnings))
}

/// Load a preset (the user chose it).
pub fn load(app: &Arc<App>, name: &str) -> CmdResult<PresetLoad> {
    let patch = app.presets.load(name).map_err(chain)?;
    // Switching presets: worth keeping the neighbours ready from now on.
    app.warm.arm();
    load_patch(app, &patch, Some(patch.name.clone()))
}

/// Swap in a whole rig: a preset's, or one the plugin host restored. `name` is the
/// preset it counts as (`None` = untitled).
pub fn load_patch(app: &Arc<App>, patch: &Patch, name: Option<String>) -> CmdResult<PresetLoad> {
    let _busy = RigGuard::take()?;
    // Switching to the preset being preloaded? Take that instance when it's ready.
    let needed: Vec<_> = patch
        .nodes
        .iter()
        .filter_map(|n| n.plugin.clone())
        .collect();
    app.warm.wait_for(&needed);
    close_all_editors(app);
    // Offer the running instances and the warm spares for reuse (see warm.rs).
    let (sample_rate, mut spares) = {
        let studio = app.studio.lock();
        (studio.sample_rate(), studio.plugin_instances())
    };
    spares.append(&mut app.warm.take());
    let (slots, warnings) = presets::instantiate_plugins(patch, sample_rate, &mut spares);

    let (studio, rig) = {
        let mut studio = app.studio.lock();
        // What the new rig doesn't use is in `spares` too; the audio thread lets go of
        // it with the next schedule swap.
        if let Err(e) = studio.apply_patch(patch, slots) {
            // Some of these may still be in the graph: only the graveyard is safe for them.
            app.bury(spares);
            return Err(chain(e));
        }
        // Under the studio lock, so a sync push never pairs the new rig with the old name.
        set_current(app, name);
        (studio.view(), app.next_rig())
    };
    app.warm.put_back(spares);
    warm::refresh(app);
    Ok(PresetLoad {
        studio,
        presets: list(app),
        warnings,
        rig,
    })
}

/// Start over with an empty rig.
pub fn new(app: &Arc<App>) -> CmdResult<PresetLoad> {
    let _busy = RigGuard::take()?;
    close_all_editors(app);
    let (studio, rig) = {
        let mut studio = app.studio.lock();
        let old = studio.reset_graph().map_err(chain)?;
        app.warm.put_back(old);
        set_current(app, None);
        (studio.view(), app.next_rig())
    };
    warm::refresh(app);
    Ok(PresetLoad {
        studio,
        presets: list(app),
        warnings: Vec::new(),
        rig,
    })
}

pub fn delete(app: &Arc<App>, name: &str) -> CmdResult<PresetList> {
    app.presets.delete(name).map_err(chain)?;
    let was_current = app
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(name));
    if was_current {
        set_current(app, None);
    }
    // The neighbours of the current preset may have changed.
    warm::refresh(app);
    Ok(list(app))
}

pub fn rename(app: &Arc<App>, from: &str, to: &str) -> CmdResult<PresetList> {
    let to = PresetStore::clean_name(to).map_err(|e| e.to_string())?;
    app.presets.rename(from, &to).map_err(chain)?;
    let was_current = app
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(from));
    if was_current {
        set_current(app, Some(to));
    }
    // The neighbours of the current preset may have changed.
    warm::refresh(app);
    Ok(list(app))
}
