//! 存下来的位置：数据目录下的 island.json。只在这台电脑上，不同步：每台电脑的屏幕不一样。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::screen::on_a_screen;

const FILE: &str = "island.json";

/// 胶囊左上角，物理像素
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) struct Saved {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) docked: bool,
}

fn path() -> PathBuf {
    crate::bootstrap::data_root().join(FILE)
}

/// 上次存的位置。不在任何一块屏上（比如拔了外接屏）就当没存过。
pub(super) fn load() -> Option<Saved> {
    let text = std::fs::read_to_string(path()).ok()?;
    let saved: Saved = serde_json::from_str(&text).ok()?;
    on_a_screen((saved.x, saved.y)).then_some(saved)
}

pub(super) fn save(saved: Saved) {
    let result = serde_json::to_string(&saved)
        .map_err(|e| e.to_string())
        .and_then(|text| std::fs::write(path(), text).map_err(|e| e.to_string()));
    if let Err(e) = result {
        log::warn!("island: cannot save where it is: {e}");
    }
}
