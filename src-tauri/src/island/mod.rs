//! The island: a small area in the middle of the menu bar that shows what Hindsight draws. It
//! does not know about plugins: which plugin has the island, and who gets a click, are in
//! `plugins/`. For the design, see island.md in docs/design/plugins/.

pub mod content;
// Only the macOS window uses the shapes
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod geometry;
#[cfg(target_os = "macos")]
pub mod macos;
