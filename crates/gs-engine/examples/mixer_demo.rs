//! Writes a "Mixer Demo" preset: two parallel paths (Nolly AU, Timeless AU) into a
//! Mixer with different levels and pans.
//! cargo run -p gs-engine --example mixer_demo -- <presets dir>
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use gs_engine::presets::PresetStore;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).expect("presets dir");
    let au = scan_format_in_process("au")?;
    let find = |n: &str| {
        au.iter()
            .find(|p| p.name.to_lowercase().contains(n))
            .cloned()
            .unwrap()
    };
    let mut s = Studio::new();
    let amp = s.add_plugin_node(
        load_plugin(&find("nolly"), 48_000.0, MAX_BLOCK)?,
        320.0,
        -160.0,
    )?;
    let delay = s.add_plugin_node(
        load_plugin(&find("timeless"), 48_000.0, MAX_BLOCK)?,
        320.0,
        160.0,
    )?;
    let mix = s.add_mixer_node(640.0, 0.0)?;
    s.move_node(2, 1000.0, 0.0);
    s.disconnect(1, 2)?;
    for (a, b) in [(1, amp), (1, delay), (amp, mix), (delay, mix), (mix, 2)] {
        s.connect(a, b)?;
    }
    s.set_mixer_strip(mix, amp, "gainDb", -3.0)?;
    s.set_mixer_strip(mix, amp, "pan", -0.3)?;
    s.set_mixer_strip(mix, delay, "gainDb", -9.5)?;
    s.set_mixer_strip(mix, delay, "pan", 0.45)?;
    let (patch, _) = s.to_patch("Mixer Demo");
    PresetStore::new(&dir)?.save(&patch)?;
    println!("saved Mixer Demo");
    Ok(())
}
