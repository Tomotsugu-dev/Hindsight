//! Linux：同一个安装包会装在不同桌面上，所以按会话挑焦点来源。
//!
//! 目前只有 [`x11`] 一种，所有会话都用它，和重构前一样。以后 GNOME Wayland 改问
//! Hindsight Focus 扩展，X11 继续用 [`x11`]。

use std::sync::Arc;

use super::FocusSource;

mod x11;

/// 返回 Linux 的焦点来源。
pub(super) fn detect() -> Arc<dyn FocusSource> {
    Arc::new(x11::X11Focus)
}
