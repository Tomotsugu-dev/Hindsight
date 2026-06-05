//! macOS `.app` bundle 归一：把"helper / mini-program 子 bundle"折叠到外层 app。
//!
//! 背景：很多 app 把 helper（Electron renderer / 微信小程序 / QQ 文档 等）打包成
//! **嵌套** `.app`，OS 的 `NSWorkspace.frontmostApplication` 会把这些子进程当成
//! 独立 app 报上来 —— `localizedName` 是 "WeChatAppEx" 而不是 "WeChat"。结果是
//! 用户在 WeChat 主聊天窗口的时间被错记成 "WeChat" 0 行、helper 0~N 行。
//!
//! 见到的两种嵌套布局：
//! - **类 A（腾讯系）**：`Parent.app/Contents/MacOS/Child.app/Contents/MacOS/Child`
//!   —— WeChat / QQ / Parallels Desktop / iMazing
//! - **类 B（Electron 系）**：`Parent.app/Contents/Frameworks/Child Helper.app/...`
//!   —— Chrome / VSCode / Claude / JLCONE
//! - **类 C（混合，三层 .app）**：`QQ.app/Contents/MacOS/QQEXDOC.app/Contents/Frameworks/QQEXDOC Helper.app/...`
//!
//! [`outermost_app_bundle`] 用一条统一规则覆盖三类：从可执行文件路径向 root 方向
//! 走，找**最外层**那个 `.app` 目录。

use std::path::{Path, PathBuf};

/// 从可执行文件 / bundle 路径里找最外层的 `.app` 目录。
///
/// 返回 `None` = 路径里没有 `.app`（裸 binary，如 `target/debug/hindsight` dev 跑的
/// Hindsight 自身）。
///
/// 不分类 A/B/C；只看 path components 里第一个以 `.app` 结尾的目录，因为按定义
/// 它已经是最外层 bundle —— 嵌套子 bundle 必然在它之下。
pub(crate) fn outermost_app_bundle(exe_path: &Path) -> Option<PathBuf> {
    let mut acc = PathBuf::new();
    for comp in exe_path.components() {
        let part = comp.as_os_str();
        acc.push(part);
        if part.to_string_lossy().ends_with(".app") {
            return Some(acc);
        }
    }
    None
}

/// 把 NSWorkspace 返回的 (raw_name, raw_path) 折叠到最外层 app 身份。
///
/// 调用方：[`crate::capture::window::macos_resolve_focused_window`]。
///
/// 三种返回情况：
/// 1. `raw_path = None` 或路径不含 `.app`（裸 binary，dev 跑的 Hindsight 自身）：
///    `(raw_name, None)` —— 退化到原行为
/// 2. 路径本身就是最外层 `.app`（如 `/Applications/Chrome.app`，未嵌套）：
///    `(raw_name, Some(raw_path))` —— 不动
/// 3. 路径在嵌套子 bundle 下（WeChatAppEx / Claude Helper / QQEXDOC 等）：
///    `(parent_name, Some(parent_path))`，`parent_name` 优先从父 bundle 的
///    `Info.plist` 读 `CFBundleDisplayName` → `CFBundleName`；都缺时退到
///    `parent_path` 的 basename 去掉 `.app` 后缀
///
/// **注意**：函数有 IO（读父 bundle Info.plist），但仅在嵌套情况才发生 ——
/// 99% 前台采样命中情况 2 零额外 IO。读失败不抛错，silently fallback。
pub(crate) fn canonicalize_to_parent_bundle(
    raw_name: &str,
    raw_path: Option<&str>,
) -> (String, Option<String>) {
    let Some(raw_path_str) = raw_path else {
        return (raw_name.to_string(), None);
    };
    let raw_path_buf = Path::new(raw_path_str);

    let Some(outer) = outermost_app_bundle(raw_path_buf) else {
        // 路径里没 .app 段（裸 binary）—— 保持 NSWorkspace 给的原始值
        return (raw_name.to_string(), Some(raw_path_str.to_string()));
    };

    if outer.as_path() == raw_path_buf {
        // 已经是最外层 bundle，无需归一
        return (raw_name.to_string(), Some(raw_path_str.to_string()));
    }

    // 嵌套情况：读父 bundle Info.plist 拿 canonical name
    let canonical_name = read_canonical_name(&outer).unwrap_or_else(|| {
        // plist 缺 / 损坏：退回 basename，去 .app 后缀
        outer
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.strip_suffix(".app").unwrap_or(s).to_string())
            .unwrap_or_else(|| raw_name.to_string())
    });

    (canonical_name, Some(outer.to_string_lossy().into_owned()))
}

/// 读 bundle 的 `Contents/Info.plist`，优先 `CFBundleDisplayName`（用户可见名），
/// 缺时退 `CFBundleName`。两者皆缺 / 文件不存在 / 解析失败 → `None`。
fn read_canonical_name(bundle: &Path) -> Option<String> {
    let plist_path = bundle.join("Contents/Info.plist");
    if !plist_path.exists() {
        return None;
    }
    let value = plist::Value::from_file(&plist_path).ok()?;
    let dict = value.as_dictionary()?;
    let pick = |key: &str| -> Option<String> {
        dict.get(key)
            .and_then(|v| v.as_string())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    pick("CFBundleDisplayName").or_else(|| pick("CFBundleName"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用不存在的路径走一遍 [`canonicalize_to_parent_bundle`]：覆盖纯路径分支 +
    /// plist 读失败时的 basename fallback。**真正的 plist 读路径**只能靠手工 e2e
    /// 验证（需要 WeChat / Claude 等真实 bundle 在线）。
    #[test]
    fn canonicalize_nested_fallback_to_basename_when_plist_missing() {
        // 嵌套 helper：父 bundle 不在磁盘 → plist 读失败 → 退到 basename "WeChat"
        let (name, path) = canonicalize_to_parent_bundle(
            "WeChatAppEx",
            Some(
                "/no/such/dir/WeChat.app/Contents/MacOS/WeChatAppEx.app/Contents/MacOS/WeChatAppEx",
            ),
        );
        assert_eq!(
            name, "WeChat",
            "嵌套时 canonical name 应是父 bundle basename"
        );
        assert_eq!(
            path.as_deref(),
            Some("/no/such/dir/WeChat.app"),
            "path 应归到最外层 .app",
        );

        // Claude Helper (Renderer) 嵌套 + plist 缺 → "Claude"
        let (name, path) = canonicalize_to_parent_bundle(
            "Claude Helper (Renderer)",
            Some("/no/such/dir/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)"),
        );
        assert_eq!(name, "Claude");
        assert_eq!(path.as_deref(), Some("/no/such/dir/Claude.app"));
    }

    #[test]
    fn canonicalize_non_nested_passthrough() {
        // 路径本身就是最外层 .app（NSWorkspace 给主进程时的典型形状）→ 不动
        let (name, path) = canonicalize_to_parent_bundle(
            "Calculator",
            Some("/System/Applications/Calculator.app"),
        );
        assert_eq!(name, "Calculator");
        assert_eq!(path.as_deref(), Some("/System/Applications/Calculator.app"));
    }

    #[test]
    fn canonicalize_no_path_passthrough() {
        // bundleURL 为 None（极少数 binary 没 bundle）→ 退化到原 (name, None)
        let (name, path) = canonicalize_to_parent_bundle("some_daemon", None);
        assert_eq!(name, "some_daemon");
        assert!(path.is_none());
    }

    #[test]
    fn canonicalize_bare_binary_keeps_raw_path() {
        // 路径里没 .app（dev cargo run 的 Hindsight 自身）→ 保留 raw path 不动
        let (name, path) = canonicalize_to_parent_bundle(
            "hindsight",
            Some("/Users/x/Hindsight/target/debug/hindsight"),
        );
        assert_eq!(name, "hindsight");
        assert_eq!(
            path.as_deref(),
            Some("/Users/x/Hindsight/target/debug/hindsight")
        );
    }

    /// 6 条 fixture，全部来自实测机器（`find /Applications -name "*.app"` 真实路径），
    /// 覆盖三种嵌套布局 + 正常单层 + 无 `.app` 的兜底。
    #[test]
    fn outermost_app_bundle_real_world_fixtures() {
        let cases: &[(&str, Option<&str>)] = &[
            // 类 A：腾讯嵌套 (WeChat → WeChatAppEx)
            (
                "/Applications/WeChat.app/Contents/MacOS/WeChatAppEx.app/Contents/MacOS/WeChatAppEx",
                Some("/Applications/WeChat.app"),
            ),
            // 类 B：Electron Helper (Claude → Claude Helper (Renderer))
            (
                "/Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)",
                Some("/Applications/Claude.app"),
            ),
            // 类 B：Framework 多层版本目录 (Chrome → Helper)
            (
                "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/148.0.7778.167/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper",
                Some("/Applications/Google Chrome.app"),
            ),
            // 类 C：三层 (.app/.../.app/.../.app)，最外层 = QQ.app
            (
                "/Applications/QQ.app/Contents/MacOS/QQEXDOC.app/Contents/Frameworks/QQEXDOC Helper.app/Contents/MacOS/QQEXDOC Helper",
                Some("/Applications/QQ.app"),
            ),
            // 正常单层 bundle（系统 Calculator）
            (
                "/System/Applications/Calculator.app/Contents/MacOS/Calculator",
                Some("/System/Applications/Calculator.app"),
            ),
            // 裸 binary（dev cargo run 的 Hindsight 自身，没 .app）
            (
                "/Users/kyotomogen/Program_Files/Hindsight/src-tauri/target/debug/hindsight",
                None,
            ),
        ];

        for (input, expected) in cases {
            let got = outermost_app_bundle(Path::new(input));
            let expected_buf = expected.map(PathBuf::from);
            assert_eq!(
                got, expected_buf,
                "outermost_app_bundle(\"{input}\") want {expected:?} got {got:?}",
            );
        }
    }
}
