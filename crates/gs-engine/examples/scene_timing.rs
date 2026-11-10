//! How long does the control thread hold a plugin's lock per parameter call? Every
//! hold must stay well under the audio thread's wait (250 µs). Also shows the stall
//! behind `plugins::READ_QUIET`: on JUCE AUs a read right after a write blocks ~40 ms.
//! cargo run -p gs-engine --example scene_timing -- au nolly
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use parking_lot::Mutex;
use std::time::{Duration, Instant};

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
    let plugin = Mutex::new(load_plugin(&desc, 48_000.0, MAX_BLOCK)?);
    let params: Vec<(usize, f64, f64)> = plugin
        .lock()
        .params
        .iter()
        .map(|p| (p.index, p.min, p.max))
        .collect();
    let (mut get_max, mut set_max, mut get_sum) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    // Rounds 0-1: reads only. Round 2: read + write per parameter, with the run loop
    // turning in between (as it does in the app between commands). Round 3: a whole
    // scene switch's shape — all reads, then all writes, then reads again after the
    // run loop has turned.
    for round in 0..4 {
        let (mut rmax, mut rsum) = (Duration::ZERO, Duration::ZERO);
        for &(index, lo, hi) in &params {
            let t = Instant::now();
            let v = plugin.lock().plugin.parameter_value(index).unwrap_or(lo);
            let d = t.elapsed();
            get_max = get_max.max(d);
            get_sum += d;
            rmax = rmax.max(d);
            rsum += d;
            if round == 2 {
                let t = Instant::now();
                let _ = plugin
                    .lock()
                    .plugin
                    .set_parameter(index, v.clamp(lo.min(hi), hi.max(lo)));
                set_max = set_max.max(t.elapsed());
                pump(0.002);
            }
        }
        if round == 2 {
            pump(0.1);
        }
        println!(
            "  round {round}: read avg {:.0} µs, slowest {:.0} µs",
            rsum.as_secs_f64() * 1e6 / params.len() as f64,
            rmax.as_secs_f64() * 1e6
        );
    }
    // After one write: does the slow read clear with time alone, or with the run loop?
    let (index, lo, hi) = params[0];
    for (label, wait) in [("sleep 60 ms", 0), ("pump 60 ms", 1), ("no wait", 2)] {
        let v = plugin.lock().plugin.parameter_value(index).unwrap_or(lo);
        let _ = plugin
            .lock()
            .plugin
            .set_parameter(index, v.clamp(lo.min(hi), hi.max(lo)));
        match wait {
            0 => std::thread::sleep(Duration::from_millis(60)),
            1 => pump(0.06),
            _ => {}
        }
        let t = Instant::now();
        let _ = plugin.lock().plugin.parameter_value(params[1].0);
        println!(
            "  read after a write, {label}: {:.0} µs",
            t.elapsed().as_secs_f64() * 1e6
        );
        pump(0.1);
    }
    println!(
        "{}: {} params, read avg {:.0} µs, slowest read {:.0} µs, slowest write {:.0} µs",
        desc.name,
        params.len(),
        get_sum.as_secs_f64() * 1e6 / (params.len() * 4) as f64,
        get_max.as_secs_f64() * 1e6,
        set_max.as_secs_f64() * 1e6
    );
    Ok(())
}
