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

## Mixer

The built-in **Mixer** node gives every cable coming into it its own channel strip:
volume, pan and mute, with a post-fader meter. A **Mix level** fader sits after the
mix. Use it to blend parallel paths, e.g. two amps, or a dry amp and a wet delay.

- Pan is balance-style: a centered strip passes stereo sources at unity, and turning
  one way fades the other side out.
- Level and pan changes ramp over one block, so moving a fader never clicks.
- Strip settings belong to the cable's source. Disconnecting and reconnecting keeps
  them, and they're saved in presets.
- Pedalboard knobs, faders and footswitches can drive any strip's volume, pan or
  mute. "Build from rig" adds a fader pedal for each mixer.

## Pedalboard

Every preset can have its own pedalboard: a control surface you lay out yourself and
wire to the rig. Switch views with **⌘1** (Rig) and **⌘2** (Pedalboard).

- **Edit mode** has a palette of footswitches, knobs, faders, selectors, meters,
  labels, pedal enclosures and a tuner display. Drag to move, pull a corner to
  resize; everything snaps to the grid. ⌫ deletes, ⌘D duplicates, arrow keys nudge.
- **Wiring:** select a control and choose what it drives in the inspector. Options are
  any plugin parameter (searchable), a node's bypass or level, a mixer strip's
  volume, pan or mute, metronome on/off, tap tempo, tempo, or the tuner. Meters can show any node's output or the main out.
- **Build from rig** creates one pedal per plugin in signal order. Each gets an on/off
  footswitch, its two main knobs and an amp/channel selector where the plugin has
  one. The board also gets tap tempo, a tuner and an output meter.
- **Play mode:** controls change the sound right away, and knobs follow changes you
  make inside the plugin's own window.
- **Stage:** playing a board shows only the board, with the tuner/metronome dock
  floating underneath. The window loses its frame and traffic lights, goes
  see-through and takes the board's shape; drag the board to move it. Move the
  mouse to bring up a small bar with the preset name (← / →, Page Up / Page Down
  or [ / ] switch presets), **Edit** and **Rig**. ⌘E, Esc or ⌘1 also leave the stage, and
  the window goes back to its old size and place.

## Scenes

A scene remembers every plugin's settings and on/off state, the Gain and Mixer
levels, and each mixer input's volume, pan and mute, so one rig can hold a clean, a
crunch and a lead sound. Press **1–9** (or click a scene) to switch; the
scene bar sits at the top of the rig and under the pedalboard (Board → *Show scenes
under the board* hides it there).

- **+** adds a scene from the current sound (up to 9). Then shape the new scene.
- Changes go into the active scene: switching away keeps what you did.
- Double-click a scene to rename or delete it.
- Scenes are saved with the preset. A plugin or cable added later keeps its settings
  in older scenes until you change it there.
- Switching only sets the parameters that differ, a parameter at a time, so the audio
  keeps running (Archetype Nolly: ~11 ms; Pro-Q 3 / Timeless 3: ~1 ms).

## Tuner & metronome

A slim dock under the canvas, always there (on stage too): note, a cents strip and
frequency on the left, with the reference pitch; on the right play/stop, beat dots,
tempo, tap, time signature, subdivision and click volume. The tuner is on from launch.

| Key | |
| --- | --- |
| Space | start / stop the metronome |
| ↑ / ↓ | tempo ±1 BPM (⇧ ±5) |
| ← / → | previous / next preset (on stage) |
| 1 – 9 | switch scene |

Keys are ignored while typing in a field; in the pedalboard editor the arrows move
the selected control instead.

## Themes

Click the palette button in the title bar (or press **⇧⌘T** to cycle). Six themes:
**Amber** (default), **Midnight**, **Neon** and **Graphite** are dark; **Daylight** and **Paper**
are light. *Match system appearance* follows macOS (Amber when dark, Daylight when light).
The choice is an app setting, not part of a preset. Pedalboard hardware (board surfaces,
knobs, footswitches) keeps its physical look in every theme.

To add a theme, add a `:root[data-theme="…"]` palette block at the top of `src/styles.css`
and an entry in `src/themes.ts`; everything else is derived from those tokens.

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
make param-test FORMAT=au PLUGIN=nolly  # set and read back a plugin's parameters through the engine
make scene-test FORMAT=au PLUGIN=nolly  # switch scenes on a real plugin, save and reload them
make params FORMAT=au PLUGIN=nolly      # list a plugin's parameters (ids for pedalboard bindings)
make scan                               # rescan plugins into the app's cache
make reset-settings                     # forget the saved audio device and plugin cache
```

## Not done yet

- MIDI foot-controller mapping for pedalboard controls (MIDI learn)
- An unsaved-changes indicator
- Plugin delay compensation on parallel branches
- MIDI foot-controller mapping
- Out-of-process plugin sandboxing (scanning is already isolated)
- Code signing and notarization. `Entitlements.plist` already has the required keys.
