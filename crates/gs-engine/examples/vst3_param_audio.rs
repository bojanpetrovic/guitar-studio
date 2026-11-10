//! Does a host-side parameter change reach a VST3 plugin's *audio*? Turn an output/gain
//! parameter to its minimum and compare output levels.
//! cargo run -p gs-engine --example vst3_param_audio -- <dir containing one .vst3> <param name substring>
use gs_engine::plugins::{PluginSlot, load_plugin, scan_path_in_process};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

fn rms(slot: &mut PluginSlot, blocks: usize) -> f32 {
    let n = 256;
    let sine: Vec<f32> = (0..n).map(|i| 0.3 * (i as f32 * 0.0144).sin()).collect();
    let input = vec![sine; slot.io.total_in];
    let mut out = vec![vec![0.0f32; n]; slot.io.total_out];
    let mut acc = 0.0;
    for b in 0..blocks {
        let ins: Vec<&[f32]> = input.iter().map(|v| &v[..]).collect();
        let mut outs: Vec<&mut [f32]> = out.iter_mut().map(|v| &mut v[..]).collect();
        let (bi, bo) = (slot.io.bus_in.clone(), slot.io.bus_out.clone());
        let mut buf = AudioBuffer::new(&ins, &mut outs, n, &bi, &bo);
        let mut oe = EventList::new();
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            max_block_size: 1024,
            transport: None,
            output_events: &mut oe,
        };
        let _ = slot.plugin.process(&mut buf, &EventList::new(), &mut ctx);
        if b >= blocks / 2 {
            acc += out[0].iter().map(|x| x * x).sum::<f32>();
        }
    }
    (acc / (n * blocks / 2) as f32).sqrt()
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let desc = scan_path_in_process("vst3", std::path::Path::new(&args[1]))?.remove(0);
    let mut slot = load_plugin(&desc, 48_000.0, 1024)?;
    let needle = args[2].to_lowercase();
    let p = slot
        .params
        .iter()
        .find(|p| p.name.to_lowercase().contains(&needle))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no param matching '{needle}'"))?;
    let before = rms(&mut slot, 40);
    slot.plugin.set_parameter(p.index, p.min)?;
    let after = rms(&mut slot, 40);
    let reached = after < before * 0.5;
    println!(
        "{} / '{}' → min: rms {before:.4} → {after:.4}: {}",
        desc.name,
        p.name,
        if reached {
            "change REACHES the audio"
        } else {
            "audio UNCHANGED (controller only)"
        }
    );
    Ok(())
}
