//! Preset commands. Loading runs on the main thread (sync command) because it
//! instantiates plugins; plugins are loaded *before* taking the studio lock so the
//! meters keep running while a heavy amp sim boots.

use crate::{AppState, CmdResult, editor};
use gs_engine::presets::{self, PresetInfo, PresetStore};
use gs_engine::studio::StudioView;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

/// Returned when saving would overwrite a different preset; the UI asks first.
const EXISTS: &str = "PRESET_EXISTS";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetList {
    pub presets: Vec<PresetInfo>,
    pub current: Option<String>,
    /// Preset that was active when the app last quit (for auto-restore on launch).
    pub last: Option<String>,
    pub dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetLoad {
    pub studio: StudioView,
    pub presets: PresetList,
    pub warnings: Vec<String>,
    /// Rig generation (see `AppState::rig`).
    pub rig: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    last_preset: Option<String>,
}

fn session_path(state: &AppState) -> std::path::PathBuf {
    state.data_dir.join("session.json")
}

fn read_session(state: &AppState) -> Session {
    std::fs::read(session_path(state))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn set_current(state: &AppState, name: Option<String>) {
    *state.current_preset.lock() = name.clone();
    let session = Session { last_preset: name };
    if let Ok(json) = serde_json::to_vec_pretty(&session) {
        let _ = std::fs::write(session_path(state), json);
    }
}

fn list(state: &AppState) -> PresetList {
    PresetList {
        presets: state.presets.list(),
        current: state.current_preset.lock().clone(),
        last: read_session(state).last_preset,
        dir: state.presets.dir().display().to_string(),
    }
}

fn next_rig(state: &AppState) -> u64 {
    state.rig.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Close every open plugin editor before the graph is swapped out.
fn close_all_editors(app: &AppHandle, state: &AppState) {
    let ids = state.studio.lock().plugin_node_ids();
    for id in ids {
        editor::close(app, state, id);
    }
}

#[tauri::command]
pub fn list_presets(state: State<AppState>) -> PresetList {
    list(&state)
}

/// Save the current rig. `overwrite` must be true to replace a *different* existing
/// preset (re-saving the current one never needs it).
#[tauri::command]
pub fn save_preset(
    state: State<AppState>,
    name: String,
    overwrite: bool,
) -> CmdResult<(PresetList, Vec<String>)> {
    let name = PresetStore::clean_name(&name).map_err(|e| e.to_string())?;
    let is_current = state
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(&name));
    if !overwrite && !is_current && state.presets.exists(&name) {
        return Err(EXISTS.into());
    }
    let (patch, warnings) = state.studio.lock().to_patch(&name);
    state.presets.save(&patch).map_err(|e| format!("{e:#}"))?;
    set_current(&state, Some(name));
    Ok((list(&state), warnings))
}

#[tauri::command]
pub fn load_preset(app: AppHandle, state: State<AppState>, name: String) -> CmdResult<PresetLoad> {
    let patch = state.presets.load(&name).map_err(|e| format!("{e:#}"))?;
    let sample_rate = state.studio.lock().sample_rate();
    let (slots, warnings) = presets::instantiate_plugins(&patch, sample_rate);

    close_all_editors(&app, &state);
    let studio = {
        let mut studio = state.studio.lock();
        let old = studio
            .apply_patch(&patch, slots)
            .map_err(|e| format!("{e:#}"))?;
        state.graveyard.lock().extend(old);
        studio.view()
    };
    set_current(&state, Some(patch.name.clone()));
    Ok(PresetLoad {
        studio,
        presets: list(&state),
        warnings,
        rig: next_rig(&state),
    })
}

/// Start over with an empty rig.
#[tauri::command]
pub fn new_preset(app: AppHandle, state: State<AppState>) -> CmdResult<PresetLoad> {
    close_all_editors(&app, &state);
    let studio = {
        let mut studio = state.studio.lock();
        let old = studio.reset_graph().map_err(|e| format!("{e:#}"))?;
        state.graveyard.lock().extend(old);
        studio.view()
    };
    set_current(&state, None);
    Ok(PresetLoad {
        studio,
        presets: list(&state),
        warnings: Vec::new(),
        rig: next_rig(&state),
    })
}

#[tauri::command]
pub fn delete_preset(state: State<AppState>, name: String) -> CmdResult<PresetList> {
    state.presets.delete(&name).map_err(|e| format!("{e:#}"))?;
    let was_current = state
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(&name));
    if was_current {
        set_current(&state, None);
    }
    Ok(list(&state))
}

#[tauri::command]
pub fn rename_preset(state: State<AppState>, from: String, to: String) -> CmdResult<PresetList> {
    let to = PresetStore::clean_name(&to).map_err(|e| e.to_string())?;
    state
        .presets
        .rename(&from, &to)
        .map_err(|e| format!("{e:#}"))?;
    let was_current = state
        .current_preset
        .lock()
        .as_ref()
        .is_some_and(|c| c.eq_ignore_ascii_case(&from));
    if was_current {
        set_current(&state, Some(to));
    }
    Ok(list(&state))
}
