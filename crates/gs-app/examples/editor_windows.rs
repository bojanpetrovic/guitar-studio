//! Plugin editor windows, outside the app: add Apple's AUDelay, open its editor, close
//! the window as a user would (the watcher must notice), reopen it, and remove the node
//! (which must close it).
//!
//! cargo run -p gs-app --example editor_windows
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use gs_app::{App, Shell, dispatch, editor};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};
    use serde_json::{Value, json};

    struct Quiet;
    impl Shell for Quiet {
        fn emit(&self, _: &str, _: Value) {}
        fn scanner(&self) -> std::io::Result<std::path::PathBuf> {
            Err(std::io::Error::other("no scans here"))
        }
    }

    let mtm = MainThreadMarker::new().expect("main thread");
    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    ns_app.finishLaunching();
    let pump = |secs: f64| {
        let until = NSDate::dateWithTimeIntervalSinceNow(secs);
        // SAFETY: AppKit event loop on the main thread.
        unsafe {
            while let Some(e) = ns_app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&until),
                NSDefaultRunLoopMode,
                true,
            ) {
                ns_app.sendEvent(&e);
            }
        }
    };

    let dir = std::env::temp_dir().join("gs-editor-windows");
    let app = App::new(dir, Box::new(Quiet), false)?;
    // AUDelay ships with macOS: no licence checks, no prompts.
    let catalog: gs_app::Catalog = serde_json::from_slice(&std::fs::read(
        gs_app::default_data_dir().join("plugins.json"),
    )?)?;
    let delay = catalog
        .plugins
        .into_iter()
        .find(|p| p.name == "AUDelay")
        .context("AUDelay isn't in the plugin list (scan first)")?;
    app.catalog.lock().plugins = vec![delay.clone()];
    app.studio.lock().start_audio(Default::default()).ok();

    let (id, _): (u32, Value) = serde_json::from_value(
        dispatch(
            &app,
            "add_plugin_node",
            json!({"key": delay.key(), "x": 0, "y": 0}),
        )
        .map_err(anyhow::Error::msg)?,
    )?;
    println!("added AUDelay as node {id}");

    dispatch(&app, "open_editor", json!({ "id": id })).map_err(anyhow::Error::msg)?;
    pump(1.0);
    if !editor::any_open(&app) {
        bail!("the editor didn't open");
    }
    // A second open just brings it forward.
    dispatch(&app, "open_editor", json!({ "id": id })).map_err(anyhow::Error::msg)?;
    let windows = ns_app.windows();
    println!("editor open ({} window(s))", windows.count());

    // Close it like the user would: the watcher closes the plugin's view within 250 ms.
    for w in windows.iter() {
        w.performClose(None);
    }
    pump(0.8);
    if editor::any_open(&app) {
        bail!("closing the window didn't close the editor");
    }
    println!("closed by the window's close button: ok");

    dispatch(&app, "open_editor", json!({ "id": id })).map_err(anyhow::Error::msg)?;
    pump(0.5);
    dispatch(&app, "remove_node", json!({ "id": id })).map_err(anyhow::Error::msg)?;
    pump(0.5);
    if editor::any_open(&app) {
        bail!("removing the node left its editor open");
    }
    println!("closed by removing the node: ok");
    app.shut_down();
    pump(0.3);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
