//! 从一个进程的多个窗口里挑主窗口的标题。只有 macOS 的标题补取用它，但不依赖
//! macOS API，所以在所有平台上编译，单测在所有平台都跑。

/// 一个候选窗口的取标题所需属性（[`pick_main_window_title`] 的输入）。
/// 独立于 SCK 类型定义，好让选择逻辑可以脱离 macOS API 单测。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WindowCandidate {
    pub pid: u32,
    pub layer: i32,
    pub on_screen: bool,
    pub area: f64,
    pub title: String,
}

/// 从候选里挑「这个进程的主窗口」的标题：layer 0 + 在屏 + 面积最大。
///
/// 三条过滤与 [`crate::capture::screenshot_macos`] 的窗口选择**逐条对齐**——两边必须挑中
/// 同一个窗口，否则记下的标题和存下的像素来自不同窗口。
///
/// 面积最大这条同时解决了实测到的"无标题"元凶：每个 app 名下都挂着一批
/// 2128×30 / 1710×34 的无名窄条窗口（层级同为 0），按 z 序取第一个会挑中它们、
/// 读出空标题；按面积取则永远输给真正的主窗口（如 2128×1114）。
///
/// 返回 `None` = 没有可用窗口，或主窗口本身就没标题（例如 QQ音乐的播放器窗口，
/// CGWindowList / SCK / AX 三套 API 都返回空串）——那种是真·无标题，救不回来。
///
/// 非 macOS 上只有单测在用（Windows 走 GetWindowText，没有这套窗口枚举），
/// 但保持跨平台编译以便测试在所有平台都跑。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn pick_main_window_title(cands: &[WindowCandidate], pid: u32) -> Option<String> {
    let main = cands
        .iter()
        .filter(|w| w.pid == pid && w.layer == 0 && w.on_screen)
        .max_by(|a, b| {
            a.area
                .partial_cmp(&b.area)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let title = main.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fixture 尺寸取自实测：主窗口 2128×1114，无名窄条 2128×30 / 1710×34。
    fn cand(pid: u32, layer: i32, on_screen: bool, w: f64, h: f64, title: &str) -> WindowCandidate {
        WindowCandidate {
            pid,
            layer,
            on_screen,
            area: w * h,
            title: title.into(),
        }
    }

    #[test]
    fn main_window_title_beats_nameless_strips() {
        // 复现「无标题」的真实场景：同一 PID 名下一堆无名窄条 + 一个真窗口，
        // 按 z 序取第一个会读到空串，按面积取则拿到真标题。
        let cands = vec![
            cand(760, 0, true, 2128.0, 30.0, ""),
            cand(760, 0, true, 1710.0, 34.0, ""),
            cand(
                760,
                0,
                true,
                2128.0,
                1114.0,
                "Issue #24 · Tomotsugu-dev/Hindsight",
            ),
            cand(760, 0, true, 2128.0, 30.0, ""),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("Issue #24 · Tomotsugu-dev/Hindsight")
        );
    }

    #[test]
    fn main_window_title_ignores_other_processes() {
        let cands = vec![
            cand(1, 0, true, 3000.0, 2000.0, "别的 app 的大窗口"),
            cand(760, 0, true, 1200.0, 800.0, "自己的窗口"),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("自己的窗口")
        );
    }

    #[test]
    fn main_window_title_skips_non_normal_layer_and_offscreen() {
        // layer≠0 = dock / menubar / 系统模态之流；on_screen=false = 其它 Space
        // 或最小化。两者都不该被当成主窗口（与截图路径的过滤一致）。
        let cands = vec![
            cand(760, 103, true, 4000.0, 3000.0, "提示气泡"),
            cand(760, 0, false, 5000.0, 4000.0, "另一个 Space 的窗口"),
            cand(760, 0, true, 800.0, 600.0, "当前窗口"),
        ];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("当前窗口")
        );
    }

    #[test]
    fn main_window_title_none_when_genuinely_untitled() {
        // QQ音乐那种:主窗口本身就没标题，三套 API 都返回空串——救不回来，
        // 返回 None 让调用方保持"无标题"，别拿窄条的空串冒充"取到了"。
        let cands = vec![
            cand(760, 0, true, 1049.0, 709.0, "   "),
            cand(760, 0, true, 300.0, 200.0, "小面板"),
        ];
        assert_eq!(pick_main_window_title(&cands, 760), None);
    }

    #[test]
    fn main_window_title_none_when_no_candidate() {
        assert_eq!(pick_main_window_title(&[], 760), None);
        let cands = vec![cand(999, 0, true, 100.0, 100.0, "别人的")];
        assert_eq!(pick_main_window_title(&cands, 760), None);
    }

    #[test]
    fn main_window_title_trims_whitespace() {
        let cands = vec![cand(760, 0, true, 900.0, 700.0, "  带空格的标题  ")];
        assert_eq!(
            pick_main_window_title(&cands, 760).as_deref(),
            Some("带空格的标题")
        );
    }
}
