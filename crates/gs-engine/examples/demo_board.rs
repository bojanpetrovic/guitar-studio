//! Writes a "Pedalboard Demo" preset (Nolly AU → Timeless AU) with a wired-up board
//! into the given presets directory.
//! cargo run -p gs-engine --example demo_board -- <presets dir>
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use gs_engine::presets::PresetStore;
use serde_json::json;

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
    let nolly = s.add_plugin_node(
        load_plugin(&find("nolly"), 48_000.0, MAX_BLOCK)?,
        300.0,
        0.0,
    )?;
    let delay = s.add_plugin_node(
        load_plugin(&find("timeless"), 48_000.0, MAX_BLOCK)?,
        600.0,
        0.0,
    )?;
    s.move_node(2, 900.0, 0.0);
    s.disconnect(1, 2)?;
    for (a, b) in [(1, nolly), (nolly, delay), (delay, 2)] {
        s.connect(a, b)?;
    }
    let p = |node: u32, id: u32, name: &str| json!({ "kind": "param", "node": node, "paramId": id, "paramName": name });
    let w = |id: &str,
             ty: &str,
             x: i32,
             y: i32,
             ww: i32,
             h: i32,
             label: &str,
             color: &str,
             binding: serde_json::Value,
             options: serde_json::Value| {
        json!({ "id": id, "type": ty, "x": x, "y": y, "w": ww, "h": h, "label": label, "color": color, "binding": binding, "options": options })
    };
    let none = json!({ "kind": "none" });
    let board = json!({
        "version": 1, "width": 1200, "height": 420, "surface": "slate",
        "widgets": [
            w("p1", "panel", 30, 30, 400, 350, "ARCHETYPE NOLLY", "#2f6fd6", none.clone(), json!({})),
            w("k1", "knob", 60, 60, 90, 120, "Rhythm Gain", "#f2f2f2", p(nolly, 1206835472, "Rhythm Gain"), json!({})),
            w("k2", "knob", 170, 60, 90, 120, "Output", "#f2f2f2", p(nolly, 1655763259, "Output Gain"), json!({})),
            w("s1", "selector", 280, 50, 130, 160, "Amp", "#f2f2f2", p(nolly, 599569593, "Amp Type"), json!({ "labels": ["Clean", "Crunch", "Rhythm", "Lead"] })),
            w("f1", "footswitch", 180, 210, 100, 130, "ON / OFF", "#ff5d5d", json!({ "kind": "bypass", "node": nolly }), json!({})),
            w("p2", "panel", 460, 30, 260, 350, "TIMELESS 3", "#e0602b", none.clone(), json!({})),
            w("k3", "knob", 490, 60, 90, 120, "Time", "#f2f2f2", p(delay, 0, "Delay Time"), json!({})),
            w("k4", "knob", 600, 60, 90, 120, "Feedback", "#f2f2f2", p(delay, 85, "Feedback"), json!({})),
            w("f2", "footswitch", 540, 210, 100, 130, "ON / OFF", "#ff5d5d", json!({ "kind": "bypass", "node": delay }), json!({})),
            w("t1", "tuner", 750, 40, 240, 110, "Tuner", "#ffa630", none.clone(), json!({})),
            w("f3", "footswitch", 760, 210, 100, 130, "TAP", "#ffa630", json!({ "kind": "tap" }), json!({})),
            w("f4", "footswitch", 880, 210, 100, 130, "CLICK", "#3ddc97", json!({ "kind": "metronome" }), json!({})),
            w("m1", "meter", 1020, 40, 50, 300, "OUT", "#ffa630", json!({ "kind": "meter", "node": "master" }), json!({})),
            w("v1", "fader", 1090, 40, 70, 300, "Master", "#ffa630", json!({ "kind": "level", "node": 2 }), json!({})),
        ]
    });
    s.set_pedalboard(board);
    let (patch, warnings) = s.to_patch("Pedalboard Demo");
    PresetStore::new(&dir)?.save(&patch)?;
    println!("saved Pedalboard Demo ({} warnings)", warnings.len());
    Ok(())
}
