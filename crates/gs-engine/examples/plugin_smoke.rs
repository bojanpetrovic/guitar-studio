//! cargo run -p gs-engine --example plugin_smoke -- <format> <name substring>
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext, buffer::BusRange};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let format = args.get(1).map(String::as_str).unwrap_or("vst3");
    let needle = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
    let t = std::time::Instant::now();
    let list = match args.get(3) {
        Some(path) => gs_engine::plugins::scan_path_in_process(format, std::path::Path::new(path))?,
        None => scan_format_in_process(format)?,
    };
    println!(
        "{format}: {} plugins scanned in {:?}",
        list.len(),
        t.elapsed()
    );
    let Some(desc) = list
        .iter()
        .find(|p| p.name.to_lowercase().contains(&needle))
    else {
        println!("no match for '{needle}'");
        return Ok(());
    };
    println!(
        "loading {} by {} ({})",
        desc.name, desc.vendor, desc.unique_id
    );
    let t = std::time::Instant::now();
    let mut slot = load_plugin(desc, 48_000.0, 1024)?;
    println!(
        "loaded in {:?}; io = {:?}; params = {}",
        t.elapsed(),
        slot.io,
        slot.plugin.parameter_count()
    );

    let n = 256;
    let input: Vec<Vec<f32>> = (0..slot.io.total_in)
        .map(|_| {
            (0..n)
                .map(|i| 0.3 * (i as f32 * 2.0 * std::f32::consts::PI * 110.0 / 48_000.0).sin())
                .collect()
        })
        .collect();
    let mut out: Vec<Vec<f32>> = vec![vec![0.0; n]; slot.io.total_out];
    let mut peak = 0.0f32;
    for _ in 0..200 {
        let ins: Vec<&[f32]> = input.iter().map(|v| &v[..]).collect();
        let mut outs: Vec<&mut [f32]> = out.iter_mut().map(|v| &mut v[..]).collect();
        let (bi, bo): (Vec<BusRange>, Vec<BusRange>) =
            (slot.io.bus_in.clone(), slot.io.bus_out.clone());
        let mut buf = AudioBuffer::new(&ins, &mut outs, n, &bi, &bo);
        let mut oe = EventList::new();
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            max_block_size: 1024,
            transport: None,
            output_events: &mut oe,
        };
        slot.plugin.process(&mut buf, &EventList::new(), &mut ctx)?;
        peak = peak.max(out[0].iter().fold(0.0, |m, s| m.max(s.abs())));
    }
    println!("processed 200 blocks OK, output peak {peak:.3}, has_editor = {}", slot.descriptor.has_editor);
    Ok(())
}
