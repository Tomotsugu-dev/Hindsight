//! 诊断 macOS 上 xcap + sysinfo 拿不到 app_path 的根因。
//! 运行：从 src-tauri 目录 `cargo run --example xcap_diag`
//!
//! 第一次跑会弹"屏幕录制权限"系统对话框（CGRequestScreenCaptureAccess）。
//! 点允许后再跑一次，xcap::Window::all() 才能看到其它进程的窗口。

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(target_os = "macos")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

fn main() {
    #[cfg(target_os = "macos")]
    {
        // CGPreflightScreenCaptureAccess: 仅查询，不弹框。
        // CGRequestScreenCaptureAccess: 没权限时弹系统对话框 + 同步等用户决定 / 把 app 加进列表。
        let pre = unsafe { CGPreflightScreenCaptureAccess() };
        println!("CGPreflightScreenCaptureAccess() = {pre}");
        if !pre {
            println!("→ 调用 CGRequestScreenCaptureAccess() 弹授权框…");
            let granted = unsafe { CGRequestScreenCaptureAccess() };
            println!("CGRequestScreenCaptureAccess() returned {granted}");
            println!(
                "注意：即使返回 true，xcap 拿到完整窗口列表也要等下一次进程启动 ——\n\
                 macOS TCC 是按进程启动时缓存的，授完权后请重跑这个 example 验证。"
            );
        }
        println!();
    }

    println!("=== xcap::Window::all() ===\n");
    let windows = match xcap::Window::all() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xcap::Window::all 失败: {e}");
            return;
        }
    };

    let mut sys = System::new();
    let pids: Vec<Pid> = windows
        .iter()
        .filter_map(|w| w.pid().ok().map(|p| Pid::from_u32(p as u32)))
        .collect();
    if !pids.is_empty() {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
        );
    }

    for (i, w) in windows.iter().enumerate() {
        let app_name = w.app_name().unwrap_or_else(|e| format!("<err: {e}>"));
        let title = w.title().unwrap_or_else(|e| format!("<err: {e}>"));
        let pid = w.pid().unwrap_or(0);
        let focused = w.is_focused().unwrap_or(false);
        let on_screen = w.is_minimized().map(|m| !m).unwrap_or(false);

        let exe = if pid > 0 {
            sys.process(Pid::from_u32(pid as u32))
                .map(|p| {
                    p.exe()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "<exe()=None>".into())
                })
                .unwrap_or_else(|| "<sys.process()=None>".into())
        } else {
            "<pid=0>".into()
        };

        let star = if focused { "★" } else { " " };
        println!(
            "{star} [{i:>2}] pid={pid:>6} focused={focused} onscreen={on_screen}\n     app_name = {app_name:?}\n     title    = {title:?}\n     exe      = {exe}",
        );
    }
}
