//! End-to-end preset check with a real plugin, through the same code paths the app
//! uses: load → add to studio → tweak → save to disk → load into a fresh studio.
//!
//! cargo run -p gs-engine --example preset_roundtrip -- <format> <name substring>
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use gs_engine::presets::{PresetStore, instantiate_plugins};
use gs_engine::studio::NodeType;

#[cfg(target_os = "macos")]
fn pump(secs: f64) {
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const std::ffi::c_void;
        fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, s: f64, r: u8) -> i32;
    }
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
}
#[cfg(not(target_os = "macos"))]
fn pump(_: f64) {}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (format, needle) = (args[1].as_str(), args[2].to_lowercase());
    let desc = scan_format_in_process(format)?
        .into_iter()
        .find(|p| p.name.to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow::anyhow!("no plugin matching '{needle}'"))?;

    // Build a rig: Input → plugin → Gain(-4.5 dB, bypassed) → Output, at 130 BPM.
    let mut studio = Studio::new();
    let slot = load_plugin(&desc, studio.sample_rate(), MAX_BLOCK)?;
    let p = studio.add_plugin_node(slot, 300.0, 40.0)?;
    let g = studio.add_gain_node(600.0, 40.0)?;
    studio.set_param(g, "gainDb", -4.5)?;
    studio.set_bypass(g, true)?;
    studio.disconnect(1, 2)?;
    for (a, b) in [(1, p), (p, g), (g, 2)] {
        studio.connect(a, b)?;
    }
    studio.set_metronome(gs_engine::metronome::MetronomeSettings {
        bpm: 130.0,
        ..Default::default()
    });

    // Turn a real knob inside the plugin (what the user does in its editor).
    let plugin = studio.plugin(p).unwrap();
    let (idx, name, target) = {
        let mut s = plugin.lock();
        let idx = (0..s.plugin.parameter_count())
            .find(|&i| {
                s.plugin.parameter_info(i).is_ok_and(|x| {
                    x.step_count == 0 && x.max > x.min && !x.name.to_lowercase().contains("bypass")
                })
            })
            .unwrap();
        let info = s.plugin.parameter_info(idx)?;
        let target = info.min + (info.max - info.min) * 0.83;
        s.plugin.set_parameter(idx, target)?;
        (idx, info.name, target)
    };
    pump(0.3);

    let dir = tempfile::tempdir()?;
    let store = PresetStore::new(dir.path())?;
    let (patch, warnings) = studio.to_patch("Nolly Rhythm");
    store.save(&patch)?;
    println!(
        "saved '{}' ({} warnings), file {} bytes",
        patch.name,
        warnings.len(),
        std::fs::metadata(dir.path().join("nolly rhythm.json"))?.len()
    );
    drop(plugin);
    drop(studio);

    // Fresh studio, load from disk like the app does.
    let loaded = store.load("nolly rhythm")?;
    let mut fresh = Studio::new();
    let (slots, warnings) = instantiate_plugins(&loaded, fresh.sample_rate());
    fresh.apply_patch(&loaded, slots)?;
    pump(0.3);

    let v = fresh.view();
    let chain: Vec<String> = v
        .nodes
        .iter()
        .map(|n| match n.kind {
            NodeType::Plugin => n.plugin.as_ref().unwrap().name.clone(),
            k => format!("{k:?}"),
        })
        .collect();
    let gain = v.nodes.iter().find(|n| n.kind == NodeType::Gain).unwrap();
    let restored = fresh
        .plugin(p)
        .unwrap()
        .lock()
        .plugin
        .parameter_value(idx)?;
    let edges: Vec<_> = v.edges.iter().map(|e| (e.from, e.to)).collect();

    println!("load warnings: {warnings:?}");
    println!("nodes: {chain:?}; edges: {edges:?}");
    println!(
        "gain node: {:.1} dB, bypass {}; tempo {} BPM",
        gain.gain_db, gain.bypass, v.metronome.bpm
    );
    let ok_param = (restored - target).abs() < 0.02 * target.abs().max(1.0);
    println!(
        "plugin knob '{name}': set {target:.3}, after reload {restored:.3} → {}",
        if ok_param { "OK" } else { "MISMATCH" }
    );
    let ok = ok_param
        && edges == vec![(1, p), (p, g), (g, 2)]
        && (gain.gain_db + 4.5).abs() < 1e-3
        && gain.bypass
        && (v.metronome.bpm - 130.0).abs() < 1e-3;
    println!(
        "{}",
        if ok {
            "PRESET ROUND TRIP: PASS"
        } else {
            "PRESET ROUND TRIP: FAIL"
        }
    );
    std::process::exit(if ok { 0 } else { 1 });
}
