//! Scenes: named snapshots of a rig's sound — every plugin's parameters and on/off
//! state, Gain and Mixer levels, and each mixer strip's volume, pan and mute.
//! Switching scenes saves the live sound into the scene you leave, then sets only the
//! plugin parameters that differ in the scene you go to.

use crate::graph::NodeId;
use crate::plugins::PluginSlot;
use crate::presets::MixerStrip;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most scenes per rig (one per number key).
pub const MAX_SCENES: usize = 9;

/// Values within this distance count as unchanged (normalised plugins round-trip
/// through f32).
const EPSILON: f64 = 1e-6;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub name: String,
    /// Plugin parameter values in native units, by node and the plugin's stable
    /// parameter id.
    #[serde(default)]
    pub params: BTreeMap<NodeId, BTreeMap<u32, f64>>,
    /// Bypass state of each plugin node.
    #[serde(default)]
    pub bypass: BTreeMap<NodeId, bool>,
    /// Level (dB) of each Gain and Mixer node.
    #[serde(default)]
    pub levels: BTreeMap<NodeId, f32>,
    /// Each mixer's input strips (volume, pan, mute), keyed by mixer node.
    #[serde(default)]
    pub strips: BTreeMap<NodeId, Vec<MixerStrip>>,
}

impl Scene {
    pub fn forget_node(&mut self, id: NodeId) {
        self.params.remove(&id);
        self.bypass.remove(&id);
        self.levels.remove(&id);
        self.strips.remove(&id);
        for strips in self.strips.values_mut() {
            strips.retain(|s| s.source != id);
        }
    }
}

/// Read all exposed parameters of one plugin. The lock is taken per parameter, so the
/// audio thread never waits longer than one (possibly slow) plugin call.
pub fn read_params(plugin: &Mutex<PluginSlot>) -> BTreeMap<u32, f64> {
    let (params, wait) = {
        let slot = plugin.lock();
        let params: Vec<(u32, usize)> = slot.params.iter().map(|p| (p.id, p.index)).collect();
        (params, slot.read_wait())
    };
    // Right after a write some plugins stall reads; wait that out with the lock free.
    if let Some(wait) = wait {
        std::thread::sleep(wait);
    }
    let mut out = BTreeMap::new();
    for (id, index) in params {
        if let Some(v) = plugin.lock().read_param(index) {
            out.insert(id, v);
        }
    }
    out
}

/// Set the parameters in `target` that differ from `current` (the plugin's live
/// values, already read). Ids the plugin doesn't know (e.g. after an update) are
/// skipped. One parameter per lock, like [`read_params`]. Returns how many changed.
pub fn write_params(
    plugin: &Mutex<PluginSlot>,
    target: &BTreeMap<u32, f64>,
    current: &BTreeMap<u32, f64>,
) -> usize {
    let mut changed = 0;
    for (&id, &value) in target {
        if current
            .get(&id)
            .is_some_and(|&c| (c - value).abs() <= EPSILON)
        {
            continue;
        }
        let mut slot = plugin.lock();
        let Some(i) = slot.param_index(id) else {
            continue;
        };
        let info = &slot.params[i];
        let (index, lo, hi) = (info.index, info.min.min(info.max), info.max.max(info.min));
        if slot.write_param(index, value.clamp(lo, hi)).is_ok() {
            changed += 1;
        }
    }
    changed
}

/// A fresh scene name that isn't taken yet ("Scene 1", "Scene 2", ...).
pub fn next_name(scenes: &[Scene]) -> String {
    (1..)
        .map(|n| format!("Scene {n}"))
        .find(|name| !scenes.iter().any(|s| &s.name == name))
        .expect("unbounded")
}

pub fn clean_name(name: &str) -> String {
    name.trim().chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_json_round_trips_with_numeric_keys() {
        let mut s = Scene {
            name: "Lead".into(),
            ..Default::default()
        };
        s.params.insert(7, BTreeMap::from([(3, 0.25), (12, 440.0)]));
        s.bypass.insert(7, true);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Scene>(&json).unwrap(), s);
        // Older / hand-written scenes without values still load.
        let bare: Scene = serde_json::from_str(r#"{"name":"Clean"}"#).unwrap();
        assert!(bare.params.is_empty());
    }

    #[test]
    fn names_count_up_past_taken_ones() {
        let taken = |n: &str| Scene {
            name: n.into(),
            ..Default::default()
        };
        assert_eq!(next_name(&[]), "Scene 1");
        assert_eq!(next_name(&[taken("Scene 1"), taken("Scene 3")]), "Scene 2");
        assert_eq!(clean_name("  Lead  "), "Lead");
    }
}
