/**
 * The UI state key for the device selected in the main window: a device id or "all". Rust reads
 * it for plugin calls, as `DEVICE_SELECTED_KEY` in src-tauri/src/repo/ui_state.rs: change both
 * together.
 */
export const DEVICE_SELECTED_KEY = "hindsight.device.selected";
