// Drag and drop from the plugin list onto the rig canvas.

export const DRAG_MIME = "application/x-gs-plugin";
export const DRAG_GAIN = "gs:builtin:gain";
export const DRAG_MIXER = "gs:builtin:mixer";
/** Also set when dragging a real plugin (not Gain / Mixer): it can replace a plugin node. */
export const DRAG_REPLACEABLE = "application/x-gs-replaces";
