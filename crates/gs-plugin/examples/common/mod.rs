//! What the plugin checks share: loading the built bundle as a DAW would (through
//! truce-rack), running audio through it, and spinning the main loop.

use anyhow::{Context, Result};
use gs_engine::plugins::{PluginSlot, load_plugin, scan_path_in_process};
use std::path::Path;
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

pub const SR: f64 = 48_000.0;
pub const BLOCK: usize = 256;

/// Load the Guitar Studio plugin from a `.clap` or `.vst3` bundle.
pub fn load(bundle: &Path) -> Result<PluginSlot> {
    let format = if bundle.extension().is_some_and(|e| e == "vst3") {
        "vst3"
    } else {
        "clap"
    };
    // Path scans walk a folder of bundles.
    let folder = bundle.parent().unwrap_or(Path::new("."));
    let list = scan_path_in_process(format, folder)?;
    let desc = list
        .iter()
        .find(|p| p.name == "Guitar Studio" && Path::new(&p.path) == bundle)
        .with_context(|| format!("no Guitar Studio plugin in {}", bundle.display()))?;
    let slot = load_plugin(desc, SR, 1024)?;
    pump(0.1);
    Ok(slot)
}

/// Process `blocks` blocks of a 110 Hz sine of peak `level` (0 = silence); returns the
/// output peak.
pub fn run(slot: &mut PluginSlot, blocks: usize, level: f32) -> Result<f32> {
    let sine: Vec<f32> = (0..BLOCK)
        .map(|i| level * (i as f32 * std::f32::consts::TAU * 110.0 / SR as f32).sin())
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

/// Run the main loop for `secs`, handling window events, as a DAW's event loop does
/// (the plugin sets up restored rigs there, and WebKit only draws windows it sees
/// handled).
pub fn pump(secs: f64) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    let until = NSDate::dateWithTimeIntervalSinceNow(secs);
    // SAFETY: AppKit event loop calls on the main thread.
    unsafe {
        while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&until),
            NSDefaultRunLoopMode,
            true,
        ) {
            app.sendEvent(&event);
        }
    }
}
