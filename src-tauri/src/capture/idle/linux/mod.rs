//! Linux: one implementation per desktop, in this folder, picked at startup by [`choose`]. On
//! desktops without one, the user is treated as always present and the screen as always visible:
//! idle detection is off, and everything else works as usual.

use std::sync::Arc;

use super::IdleSource;

mod gnome;
mod x11;

/// The idle sources implemented on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// GNOME, on Wayland and X11 alike: asks gnome-shell and gnome-session
    Gnome,
    /// X11 sessions on desktops other than GNOME: asks the X11 server
    X11,
}

/// Returns the Linux idle source.
pub(super) fn detect() -> Arc<dyn IdleSource> {
    let Some(source) = choose(|name| std::env::var(name).ok()) else {
        log::info!("This desktop has no idle source yet; the user is treated as always present");
        return Arc::new(Unsupported);
    };
    log::info!("Idle source: {source:?}");
    match source {
        Source::Gnome => Arc::new(gnome::GnomeIdle::new()),
        Source::X11 => match x11::X11Idle::connect() {
            Ok(idle) => Arc::new(idle),
            Err(e) => {
                log::warn!("Failed to connect to the X server; assuming the user is active: {e}");
                Arc::new(Unsupported)
            }
        },
    }
}

/// Picks the source from environment variables. `var` looks up an environment variable by name;
/// an empty value counts as unset.
///
/// None = this desktop has no implementation yet
fn choose(var: impl Fn(&str) -> Option<String>) -> Option<Source> {
    let var = |name: &str| var(name).filter(|v| !v.is_empty());
    // XDG_CURRENT_DESKTOP is a colon-separated list, such as "ubuntu:GNOME" on Ubuntu
    let is_gnome = var("XDG_CURRENT_DESKTOP").is_some_and(|desktops| {
        desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("GNOME"))
    });

    // Wayland sessions also set DISPLAY, for XWayland, so WAYLAND_DISPLAY has to be checked first
    if var("WAYLAND_DISPLAY").is_some() {
        // XWayland only sees input to X11 windows, so only GNOME has a source here
        return is_gnome.then_some(Source::Gnome);
    }
    if var("DISPLAY").is_some() {
        return Some(if is_gnome { Source::Gnome } else { Source::X11 });
    }
    // No graphical display, such as when started over SSH
    None
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

    /// 用给定的环境变量选来源，没列出的当作没有。
    fn chosen(vars: &[(&str, &str)]) -> Option<Source> {
        choose(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn gnome_uses_the_gnome_source_on_wayland_and_x11() {
        // Ubuntu 在前面加了自己的名字；GNOME Classic 也是 GNOME
        for desktop in ["ubuntu:GNOME", "GNOME", "GNOME-Classic:GNOME", "gnome"] {
            assert_eq!(
                chosen(&[
                    ("WAYLAND_DISPLAY", "wayland-0"),
                    ("XDG_CURRENT_DESKTOP", desktop)
                ]),
                Some(Source::Gnome),
                "{desktop}"
            );
            assert_eq!(
                chosen(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", desktop)]),
                Some(Source::Gnome),
                "{desktop}"
            );
        }
    }

    #[test]
    fn other_x11_desktops_use_the_x11_source() {
        // 名字里带 GNOME 的别的桌面（单独出现的 GNOME-Flashback）也不算 GNOME
        for desktop in ["KDE", "XFCE", "i3", "GNOME-Flashback", ""] {
            assert_eq!(
                chosen(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", desktop)]),
                Some(Source::X11),
                "{desktop}"
            );
        }
    }

    #[test]
    fn other_wayland_desktops_have_no_source() {
        // Wayland 会话也设了 DISPLAY（给 XWayland 的），不能因此用 X11 来源
        for desktop in ["KDE", "Hyprland", "sway", "GNOME-Flashback", ""] {
            assert_eq!(
                chosen(&[
                    ("WAYLAND_DISPLAY", "wayland-0"),
                    ("DISPLAY", ":0"),
                    ("XDG_CURRENT_DESKTOP", desktop)
                ]),
                None,
                "{desktop}"
            );
        }
    }

    #[test]
    fn no_graphical_display_has_no_source() {
        // 从 SSH 启动：桌面变量还在，但没有显示可连
        assert_eq!(chosen(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]), None);
        assert_eq!(
            chosen(&[
                ("WAYLAND_DISPLAY", ""),
                ("DISPLAY", ""),
                ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")
            ]),
            None
        );
    }

    #[test]
    fn unsupported_treats_the_user_as_present() {
        // 没有来源时不能误封会话：一直在用、屏幕一直能看、没有防睡请求
        assert_eq!(Unsupported.idle_secs(), 0);
        assert!(!Unsupported.screen_unavailable());
        assert!(!Unsupported.display_keepawake_active());
    }
}
