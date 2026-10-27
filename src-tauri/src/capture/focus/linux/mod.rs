//! Linux: picks the focus source for the desktop environment at startup, see [`choose`]. Desktops
//! without a source are not recorded. xcap is not used on Wayland: it only sees XWayland windows.

use std::sync::Arc;

use super::{FocusSource, WindowInfo};
use crate::error::{Error, Result};

mod gnome;
mod x11;

/// The focus sources implemented on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// GNOME on Wayland: asks the Hindsight Focus extension
    Gnome,
    /// X11 on any desktop: xcap
    X11,
}

/// Returns the Linux focus source for the desktop environment. The capture service calls it once,
/// when it is created.
pub(super) fn detect() -> Arc<dyn FocusSource> {
    let env = DesktopEnv::from_env();
    // Display protocol of the graphical login session, such as "x11" or "wayland"; only logged
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let Some(source) = choose(&env) else {
        log::warn!(
            "This desktop environment has no focus source yet; usage time will not be recorded ({env:?}, XDG_SESSION_TYPE={session_type:?})"
        );
        return Arc::new(Unsupported);
    };
    log::info!("Focus source: {source:?} ({env:?}, XDG_SESSION_TYPE={session_type:?})");
    match source {
        Source::Gnome => Arc::new(gnome::GnomeFocus::new()),
        Source::X11 => Arc::new(x11::X11Focus),
    }
}

/// Returns the focus source for this desktop environment.
///
/// None = this desktop environment is not supported yet
fn choose(env: &DesktopEnv) -> Option<Source> {
    match env.display? {
        // GNOME on Wayland
        DisplayServer::Wayland if env.has_desktop("GNOME") => Some(Source::Gnome),
        // X11 on any desktop, GNOME included
        DisplayServer::X11 => Some(Source::X11),
        _ => None,
    }
}

/// The user's desktop environment, read from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopEnv {
    /// None = no graphical display, such as when started over SSH
    display: Option<DisplayServer>,
    /// `XDG_CURRENT_DESKTOP` split on colons: "ubuntu:GNOME" gives `["ubuntu", "GNOME"]`
    desktops: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayServer {
    Wayland,
    X11,
}

impl DesktopEnv {
    fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// `var` looks up an environment variable by name; an empty value counts as unset.
    /// `WAYLAND_DISPLAY` is checked first: Wayland sessions usually set `DISPLAY` too, for XWayland.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| var(name).filter(|v| !v.is_empty());
        let display = if var("WAYLAND_DISPLAY").is_some() {
            Some(DisplayServer::Wayland)
        } else if var("DISPLAY").is_some() {
            Some(DisplayServer::X11)
        } else {
            None
        };
        let desktops = var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .collect();
        Self { display, desktops }
    }

    /// Whether `name` is in the desktop list, ignoring case.
    fn has_desktop(&self, name: &str) -> bool {
        self.desktops.iter().any(|d| d.eq_ignore_ascii_case(name))
    }
}

/// Used when no source applies: it never reads a window, so every tick is skipped and nothing is
/// recorded.
struct Unsupported;

impl FocusSource for Unsupported {
    fn focused_window(&self) -> Result<WindowInfo> {
        Err(Error::Capture(
            "no focus source for this desktop environment".into(),
        ))
    }
}

#[cfg(test)]
impl DesktopEnv {
    /// 用给定的环境变量构造，没列出的当作没有。
    fn with_vars(vars: &[(&str, &str)]) -> Self {
        Self::from_vars(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(vars: &[(&str, &str)]) -> Option<DisplayServer> {
        DesktopEnv::with_vars(vars).display
    }

    #[test]
    fn display_variables_name_the_display_server() {
        // Wayland 会话通常也设了 DISPLAY（给 XWayland），所以 WAYLAND_DISPLAY 优先
        assert_eq!(
            display(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]),
            Some(DisplayServer::Wayland)
        );
        assert_eq!(display(&[("DISPLAY", ":0")]), Some(DisplayServer::X11));
        // 都没有 = 没有图形界面，比如从 SSH 启动；不能当成 X11
        assert_eq!(display(&[]), None);
        assert_eq!(display(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", "")]), None);
    }

    #[test]
    fn session_type_does_not_decide() {
        // 文字终端登录后手动启动 Hyprland：登录类型是 tty，实际是 Wayland
        assert_eq!(
            display(&[
                ("XDG_SESSION_TYPE", "tty"),
                ("WAYLAND_DISPLAY", "wayland-1")
            ]),
            Some(DisplayServer::Wayland)
        );
        // 登录类型写着 wayland，但这个进程没有任何显示可连
        assert_eq!(display(&[("XDG_SESSION_TYPE", "wayland")]), None);
    }

    #[test]
    fn current_desktop_is_split_on_colons() {
        let env = DesktopEnv::with_vars(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]);
        assert_eq!(env.desktops, ["ubuntu", "GNOME"]);
        assert!(env.has_desktop("gnome"));
        assert!(!env.has_desktop("KDE"));

        assert!(DesktopEnv::with_vars(&[]).desktops.is_empty());
    }

    const WAYLAND_DISPLAY: (&str, &str) = ("WAYLAND_DISPLAY", "wayland-0");
    const X11_DISPLAY: (&str, &str) = ("DISPLAY", ":0");

    fn chosen(display: &[(&str, &str)], desktop: &str) -> Option<Source> {
        let mut vars = display.to_vec();
        vars.push(("XDG_CURRENT_DESKTOP", desktop));
        choose(&DesktopEnv::with_vars(&vars))
    }

    #[test]
    fn gnome_on_wayland_uses_the_extension() {
        // Ubuntu 在前面加了自己的名字；GNOME Classic 也是 GNOME
        for desktop in ["ubuntu:GNOME", "GNOME", "GNOME-Classic:GNOME", "pop:GNOME"] {
            assert_eq!(
                chosen(&[WAYLAND_DISPLAY, X11_DISPLAY], desktop),
                Some(Source::Gnome),
                "{desktop}"
            );
        }
    }

    #[test]
    fn x11_uses_xcap_on_any_desktop() {
        for desktop in ["ubuntu:GNOME", "KDE", ""] {
            assert_eq!(
                chosen(&[X11_DISPLAY], desktop),
                Some(Source::X11),
                "{desktop}"
            );
        }
    }

    #[test]
    fn other_wayland_desktops_have_no_source() {
        // Wayland 会话里的 DISPLAY 是给 XWayland 的，不能因此用 xcap。
        // 名字里带 GNOME 的别的桌面（单独出现的 GNOME-Flashback）也不算 GNOME
        for desktop in ["KDE", "Hyprland", "sway", "GNOME-Flashback", ""] {
            assert_eq!(
                chosen(&[WAYLAND_DISPLAY, X11_DISPLAY], desktop),
                None,
                "{desktop}"
            );
        }
        assert!(Unsupported.focused_window().is_err());
    }

    #[test]
    fn no_graphical_display_has_no_source() {
        assert_eq!(chosen(&[], "ubuntu:GNOME"), None);
    }
}
