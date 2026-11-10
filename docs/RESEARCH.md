# Guitar Studio: Tech Research (Nov 2026)

Goal: a desktop guitar studio in Rust. It loads VST3/AU (and ideally CLAP) plugins, lets you route them freely in a node graph, and includes a metronome and tuner. The UI should look modern.
Target machine: macOS 26, Apple Silicon, Rust 1.97.

---

## 1. The stack at a glance

| Layer | Recommendation | Alternatives |
|---|---|---|
| Audio I/O | `cpal` 0.18 (CoreAudio) | `coreaudio-rs` directly, for a true duplex AUHAL on macOS |
| Plugin hosting | `truce-rack` 1.x (VST3 + CLAP + AUv2 + AUv3, pure Rust) | `vst3` 0.3 crate + `objc2-audio-toolbox` (DIY); `clack-host` (CLAP); `vst3-host` |
| Routing graph | Our own small engine (`petgraph` on the UI side, compiled schedule sent to the audio thread) | `firewheel` 0.12 (good design to borrow from, but Bevy-focused) |
| Real-time plumbing | `rtrb`, `triple_buffer`, `basedrop`, `assert_no_alloc`, `audio_thread_priority` | `crossbeam` |
| Tuner DSP | McLeod Pitch Method (`pitch-detection` or `pitch-estimate`), `realfft` | pYIN (`pyin`) for robustness |
| Metronome | Sample-accurate click synthesized on the audio thread | Sampled clicks via `symphonia`/`hound` |
| UI | `eframe`/`egui` (wgpu) + `egui-snarl` node editor, custom theme | Tauri v2 + Svelte Flow/React Flow (web UI); GPUI + gpui-component; Slint; Vizia |
| MIDI (foot controllers) | `midir` | |
| Persistence | `serde` + `ron`/JSON; plugin state as base64 blobs | |
| Packaging | `cargo-packager` or `cargo-bundle`, plus codesign and notarization | |

---

## 2. Plugin hosting (the hardest part)

### Licensing news that helps us
Steinberg relicensed the **VST3 SDK under MIT** starting with VST 3.8 (Oct 2025). GPL and proprietary agreements are no longer required. We can host VST3 commercially with no paperwork, though use of the "VST" logo still follows Steinberg's trademark rules.

### Candidate crates
- **`truce-rack`** (github.com/truce-audio/truce-rack). MIT/Apache, v1.1.5, released 2 Nov 2026.
  - A from-scratch, pure-Rust rewrite of the older `rack` crate, built on `objc2`, `clap-sys`, `vst3` and `lilv-sys`.
  - Formats: CLAP and VST3 are on by default. AUv2 and AUv3 sit behind the `au`/`au3` features. LV2 is optional.
  - Covers scanning, loading, processing, MIDI, synthesized host transport (tempo and position), and plugin editors (embedded via baseview on macOS).
  - **Risk:** young project (~12 stars). Real-time safety and crash isolation aren't documented. **Mitigation:** hide it behind our own `PluginInstance` trait so a backend can be swapped.
- **`vst3`** 0.3 (coupler-rs). MIT/Apache bindings with pre-generated code, so it needs no libclang or SDK at build time. This is the base layer if we write VST3 hosting ourselves. Avoid `vst3-sys`, which is GPLv3.
- **`clack-host`** (prokopyl/clack). A safe, low-level CLAP host and the de-facto Rust CLAP host layer. It has a cpal host example.
- **`vst3-host`** (HelgeSverre). MIT, VST3 only. It has out-of-process isolation and egui editor embedding (macOS). By its own docs, its default play path takes locks on the audio callback.
- **`objc2-audio-toolbox` / `objc2-avf-audio`**. Raw Apple bindings, for DIY AU hosting (AudioComponent / AVAudioUnit) if needed.
- `rack` 0.4.x: superseded by truce-rack. Its VST3 path used C++ FFI and it had no VST3 GUI.

### Hosting concerns we must design for
1. **Scanning crashes.** Some plugins crash when probed. Scan in a **separate helper process** and cache the results (by path and mtime).
2. **Crashes at runtime.** Version 1 runs plugins in-process. The architecture should leave room for out-of-process "sandboxed" plugins later.
3. **Editor windows.** Plugin GUIs need a native parent (`NSView`). Show them as floating native windows, the way Logic and Gig Performer do, rather than inside the egui canvas. The `raw-window-handle` and baseview crates provide this.
4. **Latency reporting.** Each plugin reports its latency. Parallel branches in the graph need **delay compensation** before they merge.
5. **State.** Save each plugin's state blob (VST3 `getState`, AU `ClassInfo`, CLAP `state`) inside our patch files.
6. **macOS entitlements.** A hardened-runtime app needs `com.apple.security.cs.disable-library-validation` to load third-party plugin binaries. Live input needs `NSMicrophoneUsageDescription` in Info.plist.

---

## 3. Audio engine and routing graph

**Threads**
- **Audio thread** (CoreAudio callback, real-time priority). Runs the compiled graph: input → nodes → output. No allocations, locks or syscalls.
- **UI thread.** Owns the editable graph model (`petgraph::StableGraph`). On every edit it topologically sorts, inserts delay compensation, builds an immutable `Schedule`, and sends it over an `rtrb` queue.
- **GC path.** The audio thread swaps in the new schedule and returns the old one through a queue, so the UI side frees it (or use `basedrop`).
- **Analysis thread.** The tuner, spectrum and meters read audio from lock-free ring buffers.
- **Parameters.** UI → audio via a lock-free event queue. Audio → UI meters via `triple_buffer` or atomics.

**Graph model**
- Node types: `AudioInput`, `AudioOutput`, `Plugin`, `Splitter`, `Mixer`, `Gain`, `Tuner` (tap), `Metronome`, `Looper` (later).
- Ports carry typed channel counts (mono/stereo). Edges can fan out and fan in, and fan-in is summed.
- The graph must stay a DAG (cycle detection). Feedback loops would need explicit delay nodes; that's a later feature.
- Process in fixed blocks (64/128/256 frames). Keep the host block size fixed so plugins get consistent `max_block_size`.

**I/O on macOS**
`cpal` opens **separate input and output streams** and has no true duplex API. Two options:
- (a) Bridge input to output with an `rtrb` ring buffer. Simple, but adds about one buffer of latency.
- (b) Use `coreaudio-rs` with an AUHAL / aggregate device for a single duplex callback.

Start with (a) behind an `AudioBackend` trait. Move to (b) if round-trip latency matters, and for guitar monitoring it will.

`firewheel` (MIT/Apache) is a good reference for "compile graph → schedule → RT executor". It's now Bevy-focused and not meant as a DAW engine, so we write our own, smaller one.

---

## 4. Tuner

- Guitar range: low E ≈ 82.4 Hz (drop D ≈ 73.4 Hz, 7-string B ≈ 61.7 Hz) up to roughly 1.3 kHz fundamentals.
- Algorithm: **McLeod Pitch Method (MPM / NSDF)**. It's accurate on guitar, cheap, and works with short windows. Use a 2048–4096 sample window at 48 kHz with a hop of about 512.
- Pipeline: tap the input bus → ring buffer → analysis thread. Optionally decimate by 2–4, apply a high-pass filter around 50 Hz and a noise gate, then run MPM and a clarity threshold. Smooth with a median filter plus EMA, then map to note and cents (A4 adjustable, default 440).
- Crates: `pitch-detection` (McLeod + YIN; old, from 2022, but small enough to vendor), `pitch-estimate` (newer, stateless), `pyin` (more robust, heavier). The core algorithm is ~150 lines, so writing our own is also reasonable.
- UI: a large needle or strobe display with ±50 cents and a mute-output-while-tuning toggle.

## 5. Metronome

- Generated **on the audio thread**, sample-accurate. Track the beat phase in samples, not wall-clock timers.
- Click: a short synthesized blip (sine burst with an exponential decay), with an accented downbeat. Optional sample sounds.
- Controls: BPM 20–300, time signature, subdivisions, accent pattern, tap tempo, volume, and routing (headphones only vs. main mix).
- The same transport (tempo, position, playing) is passed to plugins, so tempo-synced delays and modulation follow the metronome.

---

## 6. UI: making it look modern

| Option | Pros | Cons |
|---|---|---|
| **egui + egui-snarl** (recommended) | Pure Rust. Ready-made node-graph editor (typed nodes, serializable, custom pin widgets, configurable wires). Immediate mode suits constantly animating meters and tuners. wgpu rendering. Used in audio projects (vst3-host embeds into egui). | The default look is "dev tool". It needs deliberate theming: custom fonts (Inter), spacing, rounded panels, accent colors, custom-painted knobs and meters. |
| **Tauri v2 + Svelte Flow / React Flow** | Easiest way to a beautiful UI. Polished node editors exist off the shelf. CSS animations. | The UI is TypeScript, not Rust. IPC for meters, though 60 Hz is fine. |
| **GPUI + gpui-component** | Native, fast, very polished look (Zed). | Thin docs. No node editor, so we'd build one. |
| **Slint** | Declarative, good-looking, designer-friendly. | GPLv3 or royalty-free license. No node editor. |
| **Vizia** | Built with audio in mind (nih-plug). CSS-like styling. | Smaller ecosystem. No mature node editor. |
| **iced** | Elm architecture, MIT. | Node editor ecosystem is immature. |

Recommendation: **egui + egui-snarl** with a strong custom design system: dark theme, one accent color, Inter or Geist fonts, a grid background, smooth bezier "cables", and custom-painted knobs, meters and tuner. If a pixel-perfect web-grade look matters more than "pure Rust", Tauri is the alternative.

---

## 7. Proposed workspace layout

```
guitar-studio/
  crates/
    gs-engine/     # audio backend trait, graph model, schedule compiler, RT executor
    gs-plugins/    # PluginInstance trait + truce-rack backend, scanner protocol
    gs-scanner/    # separate binary: crash-safe plugin scanning
    gs-dsp/        # tuner (MPM), metronome, meters, gain/mixer nodes
    gs-app/        # eframe UI: rack/graph view, tuner, metronome, settings
  docs/
```

## 8. Milestones

1. **Audio passthrough.** Pick input/output devices, set the buffer size, show the latency readout and input meter.
2. **Tuner and metronome.** Both are pure Rust, so they're quick wins that exercise the RT architecture.
3. **Plugin spike.** Scan, load one VST3 and one AU, process audio, and open its editor using `truce-rack`, tested with *your* actual plugins. This is the go/no-go for the hosting crate.
4. **Graph engine.** Serial chain first, then splits and merges with summing, then latency compensation.
5. **Node editor UI** with egui-snarl, wired to the engine.
6. **Patches and presets.** Save and load graph and plugin states. Add MIDI foot-controller mapping.
7. **Polish.** Theming, animations, CPU meter, packaging, signing, notarization.
