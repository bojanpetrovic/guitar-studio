//! Control-thread facade: the editable graph, plugin instances, audio lifecycle.

use crate::audio::{self, AudioConfig, AudioShared, AudioStatus, RunningAudio, Telemetry};
use crate::graph::{NodeId, Schedule, Step, topo_order};
use crate::metronome::{MetronomeSettings, MetronomeShared};
use crate::nodes::{GainNode, InputNode, MAX_BLOCK, NodeShared, PluginNode, Processor};
use crate::plugins::{self, PluginDescriptor, PluginSlot, SharedPlugin};
use crate::presets::{self, Patch, PatchNode};
use crate::tuner::{self, TunerReading, TunerShared, TunerThread};
use crate::util::db_to_gain;
use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DEFAULT_SAMPLE_RATE: f64 = 48_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeType {
    Input,
    Output,
    Gain,
    Plugin,
}

struct GraphNode {
    kind: NodeType,
    x: f32,
    y: f32,
    gain_db: f32,
    shared: Arc<NodeShared>,
    plugin: Option<SharedPlugin>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: NodeId,
    pub kind: NodeType,
    pub x: f32,
    pub y: f32,
    pub gain_db: f32,
    pub bypass: bool,
    pub channel: u32,
    pub plugin: Option<PluginDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeView {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioView {
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
    pub audio: Option<AudioStatus>,
    pub audio_config: AudioConfig,
    pub audio_error: Option<String>,
    pub metronome: MetronomeSettings,
    pub tuner_enabled: bool,
    pub reference_a4: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeMeter {
    pub id: NodeId,
    pub peak: [f32; 2],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryView {
    pub nodes: Vec<NodeMeter>,
    pub output_peak: [f32; 2],
    pub cpu_load: f32,
    pub underruns: u32,
    pub beat: u32,
    pub beat_counter: u64,
    pub tuner: Option<TunerReading>,
}

pub struct Studio {
    nodes: BTreeMap<NodeId, GraphNode>,
    edges: Vec<(NodeId, NodeId)>,
    next_id: NodeId,
    audio: Option<RunningAudio>,
    audio_config: AudioConfig,
    audio_error: Option<String>,
    sample_rate: f64,
    telemetry: Arc<Telemetry>,
    metronome: Arc<MetronomeShared>,
    tuner: Arc<TunerShared>,
    tuner_enabled: Arc<AtomicBool>,
    tuner_channel: Arc<AtomicU32>,
    _tuner_thread: Option<TunerThread>,
}

impl Default for Studio {
    fn default() -> Self {
        Self::new()
    }
}

impl Studio {
    /// A fresh studio with `Input → Output`. Audio is not started yet.
    pub fn new() -> Self {
        let metronome = Arc::new(MetronomeShared::default());
        metronome.apply(&MetronomeSettings::default());
        let mut s = Self {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            next_id: 1,
            audio: None,
            audio_config: AudioConfig {
                buffer_size: 128,
                ..Default::default()
            },
            audio_error: None,
            sample_rate: DEFAULT_SAMPLE_RATE,
            telemetry: Arc::new(Telemetry::default()),
            metronome,
            tuner: Arc::new(TunerShared::default()),
            tuner_enabled: Arc::new(AtomicBool::new(false)),
            tuner_channel: Arc::new(AtomicU32::new(0)),
            _tuner_thread: None,
        };
        let input = s.insert_node(NodeType::Input, 0.0, 0.0, 0.0, None);
        let output = s.insert_node(NodeType::Output, 600.0, 0.0, 0.0, None);
        s.edges.push((input, output));
        s
    }

    // ------------------------------------------------------------------ audio

    pub fn audio_config(&self) -> &AudioConfig {
        &self.audio_config
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// (Re)start audio with `config`. Plugins are re-activated if the rate changes.
    pub fn start_audio(&mut self, config: AudioConfig) -> Result<()> {
        self.audio = None; // stop old streams first (drops the RT state off-thread)
        self._tuner_thread = None;
        self.audio_config = config;

        let (tuner_tx, tuner_rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let shared = AudioShared {
            telemetry: self.telemetry.clone(),
            metronome: self.metronome.clone(),
            tuner_enabled: self.tuner_enabled.clone(),
            tuner_channel: self.tuner_channel.clone(),
        };
        let result = audio::start(&self.audio_config, shared, Schedule::empty(), tuner_tx);
        match result {
            Ok(running) => {
                let rate = running.status.sample_rate as f64;
                if (rate - self.sample_rate).abs() > 0.5 {
                    self.sample_rate = rate;
                    for node in self.nodes.values() {
                        if let Some(p) = &node.plugin
                            && let Err(e) = plugins::reactivate(&mut p.lock(), rate, MAX_BLOCK)
                        {
                            log::warn!("re-activating plugin failed: {e:#}");
                        }
                    }
                }
                self._tuner_thread = Some(tuner::spawn(self.tuner.clone(), tuner_rx, rate as f32));
                self.audio = Some(running);
                self.audio_error = None;
                self.rebuild()
            }
            Err(e) => {
                self.audio_error = Some(format!("{e:#}"));
                Err(e)
            }
        }
    }

    pub fn stop_audio(&mut self) {
        self.audio = None;
        self._tuner_thread = None;
    }

    /// Free retired schedules. Call periodically from a non-RT thread.
    pub fn collect_garbage(&mut self) {
        if let Some(a) = &mut self.audio {
            a.collect_garbage();
        }
    }

    // ------------------------------------------------------------------ graph

    fn insert_node(
        &mut self,
        kind: NodeType,
        x: f32,
        y: f32,
        gain_db: f32,
        plugin: Option<SharedPlugin>,
    ) -> NodeId {
        let id = self.next_id;
        self.insert_node_with_id(id, kind, x, y, gain_db, plugin);
        id
    }

    fn insert_node_with_id(
        &mut self,
        id: NodeId,
        kind: NodeType,
        x: f32,
        y: f32,
        gain_db: f32,
        plugin: Option<SharedPlugin>,
    ) {
        self.next_id = self.next_id.max(id + 1);
        let shared = NodeShared::new(db_to_gain(gain_db));
        self.nodes.insert(
            id,
            GraphNode {
                kind,
                x,
                y,
                gain_db,
                shared,
                plugin,
            },
        );
    }

    pub fn add_gain_node(&mut self, x: f32, y: f32) -> Result<NodeId> {
        let id = self.insert_node(NodeType::Gain, x, y, 0.0, None);
        self.rebuild()?;
        Ok(id)
    }

    /// Insert an already-loaded plugin (see [`plugins::load_plugin`]; load it on the
    /// main thread *without* holding the studio lock, then call this).
    pub fn add_plugin_node(&mut self, slot: PluginSlot, x: f32, y: f32) -> Result<NodeId> {
        let id = self.insert_node(
            NodeType::Plugin,
            x,
            y,
            0.0,
            Some(Arc::new(Mutex::new(slot))),
        );
        self.rebuild()?;
        Ok(id)
    }

    pub fn remove_node(&mut self, id: NodeId) -> Result<Option<SharedPlugin>> {
        let node = self.nodes.get(&id).ok_or_else(|| anyhow!("no node {id}"))?;
        if matches!(node.kind, NodeType::Input | NodeType::Output) {
            bail!("the input and output nodes can't be removed");
        }
        let node = self.nodes.remove(&id).expect("checked");
        self.edges.retain(|(a, b)| *a != id && *b != id);
        self.rebuild()?;
        Ok(node.plugin)
    }

    pub fn connect(&mut self, from: NodeId, to: NodeId) -> Result<()> {
        let (Some(a), Some(b)) = (self.nodes.get(&from), self.nodes.get(&to)) else {
            bail!("unknown node");
        };
        if from == to {
            bail!("a node can't feed itself");
        }
        if a.kind == NodeType::Output || b.kind == NodeType::Input {
            bail!("signal flows from Input towards Output");
        }
        if self.edges.contains(&(from, to)) {
            return Ok(());
        }
        self.edges.push((from, to));
        let ids: Vec<_> = self.nodes.keys().copied().collect();
        if topo_order(&ids, &self.edges).is_none() {
            self.edges.pop();
            bail!("that connection would create a feedback loop");
        }
        self.rebuild()
    }

    pub fn disconnect(&mut self, from: NodeId, to: NodeId) -> Result<()> {
        self.edges.retain(|e| *e != (from, to));
        self.rebuild()
    }

    pub fn move_node(&mut self, id: NodeId, x: f32, y: f32) {
        if let Some(n) = self.nodes.get_mut(&id) {
            n.x = x;
            n.y = y;
        }
    }

    /// Parameters: `"gainDb"`, `"channel"` (Input node).
    pub fn set_param(&mut self, id: NodeId, param: &str, value: f32) -> Result<()> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| anyhow!("no node {id}"))?;
        match param {
            "gainDb" => {
                node.gain_db = value.clamp(-60.0, 24.0);
                node.shared.gain.store(db_to_gain(node.gain_db));
            }
            "channel" => {
                let ch = value.max(0.0) as u32;
                node.shared.channel.store(ch, Ordering::Relaxed);
                if node.kind == NodeType::Input {
                    self.tuner_channel.store(ch, Ordering::Relaxed);
                }
            }
            other => bail!("unknown parameter '{other}'"),
        }
        Ok(())
    }

    pub fn set_bypass(&mut self, id: NodeId, bypass: bool) -> Result<()> {
        let node = self.nodes.get(&id).ok_or_else(|| anyhow!("no node {id}"))?;
        node.shared.bypass.store(bypass, Ordering::Relaxed);
        Ok(())
    }

    pub fn plugin(&self, id: NodeId) -> Option<SharedPlugin> {
        self.nodes.get(&id).and_then(|n| n.plugin.clone())
    }

    /// Compile the graph and hand it to the audio thread.
    fn rebuild(&mut self) -> Result<()> {
        let Some(audio) = &mut self.audio else {
            return Ok(());
        };
        let ids: Vec<_> = self.nodes.keys().copied().collect();
        let order = topo_order(&ids, &self.edges).ok_or_else(|| anyhow!("graph has a cycle"))?;
        let index: BTreeMap<NodeId, usize> =
            order.iter().enumerate().map(|(i, id)| (*id, i)).collect();

        let steps = order
            .iter()
            .map(|id| {
                let node = &self.nodes[id];
                let processor: Box<dyn Processor> = match node.kind {
                    NodeType::Input => Box::new(InputNode::new(node.shared.clone())),
                    NodeType::Output | NodeType::Gain => {
                        Box::new(GainNode::new(node.shared.clone()))
                    }
                    NodeType::Plugin => Box::new(PluginNode::new(
                        node.shared.clone(),
                        node.plugin.clone().expect("plugin node has a plugin"),
                    )),
                };
                let sources = self
                    .edges
                    .iter()
                    .filter(|(_, to)| to == id)
                    .map(|(from, _)| index[from])
                    .collect();
                Step {
                    node: *id,
                    processor,
                    shared: node.shared.clone(),
                    sources,
                    is_output: node.kind == NodeType::Output,
                }
            })
            .collect();
        audio.send_schedule(Schedule::new(steps))
    }

    // ------------------------------------------------------------------ presets

    pub fn plugin_node_ids(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.plugin.is_some())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Capture the current rig. Plugin states are read from each plugin (briefly
    /// locking it); failures are returned as warnings rather than aborting the save.
    pub fn to_patch(&self, name: &str) -> (Patch, Vec<String>) {
        let mut warnings = Vec::new();
        let nodes = self
            .nodes
            .iter()
            .map(|(id, n)| {
                let (plugin, state) = match &n.plugin {
                    Some(p) => {
                        let slot = p.lock();
                        let state = match slot.plugin.save_state() {
                            Ok(bytes) => Some(Patch::encode_state(&bytes)),
                            Err(e) => {
                                warnings.push(format!(
                                    "{}: settings couldn't be saved ({e})",
                                    slot.descriptor.name
                                ));
                                None
                            }
                        };
                        (Some(slot.descriptor.clone()), state)
                    }
                    None => (None, None),
                };
                PatchNode {
                    id: *id,
                    kind: n.kind,
                    x: n.x,
                    y: n.y,
                    gain_db: n.gain_db,
                    bypass: n.shared.bypass.load(Ordering::Relaxed),
                    channel: n.shared.channel.load(Ordering::Relaxed),
                    plugin,
                    state,
                }
            })
            .collect();
        let patch = Patch {
            format: presets::FORMAT_TAG.into(),
            version: presets::FORMAT_VERSION,
            name: name.into(),
            saved_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            metronome: Some(MetronomeSettings {
                enabled: false,
                ..self.metronome.settings()
            }),
            nodes,
            edges: self.edges.clone(),
        };
        (patch, warnings)
    }

    /// Replace the whole graph with `patch`. `slots` holds the patch's plugins,
    /// already loaded by [`presets::instantiate_plugins`]; plugin nodes without a slot
    /// (missing plugins) are dropped along with their cables. Returns the previous
    /// plugins so the caller can release them on the main thread.
    pub fn apply_patch(
        &mut self,
        patch: &Patch,
        mut slots: HashMap<NodeId, PluginSlot>,
    ) -> Result<Vec<SharedPlugin>> {
        let old: Vec<SharedPlugin> = std::mem::take(&mut self.nodes)
            .into_values()
            .filter_map(|n| n.plugin)
            .collect();
        self.edges.clear();
        self.next_id = 1;

        let (mut have_input, mut have_output) = (false, false);
        for n in &patch.nodes {
            let plugin = match n.kind {
                NodeType::Plugin => match slots.remove(&n.id) {
                    Some(slot) => Some(Arc::new(Mutex::new(slot))),
                    None => continue,
                },
                NodeType::Input if have_input => continue,
                NodeType::Output if have_output => continue,
                _ => None,
            };
            have_input |= n.kind == NodeType::Input;
            have_output |= n.kind == NodeType::Output;
            self.insert_node_with_id(n.id, n.kind, n.x, n.y, n.gain_db, plugin);
            let shared = &self.nodes[&n.id].shared;
            shared.bypass.store(n.bypass, Ordering::Relaxed);
            shared.channel.store(n.channel, Ordering::Relaxed);
            if n.kind == NodeType::Input {
                self.tuner_channel.store(n.channel, Ordering::Relaxed);
            }
        }
        if !have_input {
            self.insert_node(NodeType::Input, 0.0, 0.0, 0.0, None);
        }
        if !have_output {
            self.insert_node(NodeType::Output, 600.0, 0.0, 0.0, None);
        }

        // Re-add cables through `connect`'s rules; anything invalid is skipped.
        let audio = self.audio.take(); // avoid one schedule per cable
        for &(from, to) in &patch.edges {
            let _ = self.connect(from, to);
        }
        self.audio = audio;

        if let Some(m) = &patch.metronome {
            self.metronome.apply(&MetronomeSettings {
                enabled: self.metronome.settings().enabled,
                ..*m
            });
        }
        self.rebuild()?;
        Ok(old)
    }

    /// Back to a blank `Input → Output` rig.
    pub fn reset_graph(&mut self) -> Result<Vec<SharedPlugin>> {
        let blank = Patch {
            format: presets::FORMAT_TAG.into(),
            version: presets::FORMAT_VERSION,
            name: String::new(),
            saved_at: 0,
            metronome: None,
            nodes: vec![
                PatchNode {
                    id: 1,
                    kind: NodeType::Input,
                    x: 0.0,
                    y: 0.0,
                    gain_db: 0.0,
                    bypass: false,
                    channel: 0,
                    plugin: None,
                    state: None,
                },
                PatchNode {
                    id: 2,
                    kind: NodeType::Output,
                    x: 600.0,
                    y: 0.0,
                    gain_db: 0.0,
                    bypass: false,
                    channel: 0,
                    plugin: None,
                    state: None,
                },
            ],
            edges: vec![(1, 2)],
        };
        self.apply_patch(&blank, HashMap::new())
    }

    // ------------------------------------------------------------- metronome / tuner

    pub fn set_metronome(&mut self, settings: MetronomeSettings) {
        self.metronome.apply(&settings);
    }

    pub fn set_tuner(&mut self, enabled: bool, reference_a4: Option<f32>) {
        self.tuner_enabled.store(enabled, Ordering::Relaxed);
        self.tuner.enabled.store(enabled, Ordering::Relaxed);
        if let Some(a4) = reference_a4 {
            self.tuner.reference_a4.store(a4.clamp(400.0, 480.0));
        }
    }

    // ------------------------------------------------------------------ views

    pub fn view(&self) -> StudioView {
        StudioView {
            nodes: self
                .nodes
                .iter()
                .map(|(id, n)| NodeView {
                    id: *id,
                    kind: n.kind,
                    x: n.x,
                    y: n.y,
                    gain_db: n.gain_db,
                    bypass: n.shared.bypass.load(Ordering::Relaxed),
                    channel: n.shared.channel.load(Ordering::Relaxed),
                    plugin: n.plugin.as_ref().map(|p| p.lock().descriptor.clone()),
                })
                .collect(),
            edges: self
                .edges
                .iter()
                .map(|(from, to)| EdgeView {
                    from: *from,
                    to: *to,
                })
                .collect(),
            audio: self.audio.as_ref().map(|a| a.status.clone()),
            audio_config: self.audio_config.clone(),
            audio_error: self.audio_error.clone(),
            metronome: self.metronome.settings(),
            tuner_enabled: self.tuner_enabled.load(Ordering::Relaxed),
            reference_a4: self.tuner.reference_a4.load(),
        }
    }

    /// Meters since the last call (peaks are reset on read).
    pub fn telemetry(&self) -> TelemetryView {
        let t = &self.telemetry;
        TelemetryView {
            nodes: self
                .nodes
                .iter()
                .map(|(id, n)| NodeMeter {
                    id: *id,
                    peak: [n.shared.peak[0].take(), n.shared.peak[1].take()],
                })
                .collect(),
            output_peak: [t.output_peak[0].take(), t.output_peak[1].take()],
            cpu_load: t.cpu_load.load(),
            underruns: t.underruns.load(Ordering::Relaxed),
            beat: self.metronome.beat.load(Ordering::Relaxed),
            beat_counter: self.metronome.beat_counter.load(Ordering::Relaxed),
            tuner: self.tuner.reading.lock().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn studio_is_send() {
        assert_send::<Studio>();
    }

    #[test]
    fn patch_round_trip_restores_graph() {
        let mut s = Studio::new();
        let g = s.add_gain_node(300.0, 50.0).unwrap();
        s.set_param(g, "gainDb", -6.0).unwrap();
        s.set_bypass(g, true).unwrap();
        s.set_param(1, "channel", 2.0).unwrap();
        s.disconnect(1, 2).unwrap();
        s.connect(1, g).unwrap();
        s.connect(g, 2).unwrap();
        let (patch, warnings) = s.to_patch("Test");
        assert!(warnings.is_empty());

        // Survives JSON, then loads into a fresh studio.
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap();
        let v = t.view();
        let gain = v.nodes.iter().find(|n| n.kind == NodeType::Gain).unwrap();
        assert_eq!((gain.x, gain.gain_db, gain.bypass), (300.0, -6.0, true));
        assert_eq!(
            v.nodes
                .iter()
                .find(|n| n.kind == NodeType::Input)
                .unwrap()
                .channel,
            2
        );
        let edges: Vec<_> = v.edges.iter().map(|e| (e.from, e.to)).collect();
        assert_eq!(edges, vec![(1, gain.id), (gain.id, 2)]);
        // New nodes don't collide with restored ids.
        assert!(t.add_gain_node(0.0, 0.0).unwrap() > gain.id);
    }

    #[test]
    fn missing_plugins_are_dropped_with_their_cables() {
        let mut s = Studio::new();
        let (mut patch, _) = s.to_patch("x");
        patch.nodes.push(PatchNode {
            id: 9,
            kind: NodeType::Plugin,
            x: 0.0,
            y: 0.0,
            gain_db: 0.0,
            bypass: false,
            channel: 0,
            plugin: None,
            state: None,
        });
        patch.edges = vec![(1, 9), (9, 2)];
        s.apply_patch(&patch, HashMap::new()).unwrap();
        let v = s.view();
        assert_eq!(v.nodes.len(), 2);
        assert!(v.edges.is_empty());
    }

    #[test]
    fn rejects_feedback_loops() {
        let mut s = Studio::new();
        let a = s.add_gain_node(0.0, 0.0).unwrap();
        let b = s.add_gain_node(0.0, 0.0).unwrap();
        s.connect(a, b).unwrap();
        assert!(s.connect(b, a).is_err());
        assert!(s.connect(a, a).is_err());
    }
}
