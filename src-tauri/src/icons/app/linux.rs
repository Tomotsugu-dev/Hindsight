//! Linux: finds the app among the installed apps (`.desktop` files), looks up the icon it names
//! in the current icon theme, and converts it to PNG. All desktops follow this standard.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::error::Result;

/// Icon side length: the UI shows icons at 18 px, and 128 leaves room for high-DPI screens.
const ICON_PX: i32 = 128;
/// The longest wait for the main thread, so the caller doesn't wait forever when it is stuck
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(5);

/// Returns the app's icon as a PNG.
///
/// Returns `Ok(None)` when the app is not found, it has no icon, or the main thread doesn't
/// answer in time.
pub fn extract_png(process_name: &str, exe_path: &Path) -> Result<Option<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    let name = process_name.to_string();
    let exe = exe_path.to_path_buf();
    // GTK can only be used on the main thread: its event loop does the lookup, and this thread
    // waits for the result
    glib::idle_add_once(move || {
        let _ = tx.send(icon_png(&name, &exe));
    });
    match rx.recv_timeout(MAIN_THREAD_TIMEOUT) {
        Ok(png) => Ok(png),
        Err(e) => {
            log::warn!("The main thread did not answer, so the icon of {process_name} is skipped this time: {e}");
            Ok(None)
        }
    }
}

/// Looks up the app's icon on the main thread and returns it as a PNG.
fn icon_png(process_name: &str, exe_path: &Path) -> Option<Vec<u8>> {
    let apps = gio::AppInfo::all();
    let entries: Vec<DesktopEntry> = apps.iter().map(DesktopEntry::of).collect();
    let app = &apps[find(&entries, process_name, exe_path)?];
    let icon = app.icon()?;
    let info = gtk::IconTheme::default()?.lookup_by_gicon(
        &icon,
        ICON_PX,
        gtk::IconLookupFlags::FORCE_SIZE,
    )?;
    let pixbuf = info.load_icon().ok()?;
    pixbuf.save_to_bufferv("png", &[]).ok()
}

/// The parts of a `.desktop` file that are compared with the app name and program path.
#[derive(Debug, Default)]
struct DesktopEntry {
    /// The `.desktop` file name without the extension, such as `org.kde.konsole`
    id: String,
    /// `StartupWMClass=`: the window class of this app, or empty when it is not set
    wm_class: String,
    /// The file name of the program in `Exec=`, such as `code`
    exec: String,
    /// `Icon=`: an icon name or the path to an icon file, such as `vscode`
    icon: String,
}

impl DesktopEntry {
    fn of(app: &gio::AppInfo) -> Self {
        let id = app.id().unwrap_or_default();
        let wm_class = app
            .downcast_ref::<gio::DesktopAppInfo>()
            .and_then(|d| d.startup_wm_class())
            .unwrap_or_default();
        // Not `executable()`: some `.desktop` files make it return a null pointer, which the gio
        // binding doesn't handle, and it crashes
        let commandline = app.commandline().unwrap_or_default();
        let icon = app
            .icon()
            .and_then(|icon| gio::prelude::IconExt::to_string(&icon))
            .unwrap_or_default();
        Self {
            id: id.trim_end_matches(".desktop").to_string(),
            wm_class: wm_class.to_string(),
            exec: program_name(commandline.to_str().unwrap_or_default()),
            icon: icon.to_string(),
        }
    }
}

/// Returns the file name of the program in a command line.
///
/// For example, `/usr/share/code/code --new-window %F` returns `code`. A program path with
/// spaces is quoted: `"/opt/My App/app" %U` returns `app`.
fn program_name(commandline: &str) -> String {
    let line = commandline.trim_start();
    let program = match line.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => line.split_whitespace().next().unwrap_or_default(),
    };
    Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Finds the app named `process_name` whose program is `exe_path`, and returns its position in
/// `entries`.
///
/// Tries these in order, the same order as the GNOME and KDE taskbars, and compares the app
/// name case-insensitively:
/// 1. `StartupWMClass=` is the app name, such as `konsole` for `org.kde.konsole`
/// 2. The `.desktop` file name is the app name, such as `google-chrome`
/// 3. The program in `Exec=` has the same name as the recorded program, such as VS Code's
///    `code`. When several apps use that program, it counts only if they all have the same
///    icon: VS Code has two `.desktop` files with the same icon, while a program like
///    `python3` is used by several different apps with different icons
fn find(entries: &[DesktopEntry], process_name: &str, exe_path: &Path) -> Option<usize> {
    if process_name.is_empty() {
        return None;
    }
    let is_name = |s: &str| s.eq_ignore_ascii_case(process_name);
    if let Some(i) = entries.iter().position(|e| is_name(&e.wm_class)) {
        return Some(i);
    }
    if let Some(i) = entries.iter().position(|e| is_name(&e.id)) {
        return Some(i);
    }
    let exe = exe_path.file_name()?.to_str()?;
    let mut same_exec = entries.iter().enumerate().filter(|(_, e)| e.exec == exe);
    let (first, entry) = same_exec.next()?;
    same_exec
        .all(|(_, e)| e.icon == entry.icon)
        .then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, wm_class: &str, exec: &str, icon: &str) -> DesktopEntry {
        DesktopEntry {
            id: id.into(),
            wm_class: wm_class.into(),
            exec: exec.into(),
            icon: icon.into(),
        }
    }

    /// 这台 Ubuntu 上装着的几个应用
    fn installed() -> Vec<DesktopEntry> {
        vec![
            entry("google-chrome", "", "google-chrome-stable", "google-chrome"),
            entry(
                "org.kde.konsole",
                "konsole",
                "konsole",
                "utilities-terminal",
            ),
            entry("org.kde.kwalletd5", "", "kwalletd5", "kwalletmanager"),
            // VS Code 装了两个 .desktop，程序和图标都一样
            entry(
                "com.microsoft.VSCode",
                "com.microsoft.VSCode",
                "code",
                "vscode",
            ),
            entry("com.microsoft.VSCode.UrlHandler", "", "code", "vscode"),
            entry("org.gnome.Shell", "", "gnome-shell", ""),
            // 两个不同的应用都用 python3，图标不一样
            entry("idle-python3.12", "", "python3", "idle"),
            entry("jupyter-notebook", "", "python3", "notebook"),
        ]
    }

    fn found(name: &str, exe: &str) -> Option<String> {
        let entries = installed();
        find(&entries, name, Path::new(exe)).map(|i| entries[i].id.clone())
    }

    #[test]
    fn file_name_is_the_app_name() {
        // GNOME 记 google-chrome，X11 记 Google-chrome，都是同一个
        assert_eq!(
            found("google-chrome", "/opt/google/chrome/chrome").as_deref(),
            Some("google-chrome")
        );
        assert_eq!(
            found("Google-chrome", "/opt/google/chrome/chrome").as_deref(),
            Some("google-chrome")
        );
    }

    #[test]
    fn startup_wm_class_is_the_app_name() {
        // 只有 StartupWMClass 对得上：文件名和程序名都不一样
        let entries = vec![entry("org.kde.terminal", "konsole", "terminal", "terminal")];
        assert_eq!(
            find(&entries, "Konsole", Path::new("/usr/bin/konsole")),
            Some(0)
        );
    }

    #[test]
    fn same_program_as_exec() {
        // VS Code 打开文件夹的对话框，窗口类名是 Code；两个 .desktop 图标一样，取哪个都行
        assert_eq!(
            found("Code", "/usr/share/code/code").as_deref(),
            Some("com.microsoft.VSCode")
        );
    }

    #[test]
    fn a_program_apps_with_different_icons_use_matches_none() {
        assert_eq!(found("python3", "/usr/bin/python3"), None);
    }

    #[test]
    fn earlier_rules_win() {
        // StartupWMClass 是应用名，比文件名是应用名先算
        let entries = vec![
            entry("tool", "", "other", "b"),
            entry("org.example.tool", "tool", "tool", "a"),
        ];
        assert_eq!(find(&entries, "tool", Path::new("/usr/bin/tool")), Some(1));
    }

    #[test]
    fn program_name_reads_the_first_word() {
        assert_eq!(program_name("/usr/share/code/code --new-window %F"), "code");
        assert_eq!(program_name("konsole"), "konsole");
        assert_eq!(
            program_name("  google-chrome-stable %U"),
            "google-chrome-stable"
        );
        // 路径里有空格时用引号括起来
        assert_eq!(program_name(r#""/opt/My App/app" %U"#), "app");
        assert_eq!(program_name(""), "");
    }

    /// 手动验证：在桌面会话里跑，看某个应用能不能找到图标。应用名和程序路径用环境变量
    /// `ICON_APP`、`ICON_EXE` 给，比如 `ICON_APP=konsole ICON_EXE=/usr/bin/konsole`。
    ///
    /// 跑法：`cargo test --lib linux_icon_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn linux_icon_manual() {
        gtk::init().expect("GTK 初始化失败：要在桌面会话里跑");
        let name = std::env::var("ICON_APP").unwrap_or_else(|_| "google-chrome".into());
        let exe = std::env::var("ICON_EXE").unwrap_or_default();
        // 跟程序里一样：从别的线程调用，由这个线程的事件循环替它去查
        let main_loop = glib::MainLoop::new(None, false);
        let quit = main_loop.clone();
        let worker = std::thread::spawn(move || {
            let png = extract_png(&name, Path::new(&exe));
            quit.quit();
            (name, png)
        });
        main_loop.run();
        let (name, png) = worker.join().unwrap();
        match png.unwrap() {
            Some(png) => {
                let img = image::load_from_memory(&png).unwrap();
                println!(
                    "{name}: {}x{} PNG, {} bytes",
                    img.width(),
                    img.height(),
                    png.len()
                );
            }
            None => println!("{name}: 没有图标"),
        }
    }

    #[test]
    fn unknown_or_empty_app_matches_none() {
        // dev 版的 Hindsight 没有 .desktop
        assert_eq!(
            found("Hindsight", "/home/u/Hindsight/target/debug/hindsight"),
            None
        );
        assert_eq!(found("", "/usr/bin/konsole"), None);
    }
}
