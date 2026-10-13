// Fake user activity for the benchmark: switch apps, type, move the mouse.
//   activity <docs dir>
// Runs until <docs dir>/STOP exists. Keys go only to TextEdit, and only while its
// focused window is one of our hs-bench documents; they are sent to TextEdit's
// pid, so they cannot land in another app. The mouse only moves, never clicks.
import AppKit
import ApplicationServices

let dir = CommandLine.arguments[1]
let docA = dir + "/hs-bench-a.txt", docB = dir + "/hs-bench-b.txt", folder = dir + "/hs-bench-files"
srand48(42)

func stopped() -> Bool { FileManager.default.fileExists(atPath: dir + "/STOP") }

func run(_ args: [String]) {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
    p.arguments = args
    try? p.run()
    p.waitUntilExit()
}

func focusedTitle(_ pid: pid_t) -> String {
    let app = AXUIElementCreateApplication(pid)
    var w: AnyObject?
    guard AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &w) == .success else { return "" }
    var t: AnyObject?
    AXUIElementCopyAttributeValue(w as! AXUIElement, kAXTitleAttribute as CFString, &t)
    return (t as? String) ?? ""
}

let screen = CGDisplayBounds(CGMainDisplayID())
var pos = CGPoint(x: screen.midX, y: screen.midY)
func jiggle() {
    pos.x = min(max(pos.x + CGFloat(drand48() * 80 - 40), screen.minX + 100), screen.maxX - 100)
    pos.y = min(max(pos.y + CGFloat(drand48() * 60 - 30), screen.minY + 100), screen.maxY - 100)
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: pos, mouseButton: .left)?
        .post(tap: .cghidEventTap)
}

/// Waits, moving the mouse twice a second (keeps the idle timer at zero).
func idle(_ secs: Double) {
    let end = Date().addingTimeInterval(secs)
    while Date() < end && !stopped() { jiggle(); usleep(500_000) }
}

let words = """
the quick brown fox jumps over the lazy dog while a small team reviews the weekly plan \
budget numbers look fine but the release date may move by two days so we write notes \
meeting at three about the new design draft and the open questions from last sprint
""".split(separator: " ").map(String.init)
var w = 0, onLine = 0

func key(_ pid: pid_t, code: CGKeyCode, text: String?) {
    for down in [true, false] {
        guard let e = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) else { continue }
        if let text {
            var u = Array(text.utf16)
            e.keyboardSetUnicodeString(stringLength: u.count, unicodeString: &u)
        }
        e.postToPid(pid)
    }
}

/// Types words into TextEdit at about six characters a second.
func type(_ secs: Double) {
    let end = Date().addingTimeInterval(secs)
    while Date() < end && !stopped() {
        guard let app = NSWorkspace.shared.frontmostApplication,
              app.bundleIdentifier == "com.apple.TextEdit",
              focusedTitle(app.processIdentifier).hasPrefix("hs-bench") else { idle(0.5); continue }
        let word = words[w % words.count] + " "
        w += 1
        for ch in word {
            key(app.processIdentifier, code: 0, text: String(ch))
            usleep(UInt32(120_000 + drand48() * 80_000))
        }
        onLine += 1
        if onLine == 12 { key(app.processIdentifier, code: 36, text: nil); onLine = 0 }
        if drand48() < 0.3 { jiggle() }
    }
}

let before = Set(NSWorkspace.shared.runningApplications.compactMap(\.bundleIdentifier))
while !stopped() {
    run(["-a", "TextEdit", docA]); idle(1.5); type(25)
    run([folder]); idle(15)
    run(["-a", "Calculator"]); idle(10)
    run(["-a", "TextEdit", docB]); idle(1.5); type(20)
}
// Quit the apps we started; leave the ones that were already open.
for id in ["com.apple.TextEdit", "com.apple.calculator"] where !before.contains(id) {
    NSRunningApplication.runningApplications(withBundleIdentifier: id).forEach { $0.terminate() }
}
