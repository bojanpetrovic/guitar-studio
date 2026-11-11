//! Presets: a saved rig (graph, node settings, plugin states, tempo) as a JSON file.
//!
//! Plugin state is the plugin's own opaque blob (VST3 `getState`, AU `ClassInfo`,
//! CLAP `state`), base64-encoded, so amp/cab settings made inside a plugin's editor
//! come back exactly.

use crate::graph::NodeId;
use crate::metronome::MetronomeSettings;
use crate::nodes::MAX_BLOCK;
use crate::plugins::{self, PluginDescriptor, SharedPlugin};
use crate::scenes::Scene;
use crate::studio::NodeType;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const FORMAT_TAG: &str = "guitar-studio-preset";
pub const FORMAT_VERSION: u32 = 1;
const MAX_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchNode {
    pub id: NodeId,
    pub kind: NodeType,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default)]
    pub channel: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginDescriptor>,
    /// Base64 plugin state blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Mixer nodes: one strip per connected input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strips: Vec<MixerStrip>,
}

/// A mixer input strip, keyed by the node feeding it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerStrip {
    pub source: NodeId,
    pub gain_db: f32,
    pub pan: f32,
    #[serde(default)]
    pub mute: bool,
}

impl MixerStrip {
    pub fn unity(source: NodeId) -> Self {
        Self {
            source,
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub saved_at: u64,
    /// Tempo etc. (`enabled` is ignored on load — loading a preset never starts the click.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metronome: Option<MetronomeSettings>,
    pub nodes: Vec<PatchNode>,
    pub edges: Vec<(NodeId, NodeId)>,
    /// The preset's pedalboard layout. Owned by the UI; the engine stores it as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pedalboard: Option<serde_json::Value>,
    /// Named snapshots of plugin parameters (see [`crate::scenes`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
    /// The scene the rig was in when saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_scene: Option<usize>,
}

impl Patch {
    pub fn encode_state(bytes: &[u8]) -> String {
        B64.encode(bytes)
    }

    pub fn decode_state(s: &str) -> Result<Vec<u8>> {
        Ok(B64.decode(s)?)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    pub name: String,
    pub saved_at: u64,
    pub plugins: Vec<String>,
}

/// Get every plugin a patch needs and restore their states (call on the **main
/// thread**, without the studio lock). A running instance of the same plugin in
/// `spares` is reused (restoring a state takes milliseconds, starting a big amp sim
/// takes seconds) and removed from `spares`; anything else is loaded fresh. Plugins
/// that can't be loaded are reported in the returned warnings and left out.
pub fn instantiate_plugins(
    patch: &Patch,
    sample_rate: f64,
    spares: &mut Vec<SharedPlugin>,
) -> (HashMap<NodeId, SharedPlugin>, Vec<String>) {
    let mut slots = HashMap::new();
    let mut warnings = Vec::new();
    for node in patch.nodes.iter().filter(|n| n.kind == NodeType::Plugin) {
        let Some(desc) = &node.plugin else { continue };
        let key = desc.key();
        let plugin = match spares.iter().position(|p| p.lock().descriptor.key() == key) {
            Some(i) => spares.swap_remove(i),
            None => match plugins::load_plugin(desc, sample_rate, MAX_BLOCK) {
                Ok(slot) => Arc::new(Mutex::new(slot)),
                Err(e) => {
                    warnings.push(format!(
                        "{} is missing or failed to load ({e:#})",
                        desc.name
                    ));
                    continue;
                }
            },
        };
        if let Some(state) = &node.state {
            let mut slot = plugin.lock();
            let restored =
                Patch::decode_state(state).and_then(|bytes| Ok(slot.plugin.load_state(&bytes)?));
            slot.forget_values();
            if let Err(e) = restored {
                warnings.push(format!(
                    "{}: settings couldn't be restored ({e})",
                    desc.name
                ));
            }
        }
        slots.insert(node.id, plugin);
    }
    (slots, warnings)
}

/// The plugins a patch uses (one entry per plugin node).
pub fn patch_plugins(patch: &Patch) -> Vec<PluginDescriptor> {
    patch
        .nodes
        .iter()
        .filter(|n| n.kind == NodeType::Plugin)
        .filter_map(|n| n.plugin.clone())
        .collect()
}

/// A folder of `*.json` presets.
pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Trim and validate a user-entered preset name.
    pub fn clean_name(name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("preset name can't be empty");
        }
        if name.chars().count() > MAX_NAME_LEN {
            bail!("preset name is too long (max {MAX_NAME_LEN} characters)");
        }
        Ok(name.to_string())
    }

    fn path_for(&self, name: &str) -> PathBuf {
        let stem: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('.')
            .to_lowercase();
        self.dir.join(format!("{stem}.json"))
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path_for(name).exists()
    }

    /// All presets, sorted by name.
    pub fn list(&self) -> Vec<PresetInfo> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<PresetInfo> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read_patch(&e.path()).ok())
            .map(|p| PresetInfo {
                plugins: p
                    .nodes
                    .iter()
                    .filter_map(|n| n.plugin.as_ref().map(|d| d.name.clone()))
                    .collect(),
                name: p.name,
                saved_at: p.saved_at,
            })
            .collect();
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    pub fn load(&self, name: &str) -> Result<Patch> {
        read_patch(&self.path_for(name)).with_context(|| format!("loading preset '{name}'"))
    }

    /// Atomic write (temp file + rename) so a crash never leaves a half-written preset.
    pub fn save(&self, patch: &Patch) -> Result<()> {
        let path = self.path_for(&patch.name);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(patch)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.path_for(name);
        if !path.exists() {
            bail!("preset '{name}' doesn't exist");
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let to = Self::clean_name(to)?;
        let same_file = self.path_for(from) == self.path_for(&to);
        if !same_file && self.exists(&to) {
            bail!("a preset called '{to}' already exists");
        }
        let mut patch = self.load(from)?;
        patch.name = to;
        self.save(&patch)?;
        if !same_file {
            self.delete(from)?;
        }
        Ok(())
    }
}

fn read_patch(path: &Path) -> Result<Patch> {
    let bytes = std::fs::read(path)?;
    let patch: Patch = serde_json::from_slice(&bytes)?;
    if patch.format != FORMAT_TAG {
        return Err(anyhow!("not a Guitar Studio preset"));
    }
    if patch.version > FORMAT_VERSION {
        bail!("preset was saved by a newer version of Guitar Studio");
    }
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(name: &str) -> Patch {
        Patch {
            format: FORMAT_TAG.into(),
            version: FORMAT_VERSION,
            name: name.into(),
            saved_at: 1,
            metronome: None,
            nodes: vec![],
            edges: vec![],
            pedalboard: None,
            scenes: vec![],
            active_scene: None,
        }
    }

    #[test]
    fn save_list_rename_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = PresetStore::new(dir.path()).unwrap();
        store.save(&patch("Clean Verb")).unwrap();
        store.save(&patch("Djent / Lead")).unwrap();
        assert_eq!(
            store
                .list()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["Clean Verb", "Djent / Lead"]
        );

        store.rename("Clean Verb", "Ambient").unwrap();
        assert!(store.exists("Ambient") && !store.exists("Clean Verb"));
        assert!(
            store.rename("Ambient", "djent / lead").is_err(),
            "collides case-insensitively"
        );
        store.rename("Ambient", "ambient").unwrap(); // case-only rename is fine
        assert_eq!(store.load("Ambient").unwrap().name, "ambient");

        store.delete("ambient").unwrap();
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn rejects_bad_names_and_files() {
        assert!(PresetStore::clean_name("   ").is_err());
        assert_eq!(PresetStore::clean_name("  Lead  ").unwrap(), "Lead");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("junk.json"), b"{\"hello\":1}").unwrap();
        assert!(PresetStore::new(dir.path()).unwrap().list().is_empty());
    }

    #[test]
    fn state_round_trips() {
        let blob = vec![0u8, 1, 2, 250, 255];
        assert_eq!(
            Patch::decode_state(&Patch::encode_state(&blob)).unwrap(),
            blob
        );
    }
}
