// Accessibility helper for the benchmark.
//   axctl dump <pid>    list windows and tray items (read only)
//   axctl close <pid>   press the close button of the app's first window
//   axctl quit <pid>    open the app's tray menu and press its quit item
import AppKit
import ApplicationServices

func attr(_ el: AXUIElement, _ name: String) -> AnyObject? {
    var v: AnyObject?
    return AXUIElementCopyAttributeValue(el, name as CFString, &v) == .success ? v : nil
}
func children(_ el: AXUIElement, _ name: String = kAXChildrenAttribute) -> [AXUIElement] {
    (attr(el, name) as? [AXUIElement]) ?? []
}
func str(_ el: AXUIElement, _ name: String) -> String { (attr(el, name) as? String) ?? "" }
func press(_ el: AXUIElement) -> Bool { AXUIElementPerformAction(el, kAXPressAction as CFString) == .success }

let args = CommandLine.arguments
guard args.count == 3, let pid = pid_t(args[2]) else {
    FileHandle.standardError.write("usage: axctl dump|close|quit <pid>\n".data(using: .utf8)!)
    exit(2)
}
let app = AXUIElementCreateApplication(pid)
let windows = children(app, kAXWindowsAttribute)
let trayItems = attr(app, "AXExtrasMenuBar").map { children($0 as! AXUIElement) } ?? []

switch args[1] {
case "dump":
    print("trusted=\(AXIsProcessTrusted()) windows=\(windows.count) tray=\(trayItems.count)")
    for w in windows {
        print("window title=\(str(w, kAXTitleAttribute)) close=\(attr(w, kAXCloseButtonAttribute) != nil)")
    }
    for t in trayItems {
        print("tray title=\(str(t, kAXTitleAttribute)) desc=\(str(t, kAXDescriptionAttribute))")
    }
case "close":
    guard let w = windows.first, let b = attr(w, kAXCloseButtonAttribute) else { print("no window"); exit(1) }
    print(press(b as! AXUIElement) ? "closed" : "press failed")
case "quit":
    guard let t = trayItems.first else { print("no tray item"); exit(1) }
    _ = press(t)
    usleep(500_000)
    // The open menu hangs under the tray item: item → AXMenu → AXMenuItem.
    let items = children(t).flatMap { children($0) }
    let names = items.map { str($0, kAXTitleAttribute) }
    print("menu: \(names)")
    let quitWords = ["退出", "結束", "Quit", "Exit", "終了", "Salir"]
    guard let q = items.first(where: { i in quitWords.contains { str(i, kAXTitleAttribute).contains($0) } }) else {
        // Leave the menu closed again before giving up.
        let esc = CGEvent(keyboardEventSource: nil, virtualKey: 53, keyDown: true)
        esc?.post(tap: .cghidEventTap)
        CGEvent(keyboardEventSource: nil, virtualKey: 53, keyDown: false)?.post(tap: .cghidEventTap)
        print("no quit item"); exit(1)
    }
    print(press(q) ? "quit pressed" : "press failed")
default:
    exit(2)
}
