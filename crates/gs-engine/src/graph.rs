//! Routing graph: topological compilation into an immutable, pre-allocated
//! [`Schedule`] that the audio thread can run without allocating.

use crate::nodes::{MAX_BLOCK, NodeShared, ProcessCtx, Processor};
use crate::util::{AtomicF32, db_to_gain, peak};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub type NodeId = u32;

/// One input strip of a Mixer node: control-thread ↔ audio-thread shared state.
#[derive(Debug)]
pub struct MixerChannel {
    /// Linear gain.
    pub gain: AtomicF32,
    /// -1 (left) .. 0 (center) .. +1 (right).
    pub pan: AtomicF32,
    pub mute: AtomicBool,
    /// Post-fader peak meter.
    pub peak: [AtomicF32; 2],
}

impl MixerChannel {
    pub fn new(gain_db: f32, pan: f32, mute: bool) -> Arc<Self> {
        Arc::new(Self {
            gain: AtomicF32::new(db_to_gain(gain_db)),
            pan: AtomicF32::new(pan.clamp(-1.0, 1.0)),
            mute: AtomicBool::new(mute),
            peak: [AtomicF32::new(0.0), AtomicF32::new(0.0)],
        })
    }

    /// Per-side gains. Balance-style constant-power pan: both sides are at unity in
    /// the center (stereo sources keep their width), and turning one way fades the
    /// other side out along a cosine curve.
    pub fn side_gains(&self) -> [f32; 2] {
        if self.mute.load(Ordering::Relaxed) {
            return [0.0, 0.0];
        }
        let g = self.gain.load();
        let theta = (self.pan.load().clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
        let s2 = std::f32::consts::SQRT_2;
        [
            g * (s2 * theta.cos()).min(1.0),
            g * (s2 * theta.sin()).min(1.0),
        ]
    }
}

pub struct Step {
    pub node: NodeId,
    pub processor: Box<dyn Processor>,
    pub shared: Arc<NodeShared>,
    /// Indices of earlier steps whose outputs are summed into this step's input.
    pub sources: Vec<usize>,
    pub is_output: bool,
    /// Mixer nodes: one strip per entry in `sources` (same order). Each source is
    /// weighted by its strip's gain/pan instead of being summed as-is.
    pub channels: Option<Vec<Arc<MixerChannel>>>,
    /// Gains applied in the previous block, so changes ramp instead of clicking.
    pub last_gains: Vec<[f32; 2]>,
}

impl Step {
    pub fn new(
        node: NodeId,
        processor: Box<dyn Processor>,
        shared: Arc<NodeShared>,
        sources: Vec<usize>,
        is_output: bool,
    ) -> Self {
        Self {
            node,
            processor,
            shared,
            sources,
            is_output,
            channels: None,
            last_gains: Vec::new(),
        }
    }

    /// Turn this step into a mixer: `channels[k]` weights `sources[k]`.
    pub fn with_channels(mut self, channels: Vec<Arc<MixerChannel>>) -> Self {
        debug_assert_eq!(channels.len(), self.sources.len());
        self.last_gains = channels.iter().map(|c| c.side_gains()).collect();
        self.channels = Some(channels);
        self
    }
}

pub struct Schedule {
    steps: Vec<Step>,
    buffers: Vec<[Vec<f32>; 2]>,
    scratch: [Vec<f32>; 2],
}

impl Schedule {
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn new(steps: Vec<Step>) -> Self {
        let buffers = (0..steps.len())
            .map(|_| [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]])
            .collect();
        Self {
            steps,
            buffers,
            scratch: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
        }
    }

    /// Run every node in order and sum all Output nodes into `master`.
    /// Real-time safe: no allocation, no locking.
    pub fn run(&mut self, ctx: &ProcessCtx, master: [&mut [f32]; 2]) {
        let n = ctx.frames;
        let [ml, mr] = master;
        ml[..n].fill(0.0);
        mr[..n].fill(0.0);

        for i in 0..self.steps.len() {
            let (before, rest) = self.buffers.split_at_mut(i);
            let out = &mut rest[0];
            let step = &mut self.steps[i];

            let [sl, sr] = &mut self.scratch;
            sl[..n].fill(0.0);
            sr[..n].fill(0.0);
            match &step.channels {
                None => {
                    for &src in &step.sources {
                        let [bl, br] = &before[src];
                        for j in 0..n {
                            sl[j] += bl[j];
                            sr[j] += br[j];
                        }
                    }
                }
                Some(channels) => {
                    for (k, &src) in step.sources.iter().enumerate() {
                        let [bl, br] = &before[src];
                        let target = channels[k].side_gains();
                        let [mut gl, mut gr] = step.last_gains[k];
                        let (dl, dr) = ((target[0] - gl) / n as f32, (target[1] - gr) / n as f32);
                        let (mut pl, mut pr) = (0.0f32, 0.0f32);
                        for j in 0..n {
                            gl += dl;
                            gr += dr;
                            let (l, r) = (bl[j] * gl, br[j] * gr);
                            sl[j] += l;
                            sr[j] += r;
                            pl = pl.max(l.abs());
                            pr = pr.max(r.abs());
                        }
                        step.last_gains[k] = target;
                        channels[k].peak[0].raise_to(pl);
                        channels[k].peak[1].raise_to(pr);
                    }
                }
            }

            let [ol, or] = out;
            step.processor
                .process(ctx, [&sl[..n], &sr[..n]], [&mut ol[..n], &mut or[..n]]);
            step.shared.peak[0].raise_to(peak(&ol[..n]));
            step.shared.peak[1].raise_to(peak(&or[..n]));

            if step.is_output {
                for j in 0..n {
                    ml[j] += ol[j];
                    mr[j] += or[j];
                }
            }
        }
    }
}

/// Kahn topological sort. Returns `None` if the edges contain a cycle.
pub fn topo_order(nodes: &[NodeId], edges: &[(NodeId, NodeId)]) -> Option<Vec<NodeId>> {
    let mut indeg: BTreeMap<NodeId, usize> = nodes.iter().map(|&n| (n, 0)).collect();
    for (_, to) in edges {
        *indeg.get_mut(to)? += 1;
    }
    let mut queue: VecDeque<NodeId> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(n) = queue.pop_front() {
        order.push(n);
        for (from, to) in edges {
            if *from == n {
                let d = indeg.get_mut(to)?;
                *d -= 1;
                if *d == 0 {
                    queue.push_back(*to);
                }
            }
        }
    }
    (order.len() == nodes.len()).then_some(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cycles() {
        assert!(topo_order(&[1, 2, 3], &[(1, 2), (2, 3)]).is_some());
        assert!(topo_order(&[1, 2, 3], &[(1, 2), (2, 3), (3, 1)]).is_none());
    }

    /// Outputs a constant on both channels.
    struct Dc(f32);
    impl Processor for Dc {
        fn process(&mut self, _: &ProcessCtx, _: [&[f32]; 2], [l, r]: [&mut [f32]; 2]) {
            l.fill(self.0);
            r.fill(self.0);
        }
    }
    /// Passes its input through.
    struct Thru;
    impl Processor for Thru {
        fn process(&mut self, _: &ProcessCtx, i: [&[f32]; 2], [l, r]: [&mut [f32]; 2]) {
            l.copy_from_slice(i[0]);
            r.copy_from_slice(i[1]);
        }
    }

    fn run_twice(s: &mut Schedule) -> ([f32; 2], [f32; 2]) {
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames: 64,
            hw_input: &[],
            transport: Default::default(),
        };
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        s.run(&ctx, [&mut l, &mut r]); // first block may ramp
        s.run(&ctx, [&mut l, &mut r]);
        let first = [l[0], r[0]];
        let last = [l[63], r[63]];
        (first, last)
    }

    #[test]
    fn mixer_applies_gain_pan_and_mute_per_input() {
        let a = MixerChannel::new(-6.0206, 0.0, false); // ×0.5, center
        let b = MixerChannel::new(0.0, 1.0, false); // unity, hard right
        let steps = vec![
            Step::new(1, Box::new(Dc(1.0)), NodeShared::new(1.0), vec![], false),
            Step::new(2, Box::new(Dc(0.25)), NodeShared::new(1.0), vec![], false),
            Step::new(3, Box::new(Thru), NodeShared::new(1.0), vec![0, 1], true)
                .with_channels(vec![a.clone(), b.clone()]),
        ];
        let mut s = Schedule::new(steps);
        let (_, out) = run_twice(&mut s);
        // L = 1.0×0.5 + 0.25×0 ; R = 1.0×0.5 + 0.25×1
        assert!(
            (out[0] - 0.5).abs() < 1e-3 && (out[1] - 0.75).abs() < 1e-3,
            "{out:?}"
        );

        // Changes ramp across the next block, then settle.
        a.mute.store(true, Ordering::Relaxed);
        b.pan.store(-1.0);
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames: 64,
            hw_input: &[],
            transport: Default::default(),
        };
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        s.run(&ctx, [&mut l, &mut r]);
        assert!(
            l[0] > 0.0 && l[0] < 0.5 + 0.25,
            "ramps from the old value, no jump"
        );
        a.peak[0].take(); // meters hold their max until read; clear the ramp block
        let (_, out) = run_twice(&mut s);
        assert!(
            (out[0] - 0.25).abs() < 1e-3 && out[1].abs() < 1e-3,
            "{out:?}"
        );
        assert!(
            a.peak[0].take() < 1e-3,
            "muted strip meters silence once settled"
        );
    }

    #[test]
    fn center_pan_is_unity_both_sides() {
        let c = MixerChannel::new(0.0, 0.0, false);
        let [l, r] = c.side_gains();
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        c.pan.store(-0.5);
        let [l, r] = c.side_gains();
        assert!((l - 1.0).abs() < 1e-6 && r < 1.0 && r > 0.0);
    }

    #[test]
    fn orders_fan_in() {
        let order = topo_order(&[1, 2, 3, 4], &[(1, 2), (1, 3), (2, 4), (3, 4)]).unwrap();
        let pos = |n| order.iter().position(|x| *x == n).unwrap();
        assert!(pos(1) < pos(2) && pos(1) < pos(3) && pos(2) < pos(4) && pos(3) < pos(4));
    }
}
