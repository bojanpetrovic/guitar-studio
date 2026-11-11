//! Replace a plugin in a wired rig with Apple's built-in AUs: cables, bypass and the
//! node id survive, scenes drop the old plugin's parameters.
//!
//! cargo run -p gs-engine --example replace_plugin
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use parking_lot::Mutex;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let catalog = scan_format_in_process("au")?;
    let find = |name: &str| {
        catalog
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no {name}"))
    };
    let (delay, dist) = (find("AUDelay")?, find("AUDistortion")?);

    let mut studio = Studio::new();
    let p = studio.add_plugin_node(
        load_plugin(&delay, studio.sample_rate(), MAX_BLOCK)?,
        300.0,
        0.0,
    )?;
    let g = studio.add_gain_node(600.0, 0.0)?;
    studio.disconnect(1, 2)?;
    for (a, b) in [(1, p), (p, g), (g, 2)] {
        studio.connect(a, b)?;
    }
    studio.set_bypass(p, true)?;
    studio.add_scene(Some("Lead"))?;
    let (before, _) = studio.to_patch("t");
    let delay_params = before.scenes[0].params[&p].len();

    let new = Arc::new(Mutex::new(load_plugin(
        &dist,
        studio.sample_rate(),
        MAX_BLOCK,
    )?));
    let old = studio.replace_plugin(p, new)?;
    assert_eq!(old.lock().descriptor.name, "AUDelay");

    let view = studio.view();
    let node = view.nodes.iter().find(|n| n.id == p).unwrap();
    assert_eq!(node.plugin.as_ref().unwrap().name, "AUDistortion");
    assert!(node.bypass, "on/off state kept");
    let edges: Vec<(u32, u32)> = view.edges.iter().map(|e| (e.from, e.to)).collect();
    assert!(
        edges.contains(&(1, p)) && edges.contains(&(p, g)),
        "cables kept: {edges:?}"
    );
    assert!(
        studio
            .replace_plugin(
                g,
                Arc::new(Mutex::new(load_plugin(&dist, 48_000.0, MAX_BLOCK)?))
            )
            .is_err()
    );

    // The scene now records the new plugin's parameters, not the old ones.
    let (after, _) = studio.to_patch("t");
    let names = studio.plugin_params(p)?.len();
    assert_eq!(after.scenes[0].params[&p].len(), names);
    println!(
        "PASS: AUDelay → AUDistortion in node {p}; cables and bypass kept; scene now has {names} distortion params (was {delay_params} delay params)"
    );
    Ok(())
}
