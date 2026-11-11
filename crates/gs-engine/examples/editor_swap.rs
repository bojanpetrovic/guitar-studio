//! Replays "replace a plugin, then open the new one's editor" outside the app, with
//! plain AppKit windows, to see which step upsets the Objective-C runtime.
//!
//! cargo run -p gs-engine --example editor_swap -- <format> "<old plugin>" "<new plugin>"
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use gs_engine::nodes::MAX_BLOCK;
    use gs_engine::plugins::{load_plugin, scan_format_in_process};
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use truce_rack::core::WindowHandle;

    fn pump(secs: f64) {
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: *const std::ffi::c_void;
            fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, s: f64, r: u8) -> i32;
        }
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
    }

    let args: Vec<String> = std::env::args().collect();
    let (format, old_name, new_name) = (
        args[1].as_str(),
        args[2].to_lowercase(),
        args[3].to_lowercase(),
    );
    let catalog = scan_format_in_process(format)?;
    let find = |n: &str| {
        catalog
            .iter()
            .find(|p| p.name.to_lowercase().contains(n))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no {n}"))
    };
    let (a, b) = (find(&old_name)?, find(&new_name)?);

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    pump(0.2);
    let window = |title: &str| {
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(640.0, 420.0)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { w.setReleasedWhenClosed(false) };
        w.setTitle(&objc2_foundation::NSString::from_str(title));
        w.makeKeyAndOrderFront(None);
        w
    };
    let open = |slot: &mut gs_engine::plugins::PluginSlot, w: &NSWindow| -> anyhow::Result<()> {
        let view = w.contentView().unwrap();
        let ptr = objc2::rc::Retained::as_ptr(&view) as *mut std::ffi::c_void;
        let ed = slot
            .plugin
            .editor()
            .ok_or_else(|| anyhow::anyhow!("no editor"))?;
        ed.open(WindowHandle::NSView(ptr), 2.0)?;
        println!("  editor open: {:?}", ed.size());
        Ok(())
    };
    let mode = args.get(4).map(String::as_str).unwrap_or("opened-before");

    println!(
        "1. load {} and {}",
        a.name,
        if mode == "opened-before" {
            "open its editor"
        } else {
            "leave its editor shut"
        }
    );
    let mut old = load_plugin(&a, 48_000.0, MAX_BLOCK)?;
    let w1 = window(&a.name);
    if mode == "opened-before" {
        open(&mut old, &w1)?;
        pump(1.5);
        old.plugin.editor().unwrap().close();
    }
    w1.close();
    pump(0.3);

    println!("2. load {} (the replacement)", b.name);
    let mut new = load_plugin(&b, 48_000.0, MAX_BLOCK)?;
    pump(0.3);
    let keep = std::env::var("KEEP").is_ok();
    println!("3. {} {}", if keep { "keep" } else { "dispose" }, a.name);
    let kept = if keep {
        Some(old)
    } else {
        drop(old);
        None
    };
    pump(0.5);
    println!("4. open {}'s editor", b.name);
    let w2 = window(&b.name);
    open(&mut new, &w2)?;
    for _ in 0..4 {
        pump(0.5);
        let sub = w2.contentView().unwrap().subviews();
        let frame = sub.firstObject().map(|v| v.frame());
        println!(
            "  later: extent {:?}  view frame {:?}",
            new.plugin.editor().unwrap().size(),
            frame.map(|f| (f.size.width, f.size.height))
        );
    }
    new.plugin.editor().unwrap().close();
    w2.close();
    pump(0.3);
    drop(kept);
    println!("done");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
