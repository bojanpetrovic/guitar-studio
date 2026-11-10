# Vendored crates

## truce-rack-vst3 1.1.5 (MIT OR Apache-2.0)

Upstream: https://github.com/truce-audio/truce-rack

Patched through `[patch.crates-io]` in the workspace `Cargo.toml`. Every change is
marked `[guitar-studio patch]` in `src/lib.rs`:

1. `load_state` now also calls `IEditController::setComponentState`, as the VST3
   spec requires. Before this, a restored preset reached the audio processor but
   the plugin's editor and parameters still showed the old values.
2. `activate` now calls `IComponent::activateBus` for the main audio and event
   buses. SDK-based plugins start with every bus inactive, so without this every
   VST3 plugin rendered silence (e.g. Pro-Q 3, NeuralAmpModeler, Timeless 3).
3. `process` now passes real `IParameterChanges` (with `IParamValueQueue`s) to the
   processor. Before this, `inputParameterChanges` was always null.
4. `set_parameter` queues the change for the processor as well as the controller.
   Before this, host parameter changes never reached the audio.
5. The host installs an `IComponentHandler`, so edits made in the plugin's own
   editor (`performEdit`) are forwarded to the processor.

Checked with `examples/vst3_param_audio.rs` (turning an output parameter down from
the host must lower the audio) and `examples/plugin_smoke.rs` (state round trip).

Known upstream gaps that are not patched yet:

- Controller-only state (`IEditController::getState`) isn't saved.
- `process` allocates (pointer arrays, event and parameter COM objects) on every
  block. It should reuse buffers.
- The processor's `outputParameterChanges` (meters and automation going out) are ignored.

These should go upstream as an issue or PR. Drop the patch once they land.
