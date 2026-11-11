//! Load the built plugin bundles the way a DAW does, headless (through truce-rack, the
//! library Guitar Studio hosts plugins with), and check:
//! - audio passes through the default rig (Input → Output);
//! - the Output parameter turns it down;
//! - saved state is a Guitar Studio rig, and restores into a second instance;
//! - (CLAP, with a preset) a saved preset restored as project state brings up its
//!   plugins and changes the sound.
//!
//! `make plugin-test` runs it on `target/plugin/*`:
//! cargo run -p gs-plugin --example host_check -- <bundle>… [--preset <file.json>]
use anyhow::{Context, Result, bail};
use gs_engine::plugins::{PluginSlot, load_plugin, scan_path_in_process};
use std::path::Path;
use std::time::{Duration, Instant};
use truce::core::state::{deserialize_state, hash_plugin_id, serialize_state};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

const SR: f64 = 48_000.0;
const BLOCK: usize = 256;
const INPUT_PEAK: f32 = 0.3;

/// Let the main run loop spin, as a DAW's does: the plugin sets up restored rigs there.
fn pump_main_loop(secs: f64) {
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const std::ffi::c_void;
        fn CFRunLoopRunInMode(
            mode: *const std::ffi::c_void,
            seconds: f64,
            return_after_source: u8,
        ) -> i32;
    }
    // SAFETY: plain CoreFoundation call on the main thread.
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
}

/// Process `blocks` blocks of a 110 Hz sine; returns the output peak.
fn run(slot: &mut PluginSlot, blocks: usize) -> Result<f32> {
    let sine: Vec<f32> = (0..BLOCK)
        .map(|i| INPUT_PEAK * (i as f32 * std::f32::consts::TAU * 110.0 / SR as f32).sin())
        .collect();
    let input: Vec<Vec<f32>> = vec![sine; slot.io.total_in];
    let mut out: Vec<Vec<f32>> = vec![vec![0.0; BLOCK]; slot.io.total_out];
    let mut peak = 0.0f32;
    for _ in 0..blocks {
        let ins: Vec<&[f32]> = input.iter().map(|v| &v[..]).collect();
        let mut outs: Vec<&mut [f32]> = out.iter_mut().map(|v| &mut v[..]).collect();
        let (bi, bo) = (slot.io.bus_in.clone(), slot.io.bus_out.clone());
        let mut buf = AudioBuffer::new(&ins, &mut outs, BLOCK, &bi, &bo);
        let mut out_events = EventList::new();
        let mut ctx = ProcessContext {
            sample_rate: SR,
            max_block_size: 1024,
            transport: None,
            output_events: &mut out_events,
        };
        slot.plugin.process(&mut buf, &EventList::new(), &mut ctx)?;
        for ch in &out {
            peak = peak.max(ch.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
    }
    Ok(peak)
}

fn load(format: &str, bundle: &Path) -> Result<PluginSlot> {
    // Path scans walk a folder of bundles.
    let folder = bundle.parent().unwrap_or(Path::new("."));
    let list = scan_path_in_process(format, folder)?;
    let desc = list
        .iter()
        .find(|p| p.name == "Guitar Studio" && Path::new(&p.path) == bundle)
        .with_context(|| format!("no Guitar Studio plugin in {}", bundle.display()))?;
    let slot = load_plugin(desc, SR, 1024)?;
    pump_main_loop(0.1);
    Ok(slot)
}

/// The rig inside a saved state, if it's a CLAP / truce envelope we can read.
fn saved_rig(state: &[u8]) -> Option<String> {
    let parsed = deserialize_state(state, hash_plugin_id("dev.guitarstudio.plugin"))?;
    Some(String::from_utf8_lossy(&parsed.extra?).into_owned())
}

fn check(bundle: &Path, preset: Option<&Path>) -> Result<()> {
    let format = match bundle.extension().and_then(|e| e.to_str()) {
        Some("clap") => "clap",
        Some("vst3") => "vst3",
        _ => bail!("{}: not a .clap or .vst3 bundle", bundle.display()),
    };
    println!("== {format}: {}", bundle.display());
    let mut a = load(format, bundle)?;
    println!("   loaded: {} in / {} out", a.io.main_in, a.io.main_out);

    let peak = run(&mut a, 50)?;
    println!("   passthrough peak {peak:.3} (input {INPUT_PEAK})");
    if (peak - INPUT_PEAK).abs() > 0.02 {
        bail!("the default rig should pass the input through");
    }

    let output = (0..a.plugin.parameter_count())
        .find(|&i| a.plugin.parameter_info(i).is_ok_and(|p| p.name == "Output"))
        .context("no Output parameter")?;
    let info = a.plugin.parameter_info(output)?;
    a.plugin.set_parameter(output, info.min)?; // -60 dB = silent
    run(&mut a, 4)?;
    let quiet = run(&mut a, 10)?;
    println!("   Output at minimum: peak {quiet:.4}");
    if quiet > 0.001 {
        bail!("the Output parameter didn't turn the rig down");
    }
    a.plugin.set_parameter(output, info.default)?; // 0 dB
    let back = run(&mut a, 10)?;
    if (back - INPUT_PEAK).abs() > 0.02 {
        bail!("the Output parameter didn't come back up (peak {back:.3})");
    }

    let state = a.plugin.save_state()?;
    println!("   saved state: {} bytes", state.len());
    if format == "clap" {
        let rig = saved_rig(&state).context("state isn't a truce envelope")?;
        if !rig.contains("guitar-studio-rig") {
            bail!("saved state holds no rig: {rig:.200}");
        }
    }
    let mut b = load(format, bundle)?;
    b.plugin.load_state(&state)?;
    for _ in 0..10 {
        run(&mut b, 4)?;
        pump_main_loop(0.05);
    }
    let restored = run(&mut b, 20)?;
    println!("   restored into a second instance: peak {restored:.3}");
    if (restored - INPUT_PEAK).abs() > 0.02 {
        bail!("the restored instance should pass the input through");
    }

    if let (Some(preset), "clap") = (preset, format) {
        let patch: serde_json::Value = serde_json::from_slice(&std::fs::read(preset)?)?;
        let names: Vec<String> = patch["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n["plugin"]["name"].as_str().map(String::from))
            .collect();
        let rig = serde_json::json!({
            "format": "guitar-studio-rig",
            "version": 1,
            "preset": patch["name"],
            "patch": patch,
        });
        let extra = serde_json::to_vec(&rig)?;
        let envelope = serialize_state(
            hash_plugin_id("dev.guitarstudio.plugin"),
            &[],
            &[],
            &extra,
            &[],
        );
        println!(
            "   restoring preset {} ({})",
            preset.display(),
            names.join(", ")
        );
        let t = Instant::now();
        b.plugin.load_state(&envelope)?;
        // Restored state is re-saved from the live rig once it's up (compact JSON, so
        // it differs from the pretty preset file).
        loop {
            run(&mut b, 4)?;
            pump_main_loop(0.05);
            let now = b.plugin.save_state()?;
            let live = saved_rig(&now).unwrap_or_default();
            if live.as_bytes() != extra.as_slice()
                && names.iter().all(|n| live.contains(n.as_str()))
            {
                break;
            }
            if t.elapsed() > Duration::from_secs(60) {
                bail!("the preset's rig didn't come up within a minute");
            }
        }
        println!("   rig up in {:.1?}", t.elapsed());
        let peak = run(&mut b, 200)?;
        println!("   through the preset's rig: peak {peak:.3}");
        if peak < 0.001 || (peak - INPUT_PEAK).abs() < 0.005 {
            bail!("the preset's rig should change the sound");
        }
    }
    drop(b);
    drop(a);
    pump_main_loop(0.3);
    println!("   ok");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let preset = args
        .iter()
        .position(|a| a == "--preset")
        .and_then(|i| args.get(i + 1))
        .map(Path::new);
    let bundles: Vec<&Path> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--preset"))
        .map(|(_, a)| Path::new(a.as_str()))
        .collect();
    if bundles.is_empty() {
        bail!("usage: host_check <bundle.clap|bundle.vst3>… [--preset <file.json>]");
    }
    for bundle in bundles {
        check(bundle, preset)?;
    }
    Ok(())
}
