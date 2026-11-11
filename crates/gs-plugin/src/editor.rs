//! The plugin window: the app's own UI (the same React app as the desktop window) in a
//! webview inside the DAW's plugin window.
//!
//! The UI talks to us through `window.ipc.postMessage({id, cmd, args})`; replies and
//! events come back by calling `window.__gs` (see `src/transport.ts`). The page is
//! served from the bundle's `Contents/Resources/ui`, or from Vite when
//! `GS_PLUGIN_DEV_URL` is set (e.g. `http://localhost:1420` under `npm run dev`).

use crate::host::{Instance, Link, Ui};
use gs_app::{App, CmdResult, main_thread};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use truce::core::editor::{Editor, PluginContext, RawWindowHandle};

/// Default and smallest window size, in points.
const DEFAULT_SIZE: (u32, u32) = (1280, 800);
const MIN_SIZE: (u32, u32) = (900, 600);

/// Run a script in instance `id`'s webview, if its window is open. Main thread only.
pub fn run_script(id: usize, script: &str) {
    imp::run_script(id, script);
}

pub struct WebEditor {
    id: usize,
    app: Arc<App>,
    ui: Arc<Ui>,
    link: Arc<Link>,
}

impl WebEditor {
    pub fn new(instance: &Instance) -> Self {
        Self {
            id: instance.id,
            app: instance.app.clone(),
            ui: instance.ui.clone(),
            link: instance.link.clone(),
        }
    }
}

impl Editor for WebEditor {
    fn size(&self) -> (u32, u32) {
        self.ui.size.lock().unwrap_or(DEFAULT_SIZE)
    }

    fn open(&mut self, parent: RawWindowHandle, _context: PluginContext) {
        let RawWindowHandle::AppKit(view) = parent else {
            return log::error!("the plugin window only works on macOS so far");
        };
        let handler = Handler {
            id: self.id,
            app: self.app.clone(),
            link: self.link.clone(),
        };
        // wry panics on a parent it can't use; a panic must not reach the DAW.
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            imp::open(self.id, view, self.size(), handler)
        }));
        match opened {
            Ok(Ok(())) => self.ui.open.store(true, Ordering::Relaxed),
            Ok(Err(e)) => log::error!("the plugin window didn't open: {e}"),
            Err(_) => log::error!("the plugin window didn't open (webview panicked)"),
        }
    }

    fn close(&mut self) {
        self.ui.open.store(false, Ordering::Relaxed);
        imp::close(self.id);
    }

    fn can_resize(&self) -> bool {
        true
    }

    fn min_size(&self) -> (u32, u32) {
        MIN_SIZE
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        let size = (width.max(MIN_SIZE.0), height.max(MIN_SIZE.1));
        *self.ui.size.lock() = Some(size);
        imp::resize(self.id, size);
        true
    }
}

/// Carries the UI's requests to the app.
struct Handler {
    id: usize,
    app: Arc<App>,
    link: Arc<Link>,
}

#[derive(Deserialize)]
struct Request {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Value,
}

impl Handler {
    /// A message from the page (main thread).
    fn handle(&self, body: &str) {
        let req: Request = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => return log::warn!("bad message from the plugin window: {e}"),
        };
        if req.cmd == "__log" {
            return log::warn!("plugin window: {}", req.args["msg"].as_str().unwrap_or("?"));
        }
        log::debug!("plugin window → {}", req.cmd);
        let reply = Reply {
            view: self.id,
            request: req.id,
        };
        if gs_app::runs_off_main(&req.cmd) {
            let app = self.app.clone();
            let spawned = std::thread::Builder::new()
                .name("gs-command".into())
                .spawn(move || reply.send(gs_app::dispatch(&app, &req.cmd, req.args)));
            if let Err(e) = spawned {
                log::error!("couldn't run {}: {e}", req.id);
            }
            return;
        }
        let result = gs_app::dispatch(&self.app, &req.cmd, req.args);
        // The DAW saves the rig with its project: keep that copy current.
        if result.is_ok() && (gs_app::changes_state(&req.cmd) || req.cmd == "set_plugin_param") {
            self.link.dirty.store(true, Ordering::Relaxed);
        }
        reply.send(result);
    }
}

/// Where a request's answer goes.
struct Reply {
    view: usize,
    request: u64,
}

impl Reply {
    /// Always on a later main-loop turn: never from inside the webview's own callback.
    fn send(self, result: CmdResult<Value>) {
        let body = match result {
            Ok(ok) => serde_json::json!({ "id": self.request, "ok": ok }),
            Err(err) => serde_json::json!({ "id": self.request, "err": err }),
        };
        let script = format!("window.__gs&&window.__gs.reply({body})");
        let view = self.view;
        main_thread::run(move || run_script(view, &script));
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Handler;
    use crate::bundle;
    use raw_window_handle::{
        AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
    };
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::http::{Request, Response, StatusCode, header};
    use wry::{Rect, WebView, WebViewBuilder};

    /// Tells the UI it's in a plugin (see `src/transport.ts`), and sends the page's
    /// errors to the plugin log (a DAW gives us no web inspector).
    const INIT_SCRIPT: &str = r#"
window.__GS_PLUGIN__ = true;
(() => {
  const report = (msg) => window.ipc.postMessage(JSON.stringify({ id: 0, cmd: "__log", args: { msg: String(msg) } }));
  window.addEventListener("error", (e) => report(`${e.message} (${e.filename}:${e.lineno})`));
  window.addEventListener("unhandledrejection", (e) => report(`unhandled: ${e.reason}`));
  const error = console.error.bind(console);
  console.error = (...args) => (report(args.map(String).join(" ")), error(...args));
})();
"#;

    thread_local! {
        /// Open plugin windows by instance.
        static VIEWS: RefCell<HashMap<usize, WebView>> = RefCell::new(HashMap::new());
    }

    /// The DAW's view, as a parent wry understands.
    struct Parent(NonNull<c_void>);

    impl HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(self.0));
            // SAFETY: the DAW keeps its view alive while our editor is open.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    fn bounds((w, h): (u32, u32)) -> Rect {
        Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(f64::from(w), f64::from(h)).into(),
        }
    }

    pub fn open(
        id: usize,
        view: *mut c_void,
        size: (u32, u32),
        handler: Handler,
    ) -> Result<(), String> {
        close(id);
        let parent = Parent(NonNull::new(view).ok_or("the DAW gave no window")?);
        let url = std::env::var("GS_PLUGIN_DEV_URL").unwrap_or_else(|_| "gs://localhost/".into());
        let webview = WebViewBuilder::new()
            .with_url(url)
            // Tells the UI it's in a plugin (see `src/transport.ts`).
            .with_initialization_script(INIT_SCRIPT)
            .with_custom_protocol("gs".into(), |_id, request| asset(&request))
            .with_ipc_handler(move |request: Request<String>| handler.handle(request.body()))
            .with_bounds(bounds(size))
            .with_background_color((11, 12, 16, 255))
            .with_accept_first_mouse(true)
            .with_devtools(cfg!(debug_assertions))
            .build_as_child(&parent)
            .map_err(|e| e.to_string())?;
        log::debug!("plugin window {id} open: {:?}", webview.bounds());
        VIEWS.with_borrow_mut(|views| views.insert(id, webview));
        Ok(())
    }

    pub fn close(id: usize) {
        // Dropped outside the borrow: the webview may run the main loop as it goes.
        let gone = VIEWS.with_borrow_mut(|views| views.remove(&id));
        if gone.is_some() {
            log::debug!("plugin window {id} closed");
        }
        drop(gone);
    }

    pub fn resize(id: usize, size: (u32, u32)) {
        log::debug!("plugin window {id} resized to {size:?}");
        VIEWS.with_borrow(|views| {
            if let Some(view) = views.get(&id) {
                let _ = view.set_bounds(bounds(size));
            }
        });
    }

    pub fn run_script(id: usize, script: &str) {
        VIEWS.with_borrow(|views| match views.get(&id) {
            Some(view) => {
                if let Err(e) = view.evaluate_script(script) {
                    log::warn!("plugin window script failed: {e}");
                }
            }
            None => log::debug!("plugin window {id} is closed"),
        });
    }

    /// A file of the built UI. Unknown paths get the app (it's a single page).
    fn asset(request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let Some(root) = bundle::ui_dir() else {
            return status(StatusCode::NOT_FOUND);
        };
        let path = request.uri().path().trim_start_matches('/');
        // Only files under the UI folder.
        if path.split('/').any(|part| part == "..") {
            return status(StatusCode::FORBIDDEN);
        }
        let path = if path.is_empty() { "index.html" } else { path };
        let file = root.join(path);
        let file = if file.is_file() {
            file
        } else {
            root.join("index.html")
        };
        match std::fs::read(&file) {
            Ok(bytes) => Response::builder()
                .header(header::CONTENT_TYPE, mime(&file))
                .body(Cow::Owned(bytes))
                .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR)),
            Err(_) => status(StatusCode::NOT_FOUND),
        }
    }

    fn status(code: StatusCode) -> Response<Cow<'static, [u8]>> {
        let mut response = Response::new(Cow::Borrowed(&[][..]));
        *response.status_mut() = code;
        response
    }

    fn mime(path: &std::path::Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "ico" => "image/x-icon",
            "woff2" => "font/woff2",
            "woff" => "font/woff",
            "ttf" => "font/ttf",
            _ => "application/octet-stream",
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Handler;

    pub fn open(
        _: usize,
        _: *mut std::ffi::c_void,
        _: (u32, u32),
        _: Handler,
    ) -> Result<(), String> {
        Err("the plugin window only works on macOS so far".into())
    }
    pub fn close(_: usize) {}
    pub fn resize(_: usize, _: (u32, u32)) {}
    pub fn run_script(_: usize, _: &str) {}
}
