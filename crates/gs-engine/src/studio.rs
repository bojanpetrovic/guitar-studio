//! Control-thread facade: the editable graph, plugin instances, audio lifecycle.

use crate::audio::{
    self, AudioConfig, AudioShared, AudioStatus, HostedAudio, RunningAudio, Telemetry,
};
use crate::graph::{MixerChannel, NodeId, Schedule, Step, topo_order};
use crate::metronome::{MetronomeSettings, MetronomeShared};
use crate::nodes::{GainNode, InputNode, MAX_BLOCK, NodeShared, PluginNode, Processor};
use crate::plugins::{self, ParamInfo, PluginDescriptor, PluginSlot, SharedPlugin};
use crate::presets::{self, MixerStrip, Patch, PatchNode};
use crate::scenes::{self, MAX_SCENES, Scene};
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
    /// Mixes its inputs with per-input gain / pan / mute strips.
    Mixer,
}

struct GraphNode {
    kind: NodeType,
    x: f32,
    y: f32,
    gain_db: f32,
    shared: Arc<NodeShared>,
    plugin: Option<SharedPlugin>,
    /// The plugin's descriptor, copied out so views don't lock a live plugin.
    descriptor: Option<PluginDescriptor>,
    /// Mixer nodes: strip settings per source node. Kept when a cable is removed so
    /// reconnecting restores the strip.
    strips: BTreeMap<NodeId, Strip>,
}

struct Strip {
    setting: MixerStrip,
    shared: Arc<MixerChannel>,
}

impl Strip {
    fn new(setting: MixerStrip) -> Self {
        Self {
            shared: MixerChannel::new(setting.gain_db, setting.pan, setting.mute),
            setting,
        }
    }
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
    /// Mixer nodes: strips for the connected inputs, in cable order.
    pub strips: Vec<MixerStrip>,
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
    /// The current pedalboard layout (opaque to the engine; `null` = none yet).
    pub pedalboard: serde_json::Value,
    /// Scene names, in order (number keys 1-9 pick them).
    pub scenes: Vec<String>,
    pub active_scene: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeMeter {
    pub id: NodeId,
    pub peak: [f32; 2],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripMeter {
    pub mixer: NodeId,
    pub source: NodeId,
    pub peak: [f32; 2],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryView {
    pub nodes: Vec<NodeMeter>,
    pub strips: Vec<StripMeter>,
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
    pedalboard: serde_json::Value,
    scenes: Vec<Scene>,
    active_scene: Option<usize>,
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
            // The tuner listens from launch; the panel switch turns it off.
            tuner_enabled: Arc::new(AtomicBool::new(true)),
            tuner_channel: Arc::new(AtomicU32::new(0)),
            _tuner_thread: None,
            pedalboard: serde_json::Value::Null,
            scenes: Vec::new(),
            active_scene: None,
        };
        let input = s.insert_node(NodeType::Input, 0.0, 0.0, 0.0, None);
        let output = s.insert_node(NodeType::Output, 600.0, 0.0, 0.0, None);
        s.edges.push((input, output));
        s
    }

    // ------------------------------------------------------------------ audio

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// (Re)start audio with `config`. Plugins are re-activated if the rate changes.
    pub fn start_audio(&mut self, config: AudioConfig) -> Result<()> {
        self.audio = None; // stop old streams first (drops the RT state off-thread)
        self._tuner_thread = None;
        self.audio_config = config;

        let (tuner_tx, tuner_rx) = rtrb::RingBuffer::<f32>::new(48_000);
        match audio::start(
            &self.audio_config,
            self.audio_shared(),
            Schedule::empty(),
            tuner_tx,
        ) {
            Ok(running) => {
                self.audio_error = None;
                self.run_audio(running, tuner_rx)
            }
            Err(e) => {
                self.audio_error = Some(format!("{e:#}"));
                Err(e)
            }
        }
    }

    /// Run inside a plugin host instead of on our own devices: the host's audio thread
    /// drives the returned [`HostedAudio`]. Call again when the host changes the rate.
    pub fn start_hosted(
        &mut self,
        sample_rate: u32,
        channels: u16,
        max_block: u32,
    ) -> Result<HostedAudio> {
        self.audio = None;
        self._tuner_thread = None;
        self.audio_error = None;
        let (tuner_tx, tuner_rx) = rtrb::RingBuffer::<f32>::new(48_000);
        let (running, hosted) = audio::hosted(
            self.audio_shared(),
            Schedule::empty(),
            tuner_tx,
            sample_rate,
            channels,
            max_block,
        );
        self.run_audio(running, tuner_rx)?;
        Ok(hosted)
    }

    fn audio_shared(&self) -> AudioShared {
        AudioShared {
            telemetry: self.telemetry.clone(),
            metronome: self.metronome.clone(),
            tuner_enabled: self.tuner_enabled.clone(),
            tuner_channel: self.tuner_channel.clone(),
        }
    }

    /// Adopt freshly started audio: follow its sample rate, start the tuner, and send
    /// the graph.
    fn run_audio(&mut self, running: RunningAudio, tuner_rx: rtrb::Consumer<f32>) -> Result<()> {
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
        self.rebuild()
    }

    /// Stop audio (the plugin host deactivated us).
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
        let descriptor = plugin.as_ref().map(|p| p.lock().descriptor.clone());
        self.nodes.insert(
            id,
            GraphNode {
                kind,
                x,
                y,
                gain_db,
                shared,
                plugin,
                descriptor,
                strips: BTreeMap::new(),
            },
        );
    }

    pub fn add_mixer_node(&mut self, x: f32, y: f32) -> Result<NodeId> {
        let id = self.insert_node(NodeType::Mixer, x, y, 0.0, None);
        self.rebuild()?;
        Ok(id)
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

    /// Put a different plugin into a plugin node. Its cables, mixer strips, position and
    /// on/off state stay; scenes forget the old plugin's parameter values (the new one
    /// has its own parameters). Returns the old plugin for the caller to release on the
    /// main thread.
    pub fn replace_plugin(&mut self, id: NodeId, plugin: SharedPlugin) -> Result<SharedPlugin> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| anyhow!("no node {id}"))?;
        // Check first: `replace` on a Gain or Mixer node would put the plugin in it.
        if node.plugin.is_none() {
            bail!("only plugins can be replaced");
        }
        node.descriptor = Some(plugin.lock().descriptor.clone());
        let old = node.plugin.replace(plugin).expect("checked above");
        for scene in &mut self.scenes {
            scene.params.remove(&id);
        }
        self.rebuild()?;
        Ok(old)
    }

    /// Is `id` a plugin node?
    pub fn is_plugin(&self, id: NodeId) -> bool {
        self.nodes.get(&id).is_some_and(|n| n.plugin.is_some())
    }

    pub fn remove_node(&mut self, id: NodeId) -> Result<Option<SharedPlugin>> {
        let node = self.nodes.get(&id).ok_or_else(|| anyhow!("no node {id}"))?;
        if matches!(node.kind, NodeType::Input | NodeType::Output) {
            bail!("the input and output nodes can't be removed");
        }
        let node = self.nodes.remove(&id).expect("checked");
        self.edges.retain(|(a, b)| *a != id && *b != id);
        for n in self.nodes.values_mut() {
            n.strips.remove(&id);
        }
        for scene in &mut self.scenes {
            scene.forget_node(id);
        }
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
        if let Some(mixer) = self
            .nodes
            .get_mut(&to)
            .filter(|n| n.kind == NodeType::Mixer)
        {
            mixer
                .strips
                .entry(from)
                .or_insert_with(|| Strip::new(MixerStrip::unity(from)));
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

    /// Is the master output muted (the Output node's on/off flag)?
    pub fn output_muted(&self) -> bool {
        self.nodes
            .values()
            .find(|n| n.kind == NodeType::Output)
            .is_some_and(|n| n.shared.bypass.load(Ordering::Relaxed))
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
                    NodeType::Output => Box::new(GainNode::output(node.shared.clone())),
                    NodeType::Gain | NodeType::Mixer => {
                        Box::new(GainNode::new(node.shared.clone()))
                    }
                    NodeType::Plugin => Box::new(PluginNode::new(
                        node.shared.clone(),
                        node.plugin.clone().expect("plugin node has a plugin"),
                    )),
                };
                let feeding: Vec<NodeId> = self
                    .edges
                    .iter()
                    .filter(|(_, to)| to == id)
                    .map(|(from, _)| *from)
                    .collect();
                let sources = feeding.iter().map(|from| index[from]).collect();
                let step = Step::new(
                    processor,
                    node.shared.clone(),
                    sources,
                    node.kind == NodeType::Output,
                );
                if node.kind == NodeType::Mixer {
                    let channels = feeding
                        .iter()
                        .map(|from| match node.strips.get(from) {
                            Some(strip) => strip.shared.clone(),
                            None => MixerChannel::new(0.0, 0.0, false),
                        })
                        .collect();
                    step.with_channels(channels)
                } else {
                    step
                }
            })
            .collect();
        audio.send_schedule(Schedule::new(steps));
        Ok(())
    }

    /// Set one field of a mixer strip: `"gainDb"` (-60..+12), `"pan"` (-1..1) or
    /// `"mute"` (0 / 1). Takes effect on the next block, ramped.
    pub fn set_mixer_strip(
        &mut self,
        mixer: NodeId,
        source: NodeId,
        field: &str,
        value: f32,
    ) -> Result<()> {
        let node = self
            .nodes
            .get_mut(&mixer)
            .filter(|n| n.kind == NodeType::Mixer)
            .ok_or_else(|| anyhow!("node {mixer} is not a mixer"))?;
        let strip = node
            .strips
            .entry(source)
            .or_insert_with(|| Strip::new(MixerStrip::unity(source)));
        match field {
            "gainDb" => {
                strip.setting.gain_db = value.clamp(-60.0, 12.0);
                strip.shared.gain.store(db_to_gain(strip.setting.gain_db));
            }
            "pan" => {
                strip.setting.pan = value.clamp(-1.0, 1.0);
                strip.shared.pan.store(strip.setting.pan);
            }
            "mute" => {
                strip.setting.mute = value >= 0.5;
                strip
                    .shared
                    .mute
                    .store(strip.setting.mute, Ordering::Relaxed);
            }
            other => bail!("unknown mixer field '{other}'"),
        }
        Ok(())
    }

    /// A mixer's strips for its connected inputs, in cable order.
    fn connected_strips(&self, id: NodeId) -> Vec<MixerStrip> {
        let Some(node) = self.nodes.get(&id).filter(|n| n.kind == NodeType::Mixer) else {
            return Vec::new();
        };
        self.edges
            .iter()
            .filter(|(_, to)| *to == id)
            .map(|(from, _)| {
                node.strips
                    .get(from)
                    .map_or(MixerStrip::unity(*from), |s| s.setting)
            })
            .collect()
    }

    // ------------------------------------------------------------------ presets

    pub fn plugin_node_ids(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.plugin.is_some())
            .map(|(id, _)| *id)
            .collect()
    }

    /// The running plugin instances (for reuse by the next rig).
    pub fn plugin_instances(&self) -> Vec<SharedPlugin> {
        self.nodes
            .values()
            .filter_map(|n| n.plugin.clone())
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
                    strips: self.connected_strips(*id),
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
            pedalboard: (!self.pedalboard.is_null()).then(|| self.pedalboard.clone()),
            // The live sound belongs to the active scene; save it there too.
            scenes: self.scenes_with_live_capture(),
            active_scene: self.active_scene,
        };
        (patch, warnings)
    }

    /// Replace the whole graph with `patch`. `slots` holds the patch's plugins,
    /// already loaded by [`presets::instantiate_plugins`]; plugin nodes without a slot
    /// (missing plugins) are dropped along with their cables. Returns the previous
    /// plugins the new rig doesn't use, so the caller can keep or release them on the
    /// main thread.
    pub fn apply_patch(
        &mut self,
        patch: &Patch,
        mut slots: HashMap<NodeId, SharedPlugin>,
    ) -> Result<Vec<SharedPlugin>> {
        // Instances the new rig reuses aren't "old".
        let was_muted = self.output_muted();
        let old: Vec<SharedPlugin> = std::mem::take(&mut self.nodes)
            .into_values()
            .filter_map(|n| n.plugin)
            .filter(|p| !slots.values().any(|s| Arc::ptr_eq(s, p)))
            .collect();
        self.edges.clear();
        // Past every id in the patch, also those skipped below (a missing plugin), so a
        // node added later can't inherit their scene data.
        self.next_id = patch.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;

        let (mut have_input, mut have_output) = (false, false);
        for n in &patch.nodes {
            let plugin = match n.kind {
                NodeType::Plugin => match slots.remove(&n.id) {
                    Some(plugin) => Some(plugin),
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
            // The output's mute is a live control, not part of a sound: a preset load
            // never mutes or unmutes you.
            let bypass = if n.kind == NodeType::Output {
                was_muted
            } else {
                n.bypass
            };
            shared.bypass.store(bypass, Ordering::Relaxed);
            shared.channel.store(n.channel, Ordering::Relaxed);
            if n.kind == NodeType::Input {
                self.tuner_channel.store(n.channel, Ordering::Relaxed);
            }
        }
        if !have_input {
            self.insert_node(NodeType::Input, 0.0, 0.0, 0.0, None);
        }
        if !have_output {
            let id = self.insert_node(NodeType::Output, 600.0, 0.0, 0.0, None);
            self.nodes[&id]
                .shared
                .bypass
                .store(was_muted, Ordering::Relaxed);
        }

        // Re-add cables through `connect`'s rules; anything invalid is skipped.
        let audio = self.audio.take(); // avoid one schedule per cable
        for &(from, to) in &patch.edges {
            let _ = self.connect(from, to);
        }
        for n in patch.nodes.iter().filter(|n| n.kind == NodeType::Mixer) {
            if let Some(node) = self.nodes.get_mut(&n.id) {
                for st in &n.strips {
                    node.strips.insert(st.source, Strip::new(*st));
                }
            }
        }
        self.audio = audio;

        self.pedalboard = patch.pedalboard.clone().unwrap_or(serde_json::Value::Null);
        // Plugin states already hold the active scene's sound; nothing to apply.
        self.scenes = patch.scenes.iter().take(MAX_SCENES).cloned().collect();
        for skipped in patch
            .nodes
            .iter()
            .filter(|n| !self.nodes.contains_key(&n.id))
        {
            for scene in &mut self.scenes {
                scene.forget_node(skipped.id);
            }
        }
        self.active_scene = patch.active_scene.filter(|&i| i < self.scenes.len());
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
                    strips: vec![],
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
                    strips: vec![],
                },
            ],
            edges: vec![(1, 2)],
            pedalboard: None,
            scenes: vec![],
            active_scene: None,
        };
        self.apply_patch(&blank, HashMap::new())
    }

    // ------------------------------------------------------------------ pedalboard

    pub fn set_pedalboard(&mut self, board: serde_json::Value) {
        self.pedalboard = board;
    }

    /// Parameters a plugin node exposes (hidden / read-only ones are left out).
    pub fn plugin_params(&self, id: NodeId) -> Result<Vec<ParamInfo>> {
        let plugin = self
            .plugin(id)
            .ok_or_else(|| anyhow!("node {id} is not a plugin"))?;
        Ok(plugin.lock().params.clone())
    }

    /// Set a plugin parameter by its stable id, in native units (clamped to range).
    /// Call from the main thread: the plugin lock is held only for the call, and the
    /// audio thread spins briefly instead of dropping the block.
    pub fn set_plugin_param(&self, id: NodeId, param_id: u32, value: f64) -> Result<()> {
        let plugin = self
            .plugin(id)
            .ok_or_else(|| anyhow!("node {id} is not a plugin"))?;
        let mut slot = plugin.lock();
        let index = slot
            .param_index(param_id)
            .ok_or_else(|| anyhow!("unknown parameter {param_id}"))?;
        let info = &slot.params[index];
        let value = value.clamp(info.min.min(info.max), info.max.max(info.min));
        let plugin_index = info.index;
        slot.write_param(plugin_index, value)?;
        Ok(())
    }

    /// Current values for `(node, param id)` pairs; `None` where the node or parameter
    /// no longer exists. Each plugin is locked once for all of its parameters; right
    /// after a write the cached values are returned (see [`plugins::READ_QUIET`]).
    pub fn param_values(&self, items: &[(NodeId, u32)]) -> Vec<Option<f64>> {
        let mut out = vec![None; items.len()];
        let mut by_node: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
        for (i, (node, _)) in items.iter().enumerate() {
            by_node.entry(*node).or_default().push(i);
        }
        for (node, idxs) in by_node {
            let Some(plugin) = self.plugin(node) else {
                continue;
            };
            // One parameter per lock, like scenes: a read can take ~70 µs on a JUCE AU,
            // and the audio thread only waits so long before playing a block dry.
            for i in idxs {
                let mut slot = plugin.lock();
                out[i] = slot.param_index(items[i].1).and_then(|pi| {
                    let index = slot.params[pi].index;
                    slot.param_value_now(index)
                });
            }
        }
        out
    }

    // ------------------------------------------------------------------ scenes

    /// Snapshot the rig's sound into a scene called `name`: plugin parameters and
    /// on/off state, Gain / Mixer levels and mixer strips.
    fn capture(&self, name: String) -> Scene {
        let mut scene = Scene {
            name,
            ..Default::default()
        };
        for (id, node) in &self.nodes {
            if let Some(plugin) = &node.plugin {
                scene.params.insert(*id, scenes::read_params(plugin));
                scene
                    .bypass
                    .insert(*id, node.shared.bypass.load(Ordering::Relaxed));
            }
            if matches!(node.kind, NodeType::Gain | NodeType::Mixer) {
                scene.levels.insert(*id, node.gain_db);
            }
            if node.kind == NodeType::Mixer {
                scene.strips.insert(*id, self.connected_strips(*id));
            }
        }
        scene
    }

    /// Put one mixer strip's volume, pan and mute back, if that cable still exists.
    fn restore_strip(&mut self, mixer: NodeId, st: &MixerStrip) {
        if self.edges.contains(&(st.source, mixer)) {
            let mute = if st.mute { 1.0 } else { 0.0 };
            let _ = self.set_mixer_strip(mixer, st.source, "gainDb", st.gain_db);
            let _ = self.set_mixer_strip(mixer, st.source, "pan", st.pan);
            let _ = self.set_mixer_strip(mixer, st.source, "mute", mute);
        }
    }

    /// The scene list with the active scene refreshed from the live sound.
    fn scenes_with_live_capture(&self) -> Vec<Scene> {
        let mut list = self.scenes.clone();
        if let Some(scene) = self.active_scene.and_then(|i| list.get_mut(i)) {
            *scene = self.capture(std::mem::take(&mut scene.name));
        }
        list
    }

    /// Store the live sound in the active scene (so tweaks made in a scene stick).
    /// Returns the live snapshot either way.
    fn save_active_scene(&mut self) -> Scene {
        let live = self.capture(String::new());
        if let Some(scene) = self.active_scene.and_then(|i| self.scenes.get_mut(i)) {
            *scene = Scene {
                name: std::mem::take(&mut scene.name),
                ..live.clone()
            };
        }
        live
    }

    /// Add a scene holding the current sound and make it active. Returns its index.
    pub fn add_scene(&mut self, name: Option<&str>) -> Result<usize> {
        if self.scenes.len() >= MAX_SCENES {
            bail!("a rig can have up to {MAX_SCENES} scenes");
        }
        let live = self.save_active_scene();
        let name = name
            .map(scenes::clean_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| scenes::next_name(&self.scenes));
        self.scenes.push(Scene { name, ..live });
        let index = self.scenes.len() - 1;
        self.active_scene = Some(index);
        Ok(index)
    }

    /// Switch to scene `index`: the live sound is kept in the scene being left, then
    /// only the parameters that differ from it are set (each plugin call takes the
    /// plugin's lock on its own, so the audio thread keeps running). Selecting the
    /// active scene is a no-op.
    pub fn select_scene(&mut self, index: usize) -> Result<()> {
        if index >= self.scenes.len() {
            bail!("there is no scene {}", index + 1);
        }
        if self.active_scene == Some(index) {
            return Ok(());
        }
        let live = self.save_active_scene();
        let scene = self.scenes[index].clone();
        let none = BTreeMap::new();
        for (id, node) in &self.nodes {
            let Some(plugin) = &node.plugin else {
                continue;
            };
            // Plugins added after the scene was made keep their current settings.
            if let Some(values) = scene.params.get(id) {
                scenes::write_params(plugin, values, live.params.get(id).unwrap_or(&none));
            }
            if let Some(&bypass) = scene.bypass.get(id) {
                node.shared.bypass.store(bypass, Ordering::Relaxed);
            }
        }
        // Built-in nodes: levels, then mixer strips (gains ramp, so no clicks).
        for (&id, &db) in &scene.levels {
            let builtin = self
                .nodes
                .get(&id)
                .is_some_and(|n| matches!(n.kind, NodeType::Gain | NodeType::Mixer));
            if builtin {
                let _ = self.set_param(id, "gainDb", db);
            }
        }
        for (&mixer, strips) in &scene.strips {
            for st in strips {
                self.restore_strip(mixer, st);
            }
        }
        self.active_scene = Some(index);
        Ok(())
    }

    pub fn rename_scene(&mut self, index: usize, name: &str) -> Result<()> {
        let name = scenes::clean_name(name);
        if name.is_empty() {
            bail!("scene names can't be empty");
        }
        let scene = self
            .scenes
            .get_mut(index)
            .ok_or_else(|| anyhow!("there is no scene {}", index + 1))?;
        scene.name = name;
        Ok(())
    }

    /// Remove a scene. The sound doesn't change; if it was the active scene, no scene
    /// is active until one is picked.
    pub fn delete_scene(&mut self, index: usize) -> Result<()> {
        if index >= self.scenes.len() {
            bail!("there is no scene {}", index + 1);
        }
        self.scenes.remove(index);
        self.active_scene = match self.active_scene {
            Some(a) if a == index => None,
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
        Ok(())
    }

    // ------------------------------------------------------------- metronome / tuner

    pub fn set_metronome(&mut self, settings: MetronomeSettings) {
        self.metronome.apply(&settings);
    }

    pub fn set_tuner(&mut self, enabled: bool, reference_a4: Option<f32>) {
        // Two flags: the audio thread's tap, and the tuner thread (which resets itself).
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
                    plugin: n.descriptor.clone(),
                    strips: self.connected_strips(*id),
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
            pedalboard: self.pedalboard.clone(),
            scenes: self.scenes.iter().map(|s| s.name.clone()).collect(),
            active_scene: self.active_scene,
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
            strips: self
                .nodes
                .iter()
                .flat_map(|(id, n)| {
                    n.strips.iter().map(move |(source, st)| StripMeter {
                        mixer: *id,
                        source: *source,
                        peak: [st.shared.peak[0].take(), st.shared.peak[1].take()],
                    })
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
        assert!(v.pedalboard.is_null());
        // New nodes don't collide with restored ids.
        assert!(t.add_gain_node(0.0, 0.0).unwrap() > gain.id);
    }

    #[test]
    fn pedalboard_travels_with_presets() {
        let mut s = Studio::new();
        let board =
            serde_json::json!({ "version": 1, "widgets": [{ "id": "w1", "type": "footswitch" }] });
        s.set_pedalboard(board.clone());
        let (patch, _) = s.to_patch("Board");
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap();
        assert_eq!(t.view().pedalboard, board);
        t.reset_graph().unwrap();
        assert!(
            t.view().pedalboard.is_null(),
            "a new rig starts with an empty board"
        );
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
            strips: vec![],
        });
        patch.edges = vec![(1, 9), (9, 2)];
        s.apply_patch(&patch, HashMap::new()).unwrap();
        let v = s.view();
        assert_eq!(v.nodes.len(), 2);
        assert!(v.edges.is_empty());
    }

    #[test]
    fn scenes_keep_order_names_and_travel_with_presets() {
        let mut s = Studio::new();
        assert_eq!(s.add_scene(None).unwrap(), 0);
        assert_eq!(s.add_scene(Some("  Lead  ")).unwrap(), 1);
        s.add_scene(None).unwrap();
        assert_eq!(s.view().scenes, ["Scene 1", "Lead", "Scene 2"]);
        assert_eq!(s.view().active_scene, Some(2));

        s.select_scene(0).unwrap();
        assert!(s.select_scene(7).is_err());
        s.rename_scene(0, "Clean").unwrap();
        assert!(s.rename_scene(0, "   ").is_err());

        // Deleting before the active scene shifts it; deleting it clears it.
        s.select_scene(2).unwrap();
        s.delete_scene(0).unwrap();
        assert_eq!(s.view().active_scene, Some(1));
        s.delete_scene(1).unwrap();
        assert_eq!(s.view().active_scene, None);

        while s.view().scenes.len() < MAX_SCENES {
            s.add_scene(None).unwrap();
        }
        assert!(s.add_scene(None).is_err(), "one scene per number key");

        let (patch, _) = s.to_patch("Scenes");
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap();
        assert_eq!(t.view().scenes, s.view().scenes);
        assert_eq!(t.view().active_scene, s.view().active_scene);
        t.reset_graph().unwrap();
        assert!(t.view().scenes.is_empty());
    }

    #[test]
    fn output_mute_survives_preset_loads() {
        let mut s = Studio::new();
        let (patch, _) = s.to_patch("Unmuted"); // saved with the output on
        let out = s
            .view()
            .nodes
            .iter()
            .find(|n| n.kind == NodeType::Output)
            .unwrap()
            .id;
        s.set_bypass(out, true).unwrap();
        assert!(s.output_muted());
        s.apply_patch(&patch, HashMap::new()).unwrap();
        assert!(s.output_muted(), "loading a preset doesn't unmute");
        s.reset_graph().unwrap();
        assert!(s.output_muted(), "nor does a new rig");
        let out = s
            .view()
            .nodes
            .iter()
            .find(|n| n.kind == NodeType::Output)
            .unwrap()
            .id;
        s.set_bypass(out, false).unwrap();
        assert!(!s.output_muted());
    }

    #[test]
    fn missing_plugins_dont_hand_their_ids_or_scene_data_to_new_nodes() {
        // A saved rig with a plugin (id 9) that isn't installed here, and a scene that
        // remembers its settings.
        let mut s = Studio::new();
        s.add_scene(Some("Lead")).unwrap();
        let (mut patch, _) = s.to_patch("Missing");
        let input = patch.nodes[0].clone();
        patch.nodes.push(PatchNode {
            id: 9,
            kind: NodeType::Plugin,
            plugin: None,
            ..input
        });
        patch.scenes[0].params.insert(9, BTreeMap::from([(1, 0.5)]));
        patch.scenes[0].bypass.insert(9, true);

        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap(); // no slot for 9: skipped
        let (saved, _) = t.to_patch("Missing");
        assert!(!saved.scenes[0].params.contains_key(&9));
        assert!(!saved.scenes[0].bypass.contains_key(&9));
        assert!(
            t.add_gain_node(0.0, 0.0).unwrap() > 9,
            "a new node gets a fresh id"
        );
    }

    #[test]
    fn scenes_switch_mixer_strips_and_levels() {
        // Input → A, Input → B, A → Mix, B → Mix, Mix → Output
        let mut s = Studio::new();
        let a = s.add_gain_node(0.0, 0.0).unwrap();
        let b = s.add_gain_node(0.0, 0.0).unwrap();
        let mix = s.add_mixer_node(0.0, 0.0).unwrap();
        s.disconnect(1, 2).unwrap();
        for (f, t) in [(1, a), (1, b), (a, mix), (b, mix), (mix, 2)] {
            s.connect(f, t).unwrap();
        }
        let strip = |s: &Studio, src: NodeId| {
            let node = s.view().nodes.into_iter().find(|n| n.id == mix).unwrap();
            node.strips.into_iter().find(|st| st.source == src).unwrap()
        };
        let level =
            |s: &Studio, id: NodeId| s.view().nodes.iter().find(|n| n.id == id).unwrap().gain_db;

        // Scene 1: A loud and left, B muted, gain A at -3, mixer at -1.
        s.add_scene(Some("Rhythm")).unwrap();
        s.set_mixer_strip(mix, a, "gainDb", 2.0).unwrap();
        s.set_mixer_strip(mix, a, "pan", -0.5).unwrap();
        s.set_mixer_strip(mix, b, "mute", 1.0).unwrap();
        s.set_param(a, "gainDb", -3.0).unwrap();
        s.set_param(mix, "gainDb", -1.0).unwrap();

        // Scene 2 starts as a copy; make it the opposite.
        s.add_scene(Some("Lead")).unwrap();
        s.set_mixer_strip(mix, a, "mute", 1.0).unwrap();
        s.set_mixer_strip(mix, b, "mute", 0.0).unwrap();
        s.set_mixer_strip(mix, b, "pan", 0.7).unwrap();
        s.set_param(a, "gainDb", 0.0).unwrap();
        s.set_param(mix, "gainDb", 3.0).unwrap();

        s.select_scene(0).unwrap();
        let (sa, sb) = (strip(&s, a), strip(&s, b));
        assert_eq!((sa.gain_db, sa.pan, sa.mute), (2.0, -0.5, false));
        assert!(sb.mute);
        assert_eq!((level(&s, a), level(&s, mix)), (-3.0, -1.0));

        s.select_scene(1).unwrap();
        let (sa, sb) = (strip(&s, a), strip(&s, b));
        assert!(sa.mute);
        assert_eq!((sb.pan, sb.mute), (0.7, false));
        assert_eq!((level(&s, a), level(&s, mix)), (0.0, 3.0));

        // Saved and reloaded, the scenes still switch the mixer.
        let (patch, _) = s.to_patch("Mix scenes");
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap();
        t.select_scene(0).unwrap();
        assert!(strip(&t, b).mute);
        assert_eq!(level(&t, mix), -1.0);

        // A removed source disappears from every scene.
        t.remove_node(b).unwrap();
        t.select_scene(1).unwrap();
        assert!(strip(&t, a).mute);
    }

    #[test]
    fn mixer_strips_follow_cables_and_presets() {
        // Input → A, Input → B, A → Mix, B → Mix, Mix → Output
        let mut s = Studio::new();
        let a = s.add_gain_node(0.0, 0.0).unwrap();
        let b = s.add_gain_node(0.0, 0.0).unwrap();
        let mix = s.add_mixer_node(0.0, 0.0).unwrap();
        s.disconnect(1, 2).unwrap();
        for (f, t) in [(1, a), (1, b), (a, mix), (b, mix), (mix, 2)] {
            s.connect(f, t).unwrap();
        }
        let strips = |s: &Studio| {
            s.view()
                .nodes
                .into_iter()
                .find(|n| n.id == mix)
                .unwrap()
                .strips
        };
        assert_eq!(
            strips(&s),
            vec![MixerStrip::unity(a), MixerStrip::unity(b)],
            "one unity strip per input"
        );

        s.set_mixer_strip(mix, a, "gainDb", -12.0).unwrap();
        s.set_mixer_strip(mix, b, "pan", 0.6).unwrap();
        s.set_mixer_strip(mix, b, "mute", 1.0).unwrap();
        assert!(
            s.set_mixer_strip(a, b, "pan", 0.0).is_err(),
            "only mixers have strips"
        );
        assert!(s.set_mixer_strip(mix, a, "bogus", 0.0).is_err());

        // Disconnecting hides the strip; reconnecting brings its settings back.
        s.disconnect(a, mix).unwrap();
        assert_eq!(strips(&s).len(), 1);
        s.connect(a, mix).unwrap();
        let st = strips(&s);
        let sa = st.iter().find(|x| x.source == a).unwrap();
        assert_eq!(sa.gain_db, -12.0);

        // Presets keep the strips.
        let (patch, _) = s.to_patch("Mix");
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut t = Studio::new();
        t.apply_patch(&patch, HashMap::new()).unwrap();
        let st = strips(&t);
        let sb = st.iter().find(|x| x.source == b).unwrap();
        assert_eq!((sb.pan, sb.mute), (0.6, true));
        assert_eq!(st.iter().find(|x| x.source == a).unwrap().gain_db, -12.0);

        // Removing a source drops its strip.
        t.remove_node(b).unwrap();
        assert!(strips(&t).iter().all(|x| x.source != b));
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
