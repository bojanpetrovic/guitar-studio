//! Checks the pedalboard's parameter path with a real plugin: list → set → read back.
//! cargo run -p gs-engine --example param_control -- <format> <name substring>
use gs_engine::Studio;
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};

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
    let desc = scan_format_in_process(&args[1])?
        .into_iter()
        .find(|p| p.name.to_lowercase().contains(&args[2].to_lowercase()))
        .ok_or_else(|| anyhow::anyhow!("no match"))?;
    let mut studio = Studio::new();
    let id = studio.add_plugin_node(load_plugin(&desc, 48_000.0, MAX_BLOCK)?, 0.0, 0.0)?;
    let params = studio.plugin_params(id)?;
    let stepped = params.iter().filter(|p| p.steps > 0).count();
    println!(
        "{}: {} visible params ({} stepped)",
        desc.name,
        params.len(),
        stepped
    );
    let mut pass = true;
    for p in params
        .iter()
        .filter(|p| p.steps == 0 && !p.is_bypass)
        .take(4)
    {
        let target = p.min + (p.max - p.min) * 0.37;
        studio.set_plugin_param(id, p.id, target)?;
        pump(0.15);
        let got = studio.param_values(&[(id, p.id)])[0].unwrap_or(f64::NAN);
        let ok = (got - target).abs() < (p.max - p.min) * 0.02;
        pass &= ok;
        println!(
            "  {:<28} set {target:>8.3} {:<4} read {got:>8.3}  {}",
            p.name,
            p.unit,
            if ok { "OK" } else { "MISMATCH" }
        );
    }
    // Out-of-range values clamp; unknown ids error; a vanished node reads as None.
    let p = &params[0];
    studio.set_plugin_param(id, p.id, p.max + 1000.0)?;
    pump(0.1);
    let clamped = studio.param_values(&[(id, p.id)])[0].unwrap();
    pass &= (clamped - p.max).abs() < (p.max - p.min) * 0.02;
    pass &= studio.set_plugin_param(id, u32::MAX, 0.0).is_err();
    pass &= studio.param_values(&[(9999, p.id)])[0].is_none();
    println!("clamp/unknown/missing checks done");
    println!(
        "{}",
        if pass {
            "PARAM CONTROL: PASS"
        } else {
            "PARAM CONTROL: FAIL"
        }
    );
    std::process::exit(if pass { 0 } else { 1 });
}
