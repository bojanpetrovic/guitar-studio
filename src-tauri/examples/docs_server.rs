//! The app layer behind a WebSocket, for taking the user manual's screenshots
//! (`make docs-screenshots`, driven by `scripts/docs-screenshots.mjs`).
//!
//! It speaks the phone remote's protocol (`{id, cmd, args}` → `{id, ok | err}`, plus
//! `{event, payload}` pushes) but takes every command, like the desktop window. No
//! audio device is opened: the engine runs host-driven, fed a synthetic guitar (a
//! plucked A2 every few seconds), so meters and the tuner show something real and
//! nothing can feed back through the computer's speakers.
//!
//! Usage: `cargo run --release -p guitar-studio --example docs_server -- <data dir> [port]`
//! The data dir is the app's folder (presets, plugins.json, favorites.json); use a
//! scratch copy, never the real one while the app runs.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use gs_app::{App, Reply, Request, Shell, main_thread};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

struct DocsShell(broadcast::Sender<String>);

impl Shell for DocsShell {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self
            .0
            .send(json!({ "event": event, "payload": payload }).to_string());
    }

    fn scanner(&self) -> std::io::Result<PathBuf> {
        Err(std::io::Error::other(
            "the docs server doesn't scan: give it a plugins.json",
        ))
    }
}

#[derive(Clone)]
struct Ctx {
    app: Arc<App>,
    events: broadcast::Sender<String>,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let data_dir = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("usage: docs_server <data dir> [port]"))?,
    );
    let port: u16 = args.next().map_or(Ok(7490), |p| p.parse())?;

    gs_app::logging::init(None);
    platform::init();
    main_thread::init();

    let (events, _) = broadcast::channel(256);
    let app = App::new(data_dir, Box::new(DocsShell(events.clone())), false)?;
    let hosted = app.studio.lock().start_hosted(RATE, 2, BLOCK as u32)?;
    std::thread::Builder::new()
        .name("docs-guitar".into())
        .spawn(move || play(hosted))?;
    gs_app::spawn_pump(&app, |_, _, _| {})?;

    let ctx = Ctx { app, events };
    std::thread::Builder::new()
        .name("docs-server".into())
        .spawn(move || serve(ctx, port))?;
    platform::run_main_loop()
}

const RATE: u32 = 48_000;
const BLOCK: usize = 256;

/// Drive the engine in real time with a plucked string (Karplus–Strong) at A2.
fn play(mut audio: gs_engine::audio::HostedAudio) {
    let period = (RATE as f32 / 110.0).round() as usize;
    let mut string = vec![0.0f32; period];
    let mut pos = 0;
    let mut seed = 0x2545_f491_u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let mut input = vec![0.0f32; BLOCK];
    let (mut left, mut right) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let block_time = Duration::from_secs_f64(BLOCK as f64 / f64::from(RATE));
    let mut next = Instant::now();
    let mut frame = 0usize;
    loop {
        for sample in &mut input {
            if frame.is_multiple_of(RATE as usize * 4) {
                string.iter_mut().for_each(|s| *s = noise() * 0.5);
            }
            let next_pos = (pos + 1) % period;
            let out = string[pos];
            string[pos] = 0.4985 * (string[pos] + string[next_pos]);
            pos = next_pos;
            *sample = out;
            frame += 1;
        }
        {
            let inputs = [input.as_slice(), input.as_slice()];
            let mut outputs = [left.as_mut_slice(), right.as_mut_slice()];
            audio.process(&inputs, &mut outputs, BLOCK, None);
        }
        next += block_time;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        } else {
            next = Instant::now();
        }
    }
}

fn serve(ctx: Ctx, port: u16) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    runtime.block_on(async move {
        let router = axum::Router::new()
            .route("/ws", get(open_socket))
            .with_state(ctx);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("the docs server's port");
        log::info!("docs server on ws://127.0.0.1:{port}/ws");
        axum::serve(listener, router).await.expect("serving");
    });
}

async fn open_socket(ws: WebSocketUpgrade, State(ctx): State<Ctx>) -> Response {
    ws.on_upgrade(move |socket| session(socket, ctx))
}

async fn session(mut socket: WebSocket, ctx: Ctx) {
    let mut events = ctx.events.subscribe();
    let (replies_tx, mut replies) = tokio::sync::mpsc::unbounded_channel::<String>();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(Message::Text(text))) = incoming else { break };
                let (app, tx) = (ctx.app.clone(), replies_tx.clone());
                tokio::task::spawn_blocking(move || {
                    let _ = tx.send(handle(&app, &text));
                });
            }
            Some(reply) = replies.recv() => {
                if socket.send(Message::Text(reply.into())).await.is_err() { break }
            }
            event = events.recv() => match event {
                Ok(frame) => if socket.send(Message::Text(frame.into())).await.is_err() { break },
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
        }
    }
}

fn handle(app: &Arc<App>, text: &str) -> String {
    let req: Request = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => return Reply::new(0, Err(e.to_string())).to_json(),
    };
    let id = req.id;
    let result = match req.cmd.as_str() {
        // The desktop window's own commands (src-tauri/src/lib.rs): stand-ins.
        "set_window_buttons" => Ok(Value::Null),
        "remote_status" | "set_remote" | "renew_remote_link" => Ok(remote_status()),
        cmd if gs_app::runs_off_main(cmd) => gs_app::dispatch(app, cmd, req.args),
        _ => {
            let app = app.clone();
            main_thread::run_and_wait(move || gs_app::dispatch(&app, &req.cmd, req.args))
                .unwrap_or_else(|| Err("the command panicked".into()))
        }
    };
    Reply::new(id, result).to_json()
}

/// What the phone menu shows once the remote is on (a made-up local address).
fn remote_status() -> Value {
    let url = "http://192.168.1.20:7480/#t=3f9c2a7e5b1d4c8a9e6f0b2d7a4c1e85";
    let qr = qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
        use qrcode::render::svg;
        code.render::<svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .build()
    });
    json!({ "enabled": true, "running": true, "url": url, "qrSvg": qr, "clients": 0, "error": null })
}

#[cfg(target_os = "macos")]
mod platform {
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        fn NSApplicationLoad() -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRunLoopRun();
    }

    /// Plugins expect Cocoa to be set up.
    pub fn init() {
        // SAFETY: called once, on the main thread, before anything else uses AppKit.
        unsafe { NSApplicationLoad() };
    }

    /// The main thread's run loop: plugins load and main-thread jobs run here.
    pub fn run_main_loop() -> anyhow::Result<()> {
        // SAFETY: on the main thread; runs until the process ends.
        unsafe { CFRunLoopRun() };
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn init() {}
    pub fn run_main_loop() -> anyhow::Result<()> {
        anyhow::bail!("the docs server runs on macOS only")
    }
}
