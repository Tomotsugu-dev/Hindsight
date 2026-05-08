//! Windows: UI Automation 抠前台窗口 URL。
//!
//! 设计：
//!   1. `GetForegroundWindow()` 拿到当前前台窗口 HWND
//!   2. UI Automation 扫该窗口下所有 Edit / Document 控件
//!   3. 拿每个控件的 value pattern / legacy IAccessible value / name，
//!      第一个能解析成 URL（含 chrome:// 等内部 scheme）就返回
//!   4. 找不到返回 None
//!
//! 不评分、不挑"最像地址栏的那个" —— 我们只用结果做"要不要存截图"的判断，
//! 偶尔误把表单 URL 当地址栏 URL 影响很小（最坏只是少截一张截图，对隐私是
//! 更安全的方向）；而评分门槛会在 Chrome 高版本改 omnibox classname、
//! chrome:// 不带 http(s) 等场景里把真正的地址栏挡掉。
//!
//! 注意 matcher 必须显式 `.depth(50)` —— uiautomation 0.24 的 matcher 默认
//! 只看直接子节点，浏览器地址栏埋在很深的子树里，不开 depth 永远找不到。

use super::normalize_url;

/// 同步阻塞：从当前前台窗口抠 URL。调用方包 spawn_blocking。
/// `_app_name` 在 Windows 上忽略（走 GetForegroundWindow 自洽拿前台 HWND）。
pub(super) fn try_get_url(_app_name: &str) -> Option<String> {
    // catch_unwind 防 COM 异常一路把进程带崩
    std::panic::catch_unwind(get_url_via_uiautomation)
        .ok()
        .flatten()
}

fn get_url_via_uiautomation() -> Option<String> {
    use uiautomation::patterns::{UILegacyIAccessiblePattern, UIValuePattern};
    use uiautomation::types::{ControlType, Handle};
    use uiautomation::UIAutomation;
    use winapi::um::winuser::GetForegroundWindow;

    let hwnd_raw = unsafe { GetForegroundWindow() } as isize;
    if hwnd_raw == 0 {
        return None;
    }

    let automation = UIAutomation::new().ok()?;
    let window_element = automation
        .element_from_handle(Handle::from(hwnd_raw))
        .ok()?;

    // 任何 Edit / Document 控件的 value 第一个能解析成 URL 的就用——见文件头注释里的设计说明。
    let try_extract = |elem: &uiautomation::UIElement| -> Option<String> {
        if let Ok(p) = elem.get_pattern::<UIValuePattern>() {
            if let Ok(v) = p.get_value() {
                if let Some(u) = normalize_url(&v) {
                    return Some(u);
                }
            }
        }
        if let Ok(p) = elem.get_pattern::<UILegacyIAccessiblePattern>() {
            if let Ok(v) = p.get_value() {
                if let Some(u) = normalize_url(&v) {
                    return Some(u);
                }
            }
        }
        if let Ok(name) = elem.get_name() {
            if let Some(u) = normalize_url(&name) {
                return Some(u);
            }
        }
        None
    };

    // 先扫 Edit（地址栏几乎都是 Edit）
    if let Ok(edits) = automation
        .create_matcher()
        .from(window_element.clone())
        .control_type(ControlType::Edit)
        .timeout(2000)
        .depth(50)
        .find_all()
    {
        for el in edits {
            if let Some(url) = try_extract(&el) {
                return Some(url);
            }
        }
    }

    // 再扫 Document 兜底（少数浏览器版本把 URL 放在 Document 上）
    if let Ok(docs) = automation
        .create_matcher()
        .from(window_element)
        .control_type(ControlType::Document)
        .timeout(2000)
        .depth(50)
        .find_all()
    {
        for el in docs {
            if let Some(url) = try_extract(&el) {
                return Some(url);
            }
        }
    }

    None
}
