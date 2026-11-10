//! Headless plugin check: load, process audio, and round-trip state (what presets do).
//!
//! cargo run -p gs-engine --example plugin_smoke -- <format> <name substring> [bundle path]
use gs_engine::plugins::{PluginSlot, load_plugin, scan_format_in_process, scan_path_in_process};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

const SR: f64 = 48_000.0;

/// Let the main run loop spin, like the app's UI thread does. JUCE-based plugins
/// (e.g. Neural DSP) apply parameter changes and state on the main thread.
#[cfg(target_os = "macos")]
fn pump_main_loop(secs: f64) {
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const std::ffi::c_void;
        fn CFRunLoopRunInMode(
            mode: *const std::ffi::c_void,
            seconds: f64,
            return_after_source: u8,
        ) -> i32;
    }
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
}
#[cfg(not(target_os = "macos"))]
fn pump_main_loop(_secs: f64) {}
const BLOCK: usize = 256;

/// Process `blocks` blocks of a 110 Hz sine; returns the output peak.
fn run(slot: &mut PluginSlot, blocks: usize) -> anyhow::Result<f32> {
    let sine: Vec<f32> = (0..BLOCK)
        .map(|i| 0.3 * (i as f32 * std::f32::consts::TAU * 110.0 / SR as f32).sin())
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
        if let Some(ch) = out.first() {
            peak = peak.max(ch.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
    }
    pump_main_loop(0.2);
    Ok(peak)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let format = args.get(1).map(String::as_str).unwrap_or("vst3");
    let needle = args.get(2).map(|s| s.to_lowercase()).unwrap_or_default();
    let t = std::time::Instant::now();
    let list = match args.get(3) {
        Some(path) => scan_path_in_process(format, std::path::Path::new(path))?,
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
    let mut slot = load_plugin(desc, SR, 1024)?;
    println!(
        "loaded in {:?}; in/out channels {}/{}; {} params; editor: {}",
        t.elapsed(),
        slot.io.main_in,
        slot.io.main_out,
        slot.plugin.parameter_count(),
        slot.descriptor.has_editor
    );
    println!(
        "processed 200 blocks, output peak {:.3}",
        run(&mut slot, 200)?
    );

    // Preset round trip: save the current state, move a parameter away, restore,
    // and expect the saved value back. Audio and the run loop spin between steps, as
    // in the app (JUCE plugins apply changes on the main thread).
    let count = slot.plugin.parameter_count();
    let is_switch = |name: &str| {
        let n = name.to_lowercase();
        ["bypass", "used", "enable", " on", "solo", "mute", "active"]
            .iter()
            .any(|w| n.contains(w))
    };
    let state = slot.plugin.save_state()?;
    let mut tested = 0;
    let mut failed = false;
    for idx in 0..count.min(80) {
        if tested == 3 {
            break;
        }
        let Ok(info) = slot.plugin.parameter_info(idx) else {
            continue;
        };
        if info.max <= info.min || info.step_count > 0 || is_switch(&info.name) {
            continue;
        }
        let span = info.max - info.min;
        let saved = slot.plugin.parameter_value(idx)?;
        let other = if saved - info.min > span * 0.5 {
            info.min + span * 0.1
        } else {
            info.min + span * 0.9
        };
        slot.plugin.set_parameter(idx, other)?;
        run(&mut slot, 10)?;
        let moved = slot.plugin.parameter_value(idx)?;
        if (moved - saved).abs() < span * 0.1 {
            continue; // parameter doesn't respond; try the next one
        }
        slot.plugin.load_state(&state)?;
        run(&mut slot, 10)?;
        let restored = slot.plugin.parameter_value(idx)?;
        let ok = (restored - saved).abs() < span * 0.02;
        println!(
            "state: {} bytes; '{}' saved {saved:.3}, moved to {moved:.3}, restored {restored:.3}: {}",
            state.len(),
            info.name,
            if ok { "OK" } else { "MISMATCH" }
        );
        tested += 1;
        failed |= !ok;
    }
    if tested == 0 {
        println!("no responsive parameter found to test state with");
    }
    if failed {
        std::process::exit(2);
    }
    Ok(())
}
