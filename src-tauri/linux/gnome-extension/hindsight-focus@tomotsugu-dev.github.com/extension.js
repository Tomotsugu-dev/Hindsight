// Hindsight Focus：告诉 Hindsight 现在哪个窗口有焦点，并在它要求时截下这个窗口。
//
// GNOME 在 Wayland 下不让普通程序读焦点窗口（org.gnome.Shell.Introspect 只回答
// xdg-desktop-portal）。扩展跑在 gnome-shell 进程里，看得到焦点窗口，再通过 session
// bus 转告 Hindsight。
//
// 只回答 Hindsight：调用方的可执行文件必须叫 hindsight，并且属于 root（deb / rpm
// 安装的位置）。不检查的话，同一会话里的任何程序都能借它读窗口标题、截焦点窗口，等于把 GNOME
// 关上的口子又打开了。扩展装在用户目录里时（开发者自己放进去的），不检查属主，
// 好让自己编译的 Hindsight 也能用。
//
// 这里的代码都跑在 gnome-shell 里，Wayland 下 gnome-shell 崩溃会结束整个登录会话：
// 不要做阻塞调用，回调里的异常要兜住。
//
// D-Bus 接口是和 src-tauri/src/capture/focus/linux/gnome.rs、
// src-tauri/src/capture/screenshot/linux/gnome.rs 的约定。Wayland 下扩展更新后要
// 重新登录才生效，所以 GetFocusedWindow 只加字段、不改不删；新方法可以加。
// Hindsight 需要扩展的新行为时，VERSION 加一。VERSION 2 起有 CaptureFocusedWindow。

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'com.tomotsugu.hindsight.Focus';
const OBJECT_PATH = '/com/tomotsugu/hindsight/Focus';
const INTERFACE_NAME = 'com.tomotsugu.hindsight.Focus';
const VERSION = 2;
/** 调用方可执行文件的文件名 */
const CALLER_NAME = 'hindsight';
/** 没有截到窗口时返回的空 PNG */
const EMPTY_PNG = new Uint8Array(0);

const INTERFACE_XML = `<node>
  <interface name="${INTERFACE_NAME}">
    <!-- 焦点窗口：title (s)、wm-class (s)、app-id (s)、pid (u)。没有焦点窗口时为空 -->
    <method name="GetFocusedWindow">
      <arg type="a{sv}" direction="out" name="window"/>
    </method>
    <!-- 焦点窗口的 PNG 和 pid。没有焦点窗口或截不到时，png 为空、pid 为 0 -->
    <method name="CaptureFocusedWindow">
      <arg type="ay" direction="out" name="png"/>
      <arg type="u" direction="out" name="pid"/>
    </method>
    <!-- 焦点窗口或它的标题变了。只发给验证过的调用方，不带数据 -->
    <signal name="FocusChanged"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

class FocusService {
    /** @param {boolean} trustUserBuilds 调用方不必属于 root */
    constructor(trustUserBuilds) {
        this._trustUserBuilds = trustUserBuilds;
        /** 验证过的调用方：unique name → 监视它断开的 id */
        this._callers = new Map();
        /** 拒绝过的调用方，免得反复检查、反复记日志 */
        this._refused = new Set();
        this._window = null;
        this._destroyed = false;

        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE_XML, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._focusId = global.display.connect(
            'notify::focus-window', () => this._onFocusChanged());
        this._track(global.display.focus_window);
    }

    destroy() {
        this._destroyed = true;
        global.display.disconnect(this._focusId);
        this._untrack();
        for (const watchId of this._callers.values())
            Gio.bus_unwatch_name(watchId);
        this._callers.clear();
        this._refused.clear();
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
    }

    get Version() {
        return VERSION;
    }

    CaptureFocusedWindowAsync(_params, invocation) {
        this._checkCaller(invocation.get_sender(), allowed => {
            if (!allowed) {
                try {
                    invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
                        'Only Hindsight may capture the focused window');
                } catch (e) {
                    console.warn(`Hindsight Focus: cannot refuse the caller: ${e.message}`);
                }
                return;
            }
            this._captureFocusedWindow(invocation);
        });
    }

    /**
     * 截当前焦点窗口。Shell.Screenshot 一次只允许一个截图进行，所以每次 new 一个。
     * 带窗口边框、不带光标。截图是异步的，回调里才能读结果。
     */
    _captureFocusedWindow(invocation) {
        try {
            // screenshot_window 在调用时就截下焦点窗口，回调只是等 PNG 编码完。
            // 所以调用前读到的 pid 就是被截的窗口。
            const pid = this._focusedPid();
            if (pid === null) {
                this._replyCapture(invocation, EMPTY_PNG, 0);
                return;
            }
            const stream = Gio.MemoryOutputStream.new_resizable();
            const shooter = new Shell.Screenshot();
            // gnome-shell 用 Gio._promisify 包过 screenshot_window。带上回调时参数个数
            // 和原函数一致，走原来的回调，不会变成没人等的 Promise。
            shooter.screenshot_window(true, false, stream, (_obj, result) => {
                this._finishCapture(invocation, shooter, stream, result, pid);
            });
        } catch (e) {
            console.warn(`Hindsight Focus: cannot capture the focused window: ${e.message}`);
            this._replyCapture(invocation, EMPTY_PNG, 0);
        }
    }

    _finishCapture(invocation, shooter, stream, result, pid) {
        try {
            shooter.screenshot_window_finish(result);
            stream.close(null);
            this._replyCapture(invocation, stream.steal_as_bytes().toArray(), pid);
        } catch (e) {
            console.warn(`Hindsight Focus: cannot capture the focused window: ${e.message}`);
            this._replyCapture(invocation, EMPTY_PNG, 0);
        }
    }

    /** 没有焦点窗口时是 null，否则是 pid（未知算 0） */
    _focusedPid() {
        const window = global.display.focus_window;
        if (!window)
            return null;
        return Math.max(window.get_pid(), 0);
    }

    _replyCapture(invocation, png, pid) {
        try {
            invocation.return_value(new GLib.Variant('(ayu)', [png, pid]));
        } catch (e) {
            console.warn(`Hindsight Focus: cannot return the screenshot: ${e.message}`);
            try {
                invocation.return_value(new GLib.Variant('(ayu)', [EMPTY_PNG, 0]));
            } catch (again) {
                console.warn(`Hindsight Focus: cannot return an empty screenshot: ${again.message}`);
            }
        }
    }

    GetFocusedWindowAsync(_params, invocation) {
        this._checkCaller(invocation.get_sender(), allowed => {
            try {
                if (allowed) {
                    invocation.return_value(new GLib.Variant('(a{sv})', [this._focusedWindow()]));
                } else {
                    invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
                        'Only Hindsight may read the focused window');
                }
            } catch (e) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', `${e}`);
            }
        });
    }

    _focusedWindow() {
        const window = global.display.focus_window;
        if (!window)
            return {};
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        return {
            'title': new GLib.Variant('s', window.get_title() ?? ''),
            'wm-class': new GLib.Variant('s', window.get_wm_class() ?? ''),
            'app-id': new GLib.Variant('s', app?.get_id() ?? ''),
            'pid': new GLib.Variant('u', Math.max(window.get_pid(), 0)),
        };
    }

    /** 问总线调用方的 PID，再看它的可执行文件。结果通过 callback(allowed) 返回 */
    _checkCaller(sender, callback) {
        if (this._callers.has(sender)) {
            callback(true);
            return;
        }
        if (this._refused.has(sender)) {
            callback(false);
            return;
        }
        Gio.DBus.session.call(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [sender]),
            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, -1, null,
            (connection, result) => {
                let allowed = false;
                try {
                    const [pid] = connection.call_finish(result).deepUnpack();
                    allowed = this._isHindsight(pid);
                } catch (e) {
                    console.warn(`Hindsight Focus: cannot check caller ${sender}: ${e.message}`);
                }
                if (!this._destroyed)
                    this._remember(sender, allowed);
                callback(allowed);
            });
    }

    _isHindsight(pid) {
        // 程序升级后还在跑的旧进程，链接末尾会带 " (deleted)"
        const path = GLib.file_read_link(`/proc/${pid}/exe`).replace(/ \(deleted\)$/, '');
        if (GLib.path_get_basename(path) !== CALLER_NAME) {
            console.warn(`Hindsight Focus: refused ${path}: not Hindsight`);
            return false;
        }
        if (this._trustUserBuilds)
            return true;
        const info = Gio.File.new_for_path(path).query_info(
            'unix::uid', Gio.FileQueryInfoFlags.NONE, null);
        if (info.get_attribute_uint32('unix::uid') !== 0) {
            console.warn(`Hindsight Focus: refused ${path}: not installed by the system`);
            return false;
        }
        return true;
    }

    _remember(sender, allowed) {
        if (!allowed) {
            this._refused.add(sender);
            return;
        }
        if (this._callers.has(sender))
            return;
        // 调用方断开后忘掉它：unique name 不会复用，留着只会让信号白发
        const watchId = Gio.bus_watch_name_on_connection(
            Gio.DBus.session, sender, Gio.BusNameWatcherFlags.NONE, null,
            () => this._forget(sender));
        this._callers.set(sender, watchId);
    }

    _forget(sender) {
        const watchId = this._callers.get(sender);
        if (watchId === undefined)
            return;
        this._callers.delete(sender);
        Gio.bus_unwatch_name(watchId);
    }

    _onFocusChanged() {
        this._track(global.display.focus_window);
        this._emitChanged();
    }

    /** 监听焦点窗口的标题：浏览器切标签页时窗口不变，只有标题变 */
    _track(window) {
        this._untrack();
        if (!window)
            return;
        this._window = window;
        this._titleId = window.connect('notify::title', () => this._emitChanged());
        this._unmanagedId = window.connect('unmanaged', () => this._untrack());
    }

    _untrack() {
        if (!this._window)
            return;
        this._window.disconnect(this._titleId);
        this._window.disconnect(this._unmanagedId);
        this._window = null;
    }

    _emitChanged() {
        for (const sender of this._callers.keys()) {
            try {
                Gio.DBus.session.emit_signal(
                    sender, OBJECT_PATH, INTERFACE_NAME, 'FocusChanged', null);
            } catch (e) {
                console.warn(`Hindsight Focus: cannot signal ${sender}: ${e.message}`);
            }
        }
    }
}

export default class HindsightFocusExtension extends Extension {
    enable() {
        const home = `${GLib.get_home_dir()}/`;
        this._service = new FocusService(this.path.startsWith(home));
    }

    disable() {
        this._service?.destroy();
        this._service = null;
    }
}
