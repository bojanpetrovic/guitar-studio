//! Routing graph: topological compilation into an immutable, pre-allocated
//! [`Schedule`] that the audio thread can run without allocating.

use crate::nodes::{MAX_BLOCK, NodeShared, ProcessCtx, Processor};
use crate::util::peak;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

pub type NodeId = u32;

pub struct Step {
    pub node: NodeId,
    pub processor: Box<dyn Processor>,
    pub shared: Arc<NodeShared>,
    /// Indices of earlier steps whose outputs are summed into this step's input.
    pub sources: Vec<usize>,
    pub is_output: bool,
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
            for &src in &step.sources {
                let [bl, br] = &before[src];
                for j in 0..n {
                    sl[j] += bl[j];
                    sr[j] += br[j];
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

    #[test]
    fn orders_fan_in() {
        let order = topo_order(&[1, 2, 3, 4], &[(1, 2), (1, 3), (2, 4), (3, 4)]).unwrap();
        let pos = |n| order.iter().position(|x| *x == n).unwrap();
        assert!(pos(1) < pos(2) && pos(1) < pos(3) && pos(2) < pos(4) && pos(3) < pos(4));
    }
}
