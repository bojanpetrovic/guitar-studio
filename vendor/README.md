# Vendored crates

## truce-rack-vst3 1.1.5 (MIT OR Apache-2.0)

Upstream: https://github.com/truce-audio/truce-rack

Patched through `[patch.crates-io]` in the workspace `Cargo.toml`. Every change is
marked `[guitar-studio patch]` in `src/lib.rs`:

1. `load_state` now also calls `IEditController::setComponentState`, as the VST3
   spec requires. Before this, a restored preset reached the audio processor but
   the plugin's editor and parameters still showed the old values.

Known upstream gaps that are not patched yet:

- No `IComponentHandler`, so the host doesn't forward edits made in a split
  component/controller plugin's editor (`performEdit`) to the processor.
  Single-instance plugins (JUCE, e.g. Neural DSP) are unaffected.
- `set_parameter` updates only the controller (no `IParameterChanges` in `process`).
- Controller-only state (`IEditController::getState`) isn't saved.

These should go upstream as an issue or PR. Drop the patch once they land.
