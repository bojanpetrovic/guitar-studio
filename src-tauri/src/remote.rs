//! Phone remote: a small web server on the local network. It serves the app's own UI
//! (the phone opens just the pedalboard in its browser) and a WebSocket that carries the
//! phone's commands to the engine and pushes meters, the tuner and rig changes back.
//!
//! Wire format (JSON text frames):
//! - phone → app: `{"id": 7, "cmd": "set_bypass", "args": {"id": 3, "bypass": true}}`
//! - app → phone: `{"id": 7, "ok": …}` / `{"id": 7, "err": "…"}`, and pushes
//!   `{"event": "telemetry" | "sync" | "denied", "payload": …}`.
//!
//! Only playing commands are accepted (no rig or board editing), and the socket needs the
//! secret token from the QR code.

use crate::Desktop;
use axum::Router;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State as AxState};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use gs_app::{Reply, Request, changes_state, err};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, TcpListener, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch};

/// Default port; if it's taken we fall back to any free one.
const DEFAULT_PORT: u16 = 7480;
/// Largest message a phone may send (its commands are tiny).
const MAX_MESSAGE: usize = 64 * 1024;
const PING_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
/// A session that hasn't been heard from for this long is closed.
const SILENCE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    enabled: bool,
    token: String,
    port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            token: new_token(),
            port: DEFAULT_PORT,
        }
    }
}

struct Running {
    port: u16,
    stop: watch::Sender<bool>,
    /// Tells a server that failed apart from one started after it.
    generation: u64,
}

/// What the desktop's phone dialog shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub enabled: bool,
    pub running: bool,
    /// The link the QR code encodes.
    pub url: Option<String>,
    pub qr_svg: Option<String>,
    pub clients: usize,
    pub error: Option<String>,
}

/// Shared between the server, the telemetry pump and the desktop's command handler.
#[derive(Default)]
pub struct Hub {
    path: Mutex<Option<PathBuf>>,
    settings: Mutex<Settings>,
    server: Mutex<Option<Running>>,
    error: Mutex<Option<String>>,
    clients: AtomicUsize,
    generations: AtomicU64,
    /// Bumped when the link is renewed: sessions on the old token end.
    kick: watch::Sender<u64>,
    /// Latest telemetry / rig state, as ready-to-send event frames (cheap to clone).
    telemetry: watch::Sender<Option<Utf8Bytes>>,
    sync: watch::Sender<Option<Utf8Bytes>>,
    /// Rig state changed through the desktop UI → tell the phones.
    phones_stale: AtomicBool,
    /// … through a phone → tell the desktop (and the phones).
    desktop_stale: AtomicBool,
}

impl Hub {
    /// Read the saved settings and start serving if the remote was left on.
    pub fn init(self: &Arc<Self>, app: &AppHandle, data_dir: PathBuf) {
        let path = data_dir.join("remote.json");
        let settings = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .filter(|s| s.token.len() >= 16)
            .unwrap_or_default();
        *self.path.lock() = Some(path);
        let enabled = settings.enabled;
        *self.settings.lock() = settings;
        self.save();
        if enabled && let Err(e) = self.start(app) {
            log::warn!("phone remote failed to start: {e}");
        }
    }

    fn save(&self) {
        if let (Some(path), Ok(json)) = (
            self.path.lock().clone(),
            serde_json::to_vec_pretty(&*self.settings.lock()),
        ) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn has_clients(&self) -> bool {
        self.clients.load(Ordering::Relaxed) > 0
    }

    /// A desktop command changed the rig / board / scenes / transport.
    pub fn desktop_changed(&self) {
        self.phones_stale.store(true, Ordering::Relaxed);
    }

    fn phone_changed(&self) {
        self.phones_stale.store(true, Ordering::Relaxed);
        self.desktop_stale.store(true, Ordering::Relaxed);
    }

    /// Called by the telemetry pump: which sides need a fresh rig state?
    pub fn take_stale(&self) -> (bool, bool) {
        let phones = self.phones_stale.swap(false, Ordering::Relaxed) && self.has_clients();
        let desktop = self.desktop_stale.swap(false, Ordering::Relaxed);
        (phones, desktop)
    }

    pub fn push_telemetry<T: Serialize>(&self, t: &T) {
        if self.has_clients() {
            self.telemetry.send_replace(event_frame("telemetry", t));
        }
    }

    pub fn push_sync<T: Serialize>(&self, s: &T) {
        self.sync.send_replace(event_frame("sync", s));
    }

    pub fn set_enabled(self: &Arc<Self>, app: &AppHandle, enabled: bool) -> Result<(), String> {
        self.settings.lock().enabled = enabled;
        self.save();
        if enabled {
            self.start(app)
        } else {
            self.stop();
            Ok(())
        }
    }

    /// New secret: old links (and phones using them) stop working. The server keeps
    /// running (and its port), only the sessions on the old token are ended.
    pub fn renew_token(&self) {
        self.settings.lock().token = new_token();
        self.save();
        self.kick.send_modify(|k| *k += 1);
    }

    fn start(self: &Arc<Self>, app: &AppHandle) -> Result<(), String> {
        if self.server.lock().is_some() {
            return Ok(());
        }
        let wanted = self.settings.lock().port;
        let listener = TcpListener::bind(("0.0.0.0", wanted))
            .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
            .and_then(|l| l.set_nonblocking(true).map(|_| l));
        let listener = match listener {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Couldn't open a network port: {e}");
                *self.error.lock() = Some(msg.clone());
                return Err(msg);
            }
        };
        let port = listener.local_addr().map_err(err)?.port();
        let (stop, mut stopped) = watch::channel(false);
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        *self.server.lock() = Some(Running {
            port,
            stop,
            generation,
        });
        *self.error.lock() = None;

        let ctx = Ctx {
            app: app.clone(),
            hub: self.clone(),
            stop: stopped.clone(),
        };
        let router = Router::new()
            .route("/ws", get(open_socket))
            .fallback(get(asset))
            .with_state(ctx);
        let hub = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let served = async {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        let _ = stopped.wait_for(|s| *s).await;
                    })
                    .await
            };
            if let Err(e) = served.await {
                log::error!("phone remote server stopped: {e}");
                *hub.error.lock() = Some(format!("The phone server stopped: {e}"));
                let mut server = hub.server.lock();
                if server.as_ref().is_some_and(|r| r.generation == generation) {
                    server.take();
                }
                drop(server);
                let _ = app.emit("remote", hub.status(&app));
            }
        });
        log::info!("phone remote listening on port {port}");
        Ok(())
    }

    fn stop(&self) {
        if let Some(running) = self.server.lock().take() {
            let _ = running.stop.send(true);
        }
    }

    fn token_ok(&self, token: Option<&String>) -> bool {
        let want = self.settings.lock().token.clone();
        token.is_some_and(|t| {
            // Compare without an early exit on the first differing byte.
            t.len() == want.len()
                && t.bytes()
                    .zip(want.bytes())
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
    }

    pub fn status(&self, app: &AppHandle) -> RemoteStatus {
        let settings = self.settings.lock().clone();
        let port = self.server.lock().as_ref().map(|r| r.port);
        let ip = lan_ip();
        let url = port.map(|port| {
            let host = ip.map_or_else(|| "localhost".to_string(), |ip| ip.to_string());
            let token = &settings.token;
            match dev_server_port(app) {
                // `tauri dev`: the UI comes from Vite; the page finds us through `ws`.
                Some(vite) => format!("http://{host}:{vite}/#t={token}&ws={host}:{port}"),
                None => format!("http://{host}:{port}/#t={token}"),
            }
        });
        let mut error = self.error.lock().clone();
        if port.is_some() && ip.is_none() {
            error = Some("This computer isn't on a network — connect to Wi-Fi first.".into());
        }
        RemoteStatus {
            enabled: settings.enabled,
            running: port.is_some(),
            qr_svg: url.as_deref().and_then(qr_svg),
            url,
            clients: self.clients.load(Ordering::Relaxed),
            error,
        }
    }
}

// ---------------------------------------------------------------- http

#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    hub: Arc<Hub>,
    /// This server's stop signal (not a later server's).
    stop: watch::Receiver<bool>,
}

/// The app's own (embedded) frontend. In `tauri dev` the UI lives on the Vite server.
async fn asset(AxState(ctx): AxState<Ctx>, uri: Uri) -> Response {
    if let Some(vite) = dev_server_port(&ctx.app) {
        let host = lan_ip().map_or_else(|| "localhost".to_string(), |ip| ip.to_string());
        return Redirect::temporary(&format!("http://{host}:{vite}/")).into_response();
    }
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let resolver = ctx.app.asset_resolver();
    // Unknown paths get the app (it's a single page).
    match resolver
        .get(path.to_string())
        .or_else(|| resolver.get("index.html".into()))
    {
        Some(a) => (
            [
                (header::CONTENT_TYPE, a.mime_type),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            a.bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn open_socket(
    AxState(ctx): AxState<Ctx>,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    // Accept even with a bad token so the phone can say why (a browser can't read the
    // status of a refused WebSocket).
    let allowed = ctx.hub.token_ok(query.get("t"));
    // Phones only send small commands.
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| session(socket, ctx, allowed))
}

// ---------------------------------------------------------------- websocket

async fn session(socket: WebSocket, ctx: Ctx, allowed: bool) {
    let (mut tx, mut rx) = socket.split();
    if !allowed {
        if let Some(frame) = event_frame("denied", &()) {
            let _ = tx.send(Message::Text(frame)).await;
        }
        let _ = tx.close().await;
        return;
    }
    let mut stop = ctx.stop.clone();
    let mut kick = ctx.hub.kick.subscribe();
    let mut telemetry = ctx.hub.telemetry.subscribe();
    let mut sync = ctx.hub.sync.subscribe();
    telemetry.mark_unchanged();
    sync.mark_unchanged();
    ctx.hub.clients.fetch_add(1, Ordering::Relaxed);
    let _ = ctx.app.emit("remote", ctx.hub.status(&ctx.app));

    // A phone that sleeps or leaves the Wi-Fi often just goes quiet: ping it, and give
    // up when nothing (not even a pong) comes back for a while.
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut heard = tokio::time::Instant::now();

    loop {
        // Requests are answered in order, one at a time: a `set` is always applied
        // before the `get` that follows it.
        let message = tokio::select! {
            msg = rx.next() => {
                heard = tokio::time::Instant::now();
                match msg {
                    Some(Ok(Message::Text(text))) => Message::Text(handle(&ctx, text.as_str()).await.into()),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue, // pong, ping (answered by axum), binary
                }
            }
            changed = telemetry.changed() => {
                if changed.is_err() { break; }
                match telemetry.borrow_and_update().clone() { Some(f) => Message::Text(f), None => continue }
            }
            changed = sync.changed() => {
                if changed.is_err() { break; }
                match sync.borrow_and_update().clone() { Some(f) => Message::Text(f), None => continue }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE_LIMIT { break; }
                Message::Ping(Default::default())
            }
            _ = stop.changed() => break,
            _ = kick.changed() => break,
        };
        if tx.send(message).await.is_err() {
            break;
        }
    }

    let _ = tx.close().await;
    ctx.hub.clients.fetch_sub(1, Ordering::Relaxed);
    let _ = ctx.app.emit("remote", ctx.hub.status(&ctx.app));
}

async fn handle(ctx: &Ctx, text: &str) -> String {
    let req: Request = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => return Reply::new(0, Err(e.to_string())).to_json(),
    };
    // Plugins expect to be driven from the main thread, like the desktop's commands.
    let (done, result) = oneshot::channel();
    let app = ctx.app.clone();
    let (cmd, args) = (req.cmd.clone(), req.args);
    let result = match ctx.app.run_on_main_thread(move || {
        let _ = done.send(dispatch(&app, &cmd, args));
    }) {
        Ok(()) => result
            .await
            .unwrap_or_else(|_| Err("Guitar Studio is closing".into())),
        Err(e) => Err(e.to_string()),
    };
    if result.is_ok() && changes_state(&req.cmd) {
        ctx.hub.phone_changed();
    }
    Reply::new(req.id, result).to_json()
}

/// The commands a phone may run: everything needed to play the board, nothing that
/// edits the rig or the board itself.
const PHONE_COMMANDS: &[&str] = &[
    "remote_sync",
    "get_studio",
    "list_presets",
    "preset_ready",
    "load_preset",
    "plugin_params",
    "get_param_values",
    "set_plugin_param",
    "set_bypass",
    "set_node_param",
    "set_mixer_strip",
    "set_metronome",
    "set_tuner",
    "select_scene",
    "add_scene",
    "rename_scene",
    "delete_scene",
];

fn dispatch(app: &AppHandle, cmd: &str, args: Value) -> Result<Value, String> {
    if !PHONE_COMMANDS.contains(&cmd) {
        return Err(format!("“{cmd}” isn't available from the phone"));
    }
    gs_app::dispatch(&app.state::<Desktop>().app, cmd, args)
}

// ---------------------------------------------------------------- helpers

#[derive(Serialize)]
struct Event<'a, T> {
    event: &'a str,
    payload: &'a T,
}

fn event_frame<T: Serialize>(event: &str, payload: &T) -> Option<Utf8Bytes> {
    serde_json::to_string(&Event { event, payload })
        .ok()
        .map(Into::into)
}

/// This machine's address on the local network: the interface the default route uses.
/// (Connecting a UDP socket only picks a route; nothing is sent.)
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Vite's port when running under `tauri dev`.
fn dev_server_port(app: &AppHandle) -> Option<u16> {
    if !tauri::is_dev() {
        return None;
    }
    app.config().build.dev_url.as_ref()?.port_or_known_default()
}

fn qr_svg(text: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

/// 128 random bits, hex.
fn new_token() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let random = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if random.is_err() {
        // No /dev/urandom (Windows): std's per-process random hash keys.
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn token_check() {
        let hub = Hub::default();
        let good = hub.settings.lock().token.clone();
        assert!(hub.token_ok(Some(&good)));
        assert!(!hub.token_ok(Some(&"x".repeat(32))));
        assert!(!hub.token_ok(None));
    }
}
