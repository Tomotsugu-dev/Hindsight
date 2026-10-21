//! The island: a small area in the middle of the menu bar that shows what Hindsight draws. It
//! does not know about plugins: which plugin has the island, and who gets a click, are in
//! `plugins/`. For the design, see island.md in docs/design/plugins/.

pub mod content;
pub mod geometry;
