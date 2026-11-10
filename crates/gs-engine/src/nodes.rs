//! Audio-thread node processors.
//!
//! Every node has one stereo input (all incoming cables are summed) and one stereo
//! output. Fan-out and fan-in are expressed purely through graph edges.

use crate::plugins::SharedPlugin;
use crate::util::AtomicF32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext, ProcessStatus, TransportInfo};

/// Largest block the engine ever processes in one go (plugins are activated with it).
pub const MAX_BLOCK: usize = 1024;
/// Maximum channels across all buses we hand to a plugin.
pub const MAX_PLUGIN_CHANNELS: usize = 16;

pub struct ProcessCtx<'a> {
    pub sample_rate: f32,
    pub frames: usize,
    /// De-interleaved hardware input, one Vec per device channel (len >= frames).
    pub hw_input: &'a [Vec<f32>],
    pub transport: TransportInfo,
}

pub trait Processor: Send {
    fn process(&mut self, ctx: &ProcessCtx, input: [&[f32]; 2], output: [&mut [f32]; 2]);
}

/// Control-thread ↔ audio-thread shared state for one node: parameters and meters.
#[derive(Debug)]
pub struct NodeShared {
    pub bypass: AtomicBool,
    /// Linear gain (Input trim, Gain node level, Output master volume).
    pub gain: AtomicF32,
    /// Hardware input channel (Input node only).
    pub channel: AtomicU32,
    pub peak: [AtomicF32; 2],
}

impl NodeShared {
    pub fn new(gain: f32) -> Arc<Self> {
        Arc::new(Self {
            bypass: AtomicBool::new(false),
            gain: AtomicF32::new(gain),
            channel: AtomicU32::new(0),
            peak: [AtomicF32::new(0.0), AtomicF32::new(0.0)],
        })
    }
}

fn copy_through(input: [&[f32]; 2], output: [&mut [f32]; 2]) {
    let [ol, or] = output;
    ol.copy_from_slice(input[0]);
    or.copy_from_slice(input[1]);
}

/// Applies a gain that glides from `current` to `target` over the block (no zipper noise).
fn apply_gain(current: &mut f32, target: f32, input: [&[f32]; 2], output: [&mut [f32]; 2]) {
    let n = input[0].len();
    let step = if n > 0 {
        (target - *current) / n as f32
    } else {
        0.0
    };
    let [ol, or] = output;
    let mut g = *current;
    for i in 0..n {
        g += step;
        ol[i] = input[0][i] * g;
        or[i] = input[1][i] * g;
    }
    *current = target;
}

/// Reads one hardware input channel (the guitar) and duplicates it to L/R.
pub struct InputNode {
    pub shared: Arc<NodeShared>,
    gain: f32,
}

impl InputNode {
    pub fn new(shared: Arc<NodeShared>) -> Self {
        let gain = shared.gain.load();
        Self { shared, gain }
    }
}

impl Processor for InputNode {
    fn process(&mut self, ctx: &ProcessCtx, _input: [&[f32]; 2], output: [&mut [f32]; 2]) {
        let n = ctx.frames;
        let ch = self.shared.channel.load(Ordering::Relaxed) as usize;
        match ctx.hw_input.get(ch) {
            Some(src) => {
                let src = &src[..n];
                apply_gain(&mut self.gain, self.shared.gain.load(), [src, src], output);
            }
            None => {
                let [l, r] = output;
                l.fill(0.0);
                r.fill(0.0);
            }
        }
    }
}

/// Simple level node; also used for the Output node's master volume.
pub struct GainNode {
    pub shared: Arc<NodeShared>,
    gain: f32,
}

impl GainNode {
    pub fn new(shared: Arc<NodeShared>) -> Self {
        let gain = shared.gain.load();
        Self { shared, gain }
    }
}

impl Processor for GainNode {
    fn process(&mut self, _ctx: &ProcessCtx, input: [&[f32]; 2], output: [&mut [f32]; 2]) {
        if self.shared.bypass.load(Ordering::Relaxed) {
            copy_through(input, output);
            return;
        }
        apply_gain(&mut self.gain, self.shared.gain.load(), input, output);
    }
}

/// Hosts a VST3 / AU / CLAP plugin.
///
/// The plugin lives behind a mutex shared with the control thread (editor, state).
/// The audio thread only ever `try_lock`s it: if the UI happens to hold it for a
/// moment, that block passes the dry signal through instead of blocking.
pub struct PluginNode {
    pub shared: Arc<NodeShared>,
    plugin: SharedPlugin,
    scratch_in: Vec<Vec<f32>>,
    scratch_out: Vec<Vec<f32>>,
    events: EventList,
    out_events: EventList,
}

impl PluginNode {
    pub fn new(shared: Arc<NodeShared>, plugin: SharedPlugin) -> Self {
        Self {
            shared,
            plugin,
            scratch_in: vec![vec![0.0; MAX_BLOCK]; MAX_PLUGIN_CHANNELS],
            scratch_out: vec![vec![0.0; MAX_BLOCK]; MAX_PLUGIN_CHANNELS],
            events: EventList::new(),
            out_events: EventList::new(),
        }
    }
}

impl Processor for PluginNode {
    fn process(&mut self, ctx: &ProcessCtx, input: [&[f32]; 2], output: [&mut [f32]; 2]) {
        if self.shared.bypass.load(Ordering::Relaxed) {
            copy_through(input, output);
            return;
        }
        let Some(mut guard) = self.plugin.try_lock() else {
            copy_through(input, output);
            return;
        };
        let slot = &mut *guard;
        let io = &slot.io;
        let n = ctx.frames;

        // Feed the main input bus; zero everything else (sidechains etc.).
        for (i, ch) in self.scratch_in.iter_mut().enumerate().take(io.total_in) {
            let dst = &mut ch[..n];
            match (io.main_in, i) {
                (1, 0) => {
                    for (j, d) in dst.iter_mut().enumerate() {
                        *d = 0.5 * (input[0][j] + input[1][j]);
                    }
                }
                (m, 0) if m >= 2 => dst.copy_from_slice(input[0]),
                (m, 1) if m >= 2 => dst.copy_from_slice(input[1]),
                _ => dst.fill(0.0),
            }
        }

        let scratch_in = &self.scratch_in;
        let ins: [&[f32]; MAX_PLUGIN_CHANNELS] = std::array::from_fn(|i| {
            if i < io.total_in {
                &scratch_in[i][..n]
            } else {
                &[]
            }
        });
        let mut out_iter = self.scratch_out.iter_mut();
        let mut outs: [&mut [f32]; MAX_PLUGIN_CHANNELS] =
            std::array::from_fn(|_| out_iter.next().map(|v| &mut v[..n]).unwrap_or_default());

        let main_out = io.main_out;
        let status = {
            let mut buffer = AudioBuffer::new(
                &ins[..io.total_in],
                &mut outs[..io.total_out],
                n,
                &io.bus_in,
                &io.bus_out,
            );
            let mut pctx = ProcessContext {
                sample_rate: ctx.sample_rate as f64,
                max_block_size: MAX_BLOCK,
                transport: Some(ctx.transport),
                output_events: &mut self.out_events,
            };
            slot.plugin.process(&mut buffer, &self.events, &mut pctx)
        };
        drop(guard);

        let [ol, or] = output;
        match status {
            Ok(ProcessStatus::Error) | Err(_) => {
                ol.fill(0.0);
                or.fill(0.0);
            }
            Ok(_) => match main_out {
                0 => {
                    ol.fill(0.0);
                    or.fill(0.0);
                }
                1 => {
                    ol.copy_from_slice(&self.scratch_out[0][..n]);
                    or.copy_from_slice(&self.scratch_out[0][..n]);
                }
                _ => {
                    ol.copy_from_slice(&self.scratch_out[0][..n]);
                    or.copy_from_slice(&self.scratch_out[1][..n]);
                }
            },
        }
    }
}
