//! Where does a preset switch spend its time? Loads every preset in a folder and times,
//! per plugin: instantiating it, restoring its saved state, restoring that state again
//! into the already-running instance (what reusing an instance would cost), and
//! tearing it down. Then switches between all presets the way the app does, reusing
//! running instances, and times each switch.
//!
//! cargo run --release -p gs-engine --example preset_timing -- "<presets dir>"
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::load_plugin;
use gs_engine::presets::{Patch, PresetStore, instantiate_plugins};
use gs_engine::studio::NodeType;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).expect("presets dir");
    let store = PresetStore::new(&dir)?;
    for info in store.list() {
        let patch = store.load(&info.name)?;
        println!("\n{}", info.name);
        for node in patch.nodes.iter().filter(|n| n.kind == NodeType::Plugin) {
            let Some(desc) = &node.plugin else { continue };
            let t = Instant::now();
            let mut slot = load_plugin(desc, 48_000.0, MAX_BLOCK)?;
            let load = ms(t);
            let (mut state, mut again) = (0.0, 0.0);
            if let Some(s) = &node.state {
                let bytes = Patch::decode_state(s)?;
                let t = Instant::now();
                slot.plugin.load_state(&bytes)?;
                state = ms(t);
                let t = Instant::now();
                slot.plugin.load_state(&bytes)?;
                again = ms(t);
            }
            let t = Instant::now();
            drop(slot);
            let teardown = ms(t);
            println!(
                "  {:<22} load {load:>7.1} ms   state {state:>6.1} ms   state again {again:>6.1} ms   teardown {teardown:>6.1} ms",
                desc.name
            );
        }
    }

    // Switch A → B → C → A … like the app: the running rig's instances and the spares
    // left over from earlier switches are offered for reuse.
    println!("\nswitching (reusing instances)");
    let names: Vec<String> = store.list().into_iter().map(|p| p.name).collect();
    let mut studio = Studio::new();
    let mut pool = Vec::new();
    for name in names.iter().chain(names.iter()) {
        let patch = store.load(name)?;
        let t = Instant::now();
        let mut spares = studio.plugin_instances();
        spares.append(&mut pool);
        let (slots, warnings) = instantiate_plugins(&patch, studio.sample_rate(), &mut spares);
        studio.apply_patch(&patch, slots)?;
        let fresh = patch.nodes.iter().filter(|n| n.plugin.is_some()).count();
        println!(
            "  → {name:<18} {:>7.1} ms  {warnings:?} (rig has {fresh} plugins, {} spare left)",
            ms(t),
            spares.len()
        );
        pool = spares;
    }
    Ok(())
}
