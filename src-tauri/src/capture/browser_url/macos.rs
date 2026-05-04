//! macOS: 通过 AppleScript 抠浏览器地址栏 URL。
//!
//! 设计：
//!   1. 由 `app_name` 决定走哪条 AppleScript（Chromium 系 vs Safari）
//!   2. 内嵌脚本字符串调 `/usr/bin/osascript -e <script>` —— 不走外部 .applescript
//!      文件，免去打包资源 / dev-vs-release 路径分歧
//!   3. 解析 stdout，复用路由层 `normalize_url`
//!   4. osascript 失败（未授权 / 浏览器没开 / 无前窗）→ None
//!
//! Chromium 系（Chrome / Edge / Brave / Vivaldi / Opera / Arc / Chromium）：
//!   `tell application "<APP>" to get URL of active tab of front window`
//!
//! Safari：
//!   `tell application "Safari" to get URL of front document`
//!
//! Firefox 系（Firefox / Zen / LibreWolf / Waterfox）：**不支持** —— Firefox
//! 不暴露 AppleScript URL 接口；返回 None，URL 关键词那一路自动跳过，app/标题
//! 关键词仍然有效。
//!
//! 权限：第一次调会触发"自动化"权限弹框（System Settings → Privacy & Security
//! → Automation），文案来自 Info.plist 的 `NSAppleEventsUsageDescription`。
//! 用户拒绝 / 撤销 → osascript 非零退出 → 我们返回 None，与未实现时一致。
//!
//! 性能：osascript 启动 ~50–150ms（fork 子进程 + 解析 AS）。调用方已在
//! `service::tick` 里包了 `spawn_blocking`，且只在焦点切换时才调一次，可接受。

use std::process::Command;

use super::normalize_url;

const OSASCRIPT: &str = "/usr/bin/osascript";

/// 同步阻塞：根据 app_name 选脚本，调 osascript 拿 URL。
pub(super) fn try_get_url(app_name: &str) -> Option<String> {
    let script = build_script(app_name)?;

    let output = Command::new(OSASCRIPT)
        .arg("-e")
        .arg(&script)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    normalize_url(raw.trim())
}

/// 根据 app_name 生成对应的 AppleScript。不支持的浏览器返回 None。
fn build_script(app_name: &str) -> Option<String> {
    let lower = app_name.to_lowercase();

    // Safari（含 Safari Technology Preview）
    if lower.contains("safari") {
        let target = if lower.contains("technology preview") {
            "Safari Technology Preview"
        } else {
            "Safari"
        };
        return Some(format!(
            r#"tell application "{target}" to get URL of front document"#
        ));
    }

    // Chromium 系：直接拿 app_name 当 AppleScript 目标 app
    // （macOS 上 app_name 通常就是 .app 显示名，比如 "Google Chrome" / "Brave Browser"）
    if is_chromium_macos(&lower) {
        return Some(format!(
            r#"tell application "{}" to get URL of active tab of front window"#,
            escape_applescript(app_name)
        ));
    }

    // Firefox 系不支持
    None
}

fn is_chromium_macos(lower: &str) -> bool {
    lower.contains("chrome")
        || lower.contains("chromium")
        || lower.contains("edge") // Microsoft Edge
        || lower.contains("brave")
        || lower.contains("vivaldi")
        || lower.contains("opera")
        || lower.contains("arc")
}

/// AppleScript 字面量转义：反斜杠和双引号。
/// app_name 由 xcap 提供，正常情况不含这些字符；防御性转义防 AS 注入。
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_脚本() {
        let s = build_script("Google Chrome").unwrap();
        assert!(s.contains(r#"tell application "Google Chrome""#));
        assert!(s.contains("active tab of front window"));
    }

    #[test]
    fn edge_脚本() {
        let s = build_script("Microsoft Edge").unwrap();
        assert!(s.contains(r#"tell application "Microsoft Edge""#));
    }

    #[test]
    fn brave_脚本() {
        let s = build_script("Brave Browser").unwrap();
        assert!(s.contains(r#"tell application "Brave Browser""#));
    }

    #[test]
    fn safari_脚本() {
        let s = build_script("Safari").unwrap();
        assert!(s.contains(r#"tell application "Safari""#));
        assert!(s.contains("front document"));
    }

    #[test]
    fn safari_tp_脚本() {
        let s = build_script("Safari Technology Preview").unwrap();
        assert!(s.contains(r#"tell application "Safari Technology Preview""#));
    }

    #[test]
    fn firefox_不支持() {
        assert!(build_script("Firefox").is_none());
        assert!(build_script("LibreWolf").is_none());
        assert!(build_script("Zen Browser").is_none());
    }

    #[test]
    fn 转义引号() {
        assert_eq!(escape_applescript(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_applescript(r"a\b"), r"a\\b");
    }

    #[test]
    fn 非浏览器返回_none() {
        assert!(build_script("Slack").is_none());
        assert!(build_script("Code").is_none());
    }
}
