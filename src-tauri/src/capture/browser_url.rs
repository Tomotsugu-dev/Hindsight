//! 从浏览器地址栏抠取当前 URL，给隐私过滤判断用。
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
//!
//! 阻塞 + 偶尔卡 300ms（UIA 没异步 API），调用方包 `spawn_blocking`。
//!
//! 局限：macOS / Linux 暂不支持，永远返回 None。

/// 该应用是不是浏览器（基于进程名 / app_name 粗判）。
/// 非浏览器就不要花 UIA 调用的钱。
pub fn is_browser_app(app_name: &str) -> bool {
    let s = app_name.to_lowercase();
    s.contains("chrome")
        || s.contains("msedge")
        || s.contains("edge")
        || s.contains("firefox")
        || s.contains("brave")
        || s.contains("vivaldi")
        || s.contains("opera")
        || s.contains("arc")
        || s.contains("zen-browser")
        || s.contains("librewolf")
        || s.contains("waterfox")
}

/// 同步阻塞：从当前前台窗口抠 URL。调用方包 spawn_blocking。
#[cfg(target_os = "windows")]
pub fn try_get_foreground_browser_url() -> Option<String> {
    // catch_unwind 防 COM 异常一路把进程带崩
    std::panic::catch_unwind(get_url_via_uiautomation)
        .ok()
        .flatten()
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn try_get_foreground_browser_url() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
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
    let window_element = automation.element_from_handle(Handle::from(hwnd_raw)).ok()?;

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

/// 把 UIA 拿到的字符串规范化成 URL。能解析就返回，否则 None。
#[cfg(target_os = "windows")]
fn normalize_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    // 已经带 scheme 的直接收（含 chrome:// / edge:// / about: 等内部页 ——
    // 关键词列表里 /password 等子串能照常匹配上）
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("chrome://")
        || lower.starts_with("edge://")
        || lower.starts_with("about:")
        || lower.starts_with("file://")
    {
        return Some(s.to_string());
    }
    // Chrome 默认隐藏 https:// 前缀，地址栏可能就显示 "example.com/path"
    // 启发式：含 . 且不含空格、不含中文，补 https://
    if !s.contains(' ') && s.contains('.') {
        let has_cjk = s.chars().any(|c| c >= '\u{4E00}');
        if !has_cjk {
            return Some(format!("https://{s}"));
        }
    }
    None
}
