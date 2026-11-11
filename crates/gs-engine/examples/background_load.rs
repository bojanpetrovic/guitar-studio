//! Can a plugin be started off the main thread? Loads it on a worker thread while the
//! main thread keeps its run loop going (as the app's UI does), and reports how long
//! the load took, the longest the main thread was ever held up, and whether the
//! instance processes audio afterwards.
//!
//! cargo run --release -p gs-engine --example background_load -- <format> "<plugin>" [...]
use gs_engine::nodes::MAX_BLOCK;
use gs_engine::plugins::{load_plugin, scan_format_in_process};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use truce_rack::core::{AudioBuffer, EventList, ProcessContext};

fn pump(secs: f64) {
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const std::ffi::c_void;
        fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, s: f64, r: u8) -> i32;
    }
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
}

/// Open the plugin's editor in a plain AppKit window on the main thread, let it settle
/// and report its size (0×0 / 1×1 = it didn't come up).
fn open_editor_briefly(slot: &mut gs_engine::plugins::PluginSlot) -> String {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let mtm = MainThreadMarker::new().expect("main thread");
    let w = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(640.0, 420.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { w.setReleasedWhenClosed(false) };
    w.makeKeyAndOrderFront(None);
    let view = w.contentView().unwrap();
    let ptr = objc2::rc::Retained::as_ptr(&view) as *mut std::ffi::c_void;
    let Some(ed) = slot.plugin.editor() else {
        return "no editor".into();
    };
    if let Err(e) = ed.open(truce_rack::core::WindowHandle::NSView(ptr), 2.0) {
        return format!("FAILED: {e}");
    }
    pump(2.5);
    let frame = view.subviews().firstObject().map(|v| v.frame().size);
    ed.close();
    w.close();
    match frame {
        Some(s) if s.width > 20.0 && s.height > 20.0 => {
            format!("opened {}×{} PASS", s.width, s.height)
        }
        other => format!("didn't come up ({other:?})"),
    }
}

fn main() -> anyhow::Result<()> {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
    }
    let args: Vec<String> = std::env::args().collect();
    let format = args[1].clone();
    let catalog = scan_format_in_process(&format)?;
    for needle in &args[2..] {
        let desc = catalog
            .iter()
            .find(|p| p.name.to_lowercase().contains(&needle.to_lowercase()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no plugin matching {needle}"))?;
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        // TWINS=1: start a second instance of the same plugin at the same time on
        // another thread (a preload racing a load).
        let twin = std::env::var("TWINS").is_ok().then(|| {
            let d = desc.clone();
            std::thread::spawn(move || {
                load_plugin(&d, 48_000.0, MAX_BLOCK).map(|_| started.elapsed())
            })
        });
        let d = desc.clone();
        std::thread::spawn(move || {
            let r = load_plugin(&d, 48_000.0, MAX_BLOCK);
            let _ = tx.send((r, started.elapsed()));
        });
        // The "UI": keep the main run loop turning and note the longest stall.
        // MAIN_BLOCKED=1: keep the main thread busy instead (never serving its run loop),
        // to see whether the load needs the main thread.
        let blocked = std::env::var("MAIN_BLOCKED").is_ok();
        let mut worst = Duration::ZERO;
        let (result, took) = loop {
            let t = Instant::now();
            if blocked {
                std::thread::sleep(Duration::from_millis(10));
                if started.elapsed() > Duration::from_secs(60) {
                    println!("{:<22} HUNG: needs the main thread", desc.name);
                    std::process::exit(1);
                }
            } else {
                pump(0.01);
            }
            worst = worst.max(t.elapsed());
            if let Ok(done) = rx.try_recv() {
                break done;
            }
        };
        match result {
            Ok(mut slot) => {
                // Feed a sine through it and check something comes out.
                let n = 256;
                let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
                let mut energy = 0.0f32;
                for _ in 0..40 {
                    let ins: Vec<&[f32]> =
                        (0..slot.io.total_in).map(|_| input.as_slice()).collect();
                    let mut outs_store = vec![vec![0.0f32; n]; slot.io.total_out];
                    let mut outs: Vec<&mut [f32]> =
                        outs_store.iter_mut().map(|v| v.as_mut_slice()).collect();
                    let mut events = EventList::new();
                    let mut buffer =
                        AudioBuffer::new(&ins, &mut outs, n, &slot.io.bus_in, &slot.io.bus_out);
                    let mut ctx = ProcessContext {
                        sample_rate: 48_000.0,
                        max_block_size: MAX_BLOCK,
                        transport: None,
                        output_events: &mut events,
                    };
                    slot.plugin
                        .process(&mut buffer, &EventList::new(), &mut ctx)?;
                    energy += outs_store.iter().flatten().map(|s| s * s).sum::<f32>();
                    pump(0.005);
                }
                println!(
                    "{:<22} loaded off-main in {:>7.0} ms; main thread's longest stall {:>5.0} ms; output energy {:.3} {}",
                    desc.name,
                    took.as_secs_f64() * 1000.0,
                    worst.as_secs_f64() * 1000.0,
                    energy,
                    if energy > 1e-6 { "PASS" } else { "(silent)" }
                );
                if let Some(t) = twin {
                    match t.join() {
                        Ok(Ok(took)) => println!(
                            "  twin loaded alongside in {:.0} ms PASS",
                            took.as_secs_f64() * 1000.0
                        ),
                        Ok(Err(e)) => println!("  twin FAILED: {e:#}"),
                        Err(_) => println!("  twin PANICKED"),
                    }
                }
                if std::env::var("EDITOR_CHECK").is_ok() {
                    println!("  editor: {}", open_editor_briefly(&mut slot));
                }
                drop(slot); // dropped on the main thread, as the app does
            }
            Err(e) => println!("{:<22} FAILED off-main: {e:#}", desc.name),
        }
    }
    Ok(())
}
