//! Linux：还没有实现，一律返回 None。

use crate::capture::focus::WindowInfo;

pub(super) fn read_url(_window: &WindowInfo) -> Option<String> {
    None
}
