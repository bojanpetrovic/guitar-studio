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

The plugin list sits on the right; show or hide it with the button at the right end of
the title bar (or ⌘B). Star a plugin (☆ on hover) to keep it in **Favourites** at the
top of the list. Favourites
also come first in "Replace with…", and the desktop app and the DAW plugin share them.

## Replacing a plugin

Swap a plugin for another without rewiring: click ⇄ on its node and pick one, with
search and arrow keys; plugins from the same vendor are listed first. Or drag a plugin
from the list and drop it onto the node. The node keeps its cables, place and on/off
state, so footswitches still work. The new plugin starts at its own defaults, and
scenes forget the old one's settings. Pedalboard knobs move to the new plugin's
parameter with the same name; any without a match are reported for rebinding.

## Presets

A preset saves the whole rig: nodes, cables, levels, bypass states, the input
channel, the tempo, and every plugin's own state. The plugin state is the same
blob a DAW stores, so settings you dial in inside a plugin's editor come back
exactly.

- **⌘S** saves to the current preset (or asks for a name). **⇧⌘S** saves as a new one.
- The **Presets panel** on the left lists every preset: click one to load it, search by
  name or plugin, rename or delete on hover, *Save as…* and *New rig* at the top. Show
  or hide it with the button next to the logo, the preset name in the title bar, or ⌘P.
- On launch, the app reloads the preset you last had open.
- Presets are plain JSON files in `~/Library/Application Support/dev.guitarstudio.app/presets/`.
  You can back them up or copy them between machines.
- If a plugin in a preset isn't installed, the rest of the rig still loads, and a
  warning names the missing plugin.
- **Switching is fast.** Starting a plugin can take seconds (Archetype Nolly: ~2.3 s),
  but restoring settings into one that's already running takes milliseconds. So a switch
  reuses running instances of the same plugins: Mixer Demo ↔ Pedalboard Demo takes ~2 ms
  instead of ~2.2 s. Spare instances that the previous or next preset in the list
  needs are kept loaded. Missing ones are preloaded in the background a moment after a
  switch, one at a time, with a *Preloading …* chip in the title bar. AU plugins start
  on a background thread, so the app stays responsive even while a slow one loads
  (Archetype Cory Wong: ~12 s); VST3 / CLAP plugins have to start on the main thread,
  so the window may pause while one of those preloads (audio keeps playing).
- A reused delay or reverb carries its tail into the next preset (spillover).

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
- **Keyboard keys:** select a footswitch, knob, fader or selector and use
  *Keyboard key* to give it a key. In play mode a switch's key stomps it (momentary
  switches are held while the key is down). A knob's, fader's or selector's key
  selects it (it glows): then ↑ / ↓ turn it (⇧ for bigger steps) — no need to hold
  anything. Press the key again, another control's key, or Esc to let go; while a
  control is selected, ↑ / ↓ don't change the tempo. Keys the app already uses (Space, arrows, 1–9, Esc, …)
  can't be assigned, and a key on several controls moves them together.
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

## Muting the output

The speaker button next to the OUT meter (also on the Main Out node and in the phone's
header) mutes the main output, fading out in a few milliseconds so it doesn't click.
Mute is a live control, not part of a sound: loading a preset or scene leaves it as
it is. A pedalboard footswitch can do it too: bind it to *On/off* on Main Out (lit
while the sound is on).

## Phone remote

Play the pedalboard from your phone. Click the phone icon in the title bar, switch on
*Allow phones on this Wi-Fi* and scan the QR code with the phone's camera. The phone
opens the rig's pedalboard in its browser; nothing needs installing.

- The phone controls the whole rig: switches, knobs and faders, scenes, the preset
  (‹ ›), the tuner and the metronome. Turn the phone sideways for a bigger board.
- Both screens stay in step: a change on either shows up on the other.
- The phone plays but doesn't edit: design boards and the rig on the computer.
- The link holds a secret key. *New link* makes a new one and drops phones using the old.
- The remote stays on across launches until you switch it off. The first time, macOS
  may ask whether Guitar Studio may accept incoming connections — allow it.
- iPhone: Share → *Add to Home Screen* opens it full screen. Android has a full-screen button.
- The phone and computer must be on the same network. The page is plain `http` on the
  local network, so the browser may still dim and lock the phone's screen while you play.

Under `make dev`, the QR code points at the Vite server (it listens on the network for
this) and the page connects back to the app's port, 7480.

## Inside a DAW (REAPER and others)

Guitar Studio also runs as a VST3 / CLAP effect plugin, so the whole rig (its plugins,
cables, pedalboard, scenes) can sit on a DAW track.

```sh
make plugin-install   # build, then copy to ~/Library/Audio/Plug-Ins/{VST3,CLAP}
```

In REAPER, rescan plugins (Preferences → Plug-ins → VST → Re-scan; CLAP is picked up on
its own), then put **Guitar Studio** on a track with your guitar as its input. The plugin
window is the same app: rig, pedalboard, presets and scenes all work.

- **Audio comes from the track.** The DAW sets the sample rate and buffer, so there are
  no device settings. A mono track feeds both sides. A preset made in the desktop app
  for, say, input 4 of your interface just uses the track's input.
- **The rig is saved with the project.** Each instance remembers its rig, including its
  plugins' settings. Presets and the plugin list are shared with the desktop app, so
  presets saved in either show up in both.
- **Output** is the one parameter the DAW can automate (rig level, −60 to +12 dB).
- **Plugins load in the DAW's process.** Plugin scanning still runs in a separate
  process (`gs-scan`, inside the bundle), so a crashing plugin can't take the DAW down
  during a scan.
- Preloading of the previous/next preset starts once you switch presets in the plugin
  window, not when a project opens. This keeps a project with several rigs from loading
  extra plugins.
- In REAPER, turn on *Send all keyboard input to plugin* (FX window menu) to use the
  pedalboard's keyboard shortcuts.
- The plugin logs to `~/Library/Logs/Guitar Studio/plugin.log` (`RUST_LOG=debug` for
  more). `GS_PLUGIN_DEV_URL=http://localhost:1420` loads the UI from `npm run dev`
  instead of the bundle.
- `make plugin-test PRESET="pedalboard demo"` loads the built bundles headless. It checks
  audio, the Output parameter and saved state, and restores a preset as if from a
  project.

Not yet: an AU version (for Logic / GarageBand), the phone remote while running as a
plugin, and reporting hosted plugins' latency to the DAW.

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
| `crates/gs-engine` | Audio engine: cpal I/O or host-driven audio, graph compiler, RT executor, plugin hosting (truce-rack), tuner (McLeod pitch method), metronome |
| `crates/gs-app` | App layer shared by the desktop app and the plugin: state, every UI command (`dispatch`), presets, preloading, plugin-editor windows, telemetry pump (30 Hz) |
| `crates/gs-plugin` | Guitar Studio as a VST3 / CLAP plugin (truce): host audio, project state, the UI in a webview |
| `src-tauri` | Tauri shell: the `call` / `call_async` commands, phone-remote server (`remote.rs`) |
| `scripts/bundle-plugin.sh` | Packages the plugin bundles (`make plugin`) |
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
- **One command layer.** The UI sends every command by name with JSON arguments. The
  desktop window, the phone socket and the plugin window all go through
  `gs_app::dispatch`. Work that needs the main thread from elsewhere uses the main
  dispatch queue (`gs_app::main_thread`), which also works inside a DAW.

## Useful dev commands

```sh
make check                              # fmt-check + clippy + tsc + tests (what CI should run)
make smoke FORMAT=au PLUGIN=nolly       # load, process and state-round-trip a plugin headless
make preset-test FORMAT=au PLUGIN=nolly # save and reload a rig with a real plugin
make param-test FORMAT=au PLUGIN=nolly  # set and read back a plugin's parameters through the engine
make scene-test FORMAT=au PLUGIN=nolly  # switch scenes on a real plugin, save and reload them
make preset-timing                      # where preset switching spends its time, per plugin
cargo run --release -p gs-engine --example background_load -- au "Cory Wong"  # start a plugin off the main thread
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
