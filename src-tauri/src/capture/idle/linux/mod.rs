//! Linux：每个桌面一份实现，放在这个目录里，启动时按桌面选。现在还一个都没有，所以当作
//! 用户一直在用、屏幕一直能看：挂机检测不生效，其它功能照常。

use std::sync::Arc;

use super::IdleSource;

/// 返回 Linux 的来源。
pub(super) fn detect() -> Arc<dyn IdleSource> {
    Arc::new(Unsupported)
}

struct Unsupported;

impl IdleSource for Unsupported {
    fn idle_secs(&self) -> u64 {
        0
    }

    fn screen_unavailable(&self) -> bool {
        false
    }

    fn display_keepawake_active(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_treats_the_user_as_present() {
        // 没有来源时不能误封会话：一直在用、屏幕一直能看、没有防睡请求
        let idle = detect();
        assert_eq!(idle.idle_secs(), 0);
        assert!(!idle.screen_unavailable());
        assert!(!idle.display_keepawake_active());
    }
}
