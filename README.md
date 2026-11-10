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

## Presets

A preset saves the whole rig: nodes, cables, levels, bypass states, the input
channel, the tempo, and every plugin's own state. The plugin state is the same
blob a DAW stores, so settings you dial in inside a plugin's editor come back
exactly.

- **⌘S** saves to the current preset (or asks for a name). **⇧⌘S** saves as a new one.
- The preset menu next to the logo loads, renames and deletes presets, and starts a new empty rig.
- On launch, the app reloads the preset you last had open.
- Presets are plain JSON files in `~/Library/Application Support/dev.guitarstudio.app/presets/`.
  You can back them up or copy them between machines.
- If a plugin in a preset isn't installed, the rest of the rig still loads, and a
  warning names the missing plugin.

## Layout

| Path | What |
|---|---|
| `crates/gs-engine` | Audio engine: cpal I/O, graph compiler, RT executor, plugin hosting (truce-rack), tuner (McLeod pitch method), metronome |
| `src-tauri` | Tauri shell: commands, plugin-editor windows, telemetry pump (30 Hz) |
| `src` | React UI: React Flow patch canvas, plugin browser, tuner, metronome, settings |
| `docs/RESEARCH.md` | Tech research and roadmap |
| `vendor/truce-rack-vst3` | Patched copy of the VST3 host crate (see `vendor/README.md`) |

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
make smoke FORMAT=au PLUGIN=nolly       # load, process and state-round-trip a plugin headless
make preset-test FORMAT=au PLUGIN=nolly # save and reload a rig with a real plugin
make scan                               # rescan plugins into the app's cache
make reset-settings                     # forget the saved audio device and plugin cache
```

## Not done yet

- VST3: forwarding edits made in a plugin's editor to its audio processor (`IComponentHandler`).
  This affects plugins built with separate component and controller parts. JUCE plugins such as
  Neural DSP's don't need it. Prefer the AU version on macOS until it's fixed.
- An unsaved-changes indicator
- Plugin delay compensation on parallel branches
- MIDI foot-controller mapping
- Out-of-process plugin sandboxing (scanning is already isolated)
- Code signing and notarization. `Entitlements.plist` already has the required keys.
