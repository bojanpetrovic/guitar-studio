//! Checks scenes with a real plugin: two scenes with different settings, switch back
//! and forth, tweak inside a scene, then save + reload the rig.
//! cargo run -p gs-engine --example scene_switch -- <format> <name substring>
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use gs_engine::presets::{Patch, instantiate_plugins};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

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

/// Let changes land the way they do in the app: pump the run loop (JUCE applies
/// changes there) and run a few audio blocks (VST3 delivers parameters in process()).
fn settle(studio: &Studio, id: u32) {
    pump(0.2);
    let Some(plugin) = studio.plugin(id) else {
        return;
    };
    let mut slot = plugin.lock();
    let n = 256;
    let input = vec![vec![0.0f32; n]; slot.io.total_in];
    let mut out = vec![vec![0.0f32; n]; slot.io.total_out];
    for _ in 0..4 {
        let ins: Vec<&[f32]> = input.iter().map(|v| &v[..]).collect();
        let mut outs: Vec<&mut [f32]> = out.iter_mut().map(|v| &mut v[..]).collect();
        let (bi, bo) = (slot.io.bus_in.clone(), slot.io.bus_out.clone());
        let mut buf = AudioBuffer::new(&ins, &mut outs, n, &bi, &bo);
        let mut events_out = EventList::new();
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            max_block_size: MAX_BLOCK,
            transport: None,
            output_events: &mut events_out,
        };
        let _ = slot.plugin.process(&mut buf, &EventList::new(), &mut ctx);
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let desc = scan_format_in_process(&args[1])?
        .into_iter()
        .find(|p| p.name.to_lowercase().contains(&args[2].to_lowercase()))
        .ok_or_else(|| anyhow::anyhow!("no match"))?;
    let mut studio = Studio::new();
    let id = studio.add_plugin_node(load_plugin(&desc, 48_000.0, MAX_BLOCK)?, 0.0, 0.0)?;
    let params = studio.plugin_params(id)?;
    let p = params
        .iter()
        .find(|p| p.steps == 0 && !p.is_bypass && p.max > p.min)
        .ok_or_else(|| anyhow::anyhow!("no continuous parameter"))?
        .clone();
    let at = |f: f64| p.min + (p.max - p.min) * f;
    let read = |s: &Studio| s.param_values(&[(id, p.id)])[0].unwrap_or(f64::NAN);
    let close = |a: f64, b: f64| (a - b).abs() < (p.max - p.min) * 0.02;
    println!(
        "{}: using '{}' ({} params)",
        desc.name,
        p.name,
        params.len()
    );

    let mut pass = true;
    let mut check = |label: &str, ok: bool| {
        pass &= ok;
        println!("  {label:<46} {}", if ok { "OK" } else { "FAIL" });
    };

    // Edits go into the active scene, so: make a scene, then shape its sound.
    studio.add_scene(Some("Clean"))?;
    studio.set_plugin_param(id, p.id, at(0.2))?;
    settle(&studio, id);
    studio.add_scene(Some("Lead"))?; // starts as a copy of Clean
    studio.set_plugin_param(id, p.id, at(0.8))?;
    studio.set_bypass(id, true)?;
    settle(&studio, id);

    let t = std::time::Instant::now();
    studio.select_scene(0)?;
    println!("  switch took {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    settle(&studio, id);
    check("scene 1 restores its value", close(read(&studio), at(0.2)));
    check(
        "scene 1 restores bypass off",
        !studio.view().nodes.iter().any(|n| n.id == id && n.bypass),
    );
    studio.select_scene(1)?;
    settle(&studio, id);
    check("scene 2 restores its value", close(read(&studio), at(0.8)));
    check(
        "scene 2 restores bypass on",
        studio.view().nodes.iter().any(|n| n.id == id && n.bypass),
    );

    // A tweak inside a scene is kept when you leave and come back.
    studio.set_plugin_param(id, p.id, at(0.6))?;
    settle(&studio, id);
    studio.select_scene(0)?;
    settle(&studio, id);
    studio.select_scene(1)?;
    settle(&studio, id);
    check(
        "tweaks made in a scene stick",
        close(read(&studio), at(0.6)),
    );

    // Save in scene 2 with a live tweak, reload, and switch scenes in the copy.
    studio.set_plugin_param(id, p.id, at(0.7))?;
    settle(&studio, id);
    let (patch, warnings) = studio.to_patch("Scenes");
    let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch)?)?;
    let (slots, w2) = instantiate_plugins(&patch, 48_000.0);
    for w in warnings.iter().chain(&w2) {
        println!("  warning: {w}");
    }
    let mut copy = Studio::new();
    copy.apply_patch(&patch, slots)?;
    settle(&copy, id);
    check(
        "reloaded rig is in scene 2",
        copy.view().active_scene == Some(1),
    );
    check(
        "reloaded rig has the live tweak",
        close(read(&copy), at(0.7)),
    );
    copy.select_scene(0)?;
    settle(&copy, id);
    check("reloaded scene 1 still works", close(read(&copy), at(0.2)));
    copy.select_scene(1)?;
    settle(&copy, id);
    check(
        "reloaded scene 2 kept the saved tweak",
        close(read(&copy), at(0.7)),
    );

    println!("{}", if pass { "SCENES: PASS" } else { "SCENES: FAIL" });
    std::process::exit(if pass { 0 } else { 1 });
}
