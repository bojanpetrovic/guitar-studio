# Guitar Studio

A desktop guitar studio: host VST3 / AU / CLAP plugins, patch them together freely,
with a built-in tuner and metronome. Rust audio engine, Tauri v2 + React UI.

## Run

```sh
make install   # npm packages + Rust crates
make dev       # run with hot reload
make bundle    # release .app / .dmg
make help      # list every task
```

The first launch has no plugin list. Click the refresh icon in the Plugins panel to scan.
Each format is scanned in its own child process, so a crashing plugin can't take the app
down. A full scan takes about a minute and the results are cached.

## Layout

| Path | What |
|---|---|
| `crates/gs-engine` | Audio engine: cpal I/O, graph compiler, RT executor, plugin hosting (truce-rack), tuner (McLeod pitch method), metronome |
| `src-tauri` | Tauri shell: commands, plugin-editor windows, telemetry pump (30 Hz) |
| `src` | React UI: React Flow patch canvas, plugin browser, tuner, metronome, settings |
| `docs/RESEARCH.md` | Tech research and roadmap |

## How the engine works

- **Graph edits** happen on the control thread. Each edit compiles a new, fully
  pre-allocated `Schedule` and sends it to the audio thread over a lock-free queue. Old
  schedules come back through a second queue and are freed off the audio thread.
- **Routing** works through cables. Every node has one stereo in and one stereo out.
  Fan-out is free, fan-in is summed, and cycles are rejected.
- **Plugins** sit behind a mutex. The audio thread only `try_lock`s it, and passes the
  dry signal through on contention, so it never blocks.
- **Plugin editors** open in native windows; their `NSView` becomes the plugin's parent.

## Useful dev commands

```sh
make check                              # fmt-check + clippy + tsc + tests (what CI should run)
make smoke FORMAT=au PLUGIN=nolly       # load and process a plugin headless
make scan                               # rescan plugins into the app's cache
make reset-settings                     # forget the saved audio device and plugin cache
```

## Not done yet

- Saving and loading patches and presets, including plugin state blobs
- Plugin delay compensation on parallel branches
- MIDI foot-controller mapping
- Out-of-process plugin sandboxing (scanning is already isolated)
- Code signing and notarization. `Entitlements.plist` already has the required keys.
