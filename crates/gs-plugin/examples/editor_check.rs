//! Open the built plugin's window outside a DAW: load the bundle (through truce-rack),
//! show its editor in a plain window while audio runs, and save a picture of just that
//! window. With `RUST_LOG=debug` the plugin log (~/Library/Logs/Guitar Studio/plugin.log)
//! lists the commands the page sends.
//!
//! cargo run -p gs-plugin --example editor_check -- "target/plugin/Guitar Studio.clap" [out.png] [seconds]
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use gs_engine::plugins::{load_plugin, scan_path_in_process};
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::path::Path;
    use truce_rack::core::{AudioBuffer, EventList, ProcessContext, WindowHandle};

    /// Run the main loop for `secs`, handling window events, like a DAW's event loop.
    fn pump(secs: f64) {
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

    let args: Vec<String> = std::env::args().collect();
    let bundle = Path::new(
        args.get(1)
            .context("usage: editor_check <bundle> [out.png] [seconds]")?,
    );
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "plugin-window.png".into());
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let format = if bundle.extension().is_some_and(|e| e == "vst3") {
        "vst3"
    } else {
        "clap"
    };
    let list = scan_path_in_process(format, bundle.parent().unwrap_or(Path::new(".")))?;
    let desc = list
        .iter()
        .find(|p| p.name == "Guitar Studio" && Path::new(&p.path) == bundle)
        .context("no Guitar Studio plugin there")?;

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    // A regular, active app, like a DAW: WebKit holds back drawing in windows it
    // thinks are hidden (a background accessory app's).
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    pump(0.2);

    let mut slot = load_plugin(desc, 48_000.0, 1024)?;
    let size = {
        let editor = slot.plugin.editor().context("no editor")?;
        editor.size().unwrap_or((1280, 800))
    };
    // SAFETY: plain window creation on the main thread.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(
                NSPoint::new(80.0, 80.0),
                NSSize::new(size.0 as f64, size.1 as f64),
            ),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: before anything else holds the window.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&objc2_foundation::NSString::from_str(
        "Guitar Studio (plugin window check)",
    ));
    // On top for the few seconds this runs: web content behind other windows isn't
    // always drawn, so the picture would come out blank.
    window.setLevel(objc2_app_kit::NSFloatingWindowLevel);
    window.orderFrontRegardless();
    let view = window.contentView().context("no content view")?;
    let parent = WindowHandle::NSView(objc2::rc::Retained::as_ptr(&view) as *mut std::ffi::c_void);
    slot.plugin
        .editor()
        .context("no editor")?
        .open(parent, window.backingScaleFactor())?;
    println!("editor open at {size:?}");

    // Run audio (silence) and the main loop, as a DAW would.
    let block = 256;
    let input = vec![vec![0.0f32; block]; slot.io.total_in];
    let mut output = vec![vec![0.0f32; block]; slot.io.total_out];
    let started = std::time::Instant::now();
    while started.elapsed().as_secs_f64() < seconds {
        let ins: Vec<&[f32]> = input.iter().map(|v| &v[..]).collect();
        let mut outs: Vec<&mut [f32]> = output.iter_mut().map(|v| &mut v[..]).collect();
        let (bi, bo) = (slot.io.bus_in.clone(), slot.io.bus_out.clone());
        let mut buf = AudioBuffer::new(&ins, &mut outs, block, &bi, &bo);
        let mut events = EventList::new();
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            max_block_size: 1024,
            transport: None,
            output_events: &mut events,
        };
        slot.plugin.process(&mut buf, &EventList::new(), &mut ctx)?;
        pump(0.005);
    }

    let number = window.windowNumber();
    let shot = std::process::Command::new("screencapture")
        .args(["-x", "-o", &format!("-l{number}"), &out])
        .status()?;
    println!("window picture: {out} ({shot})");

    if let Some(editor) = slot.plugin.editor() {
        editor.close();
    }
    window.close();
    drop(slot);
    pump(0.5);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
