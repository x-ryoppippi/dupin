// The stage scripts/docs-glass.nu records Dupin on:
//
//   stage preflight          exits 0 when this process may capture the screen, without asking
//   stage wallpaper <image>  covers every screen with <image> below all windows until killed
//   stage panel <pid>        prints the frame of <pid>'s largest on-screen window, in pixels
//
// The wallpaper is a window, so the desktop's own picture is never changed.
import AppKit

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func wallpaper(_ path: String) -> Never {
    guard let image = NSImage(contentsOfFile: path) else { fail("cannot read \(path)") }
    let application = NSApplication.shared
    application.setActivationPolicy(.accessory)
    let windows = NSScreen.screens.map { screen in
        let window = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        // Just above the desktop picture and its icons, below every app's windows.
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        window.ignoresMouseEvents = true
        window.isOpaque = true
        let view = NSImageView(frame: NSRect(origin: .zero, size: screen.frame.size))
        view.image = image
        view.imageScaling = .scaleAxesIndependently
        window.contentView = view
        window.setFrame(screen.frame, display: true)
        window.orderFrontRegardless()
        return window
    }
    _ = windows
    signal(SIGTERM) { _ in exit(0) }
    signal(SIGINT) { _ in exit(0) }
    application.run()
    exit(0)
}

func panel(_ pid: Int32) -> Never {
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
        fail("cannot list windows")
    }
    let frames = list.compactMap { info -> CGRect? in
        guard (info[kCGWindowOwnerPID as String] as? Int32) == pid,
              let bounds = info[kCGWindowBounds as String] as? NSDictionary,
              let rect = CGRect(dictionaryRepresentation: bounds)
        else { return nil }
        return rect
    }
    guard let frame = frames.max(by: { $0.width * $0.height < $1.width * $1.height }) else {
        fail("no window of \(pid) is on screen")
    }
    // Window bounds are in points from the main screen's top left; recordings are in its pixels.
    // `bottom` is where the Dock or the screen's edge begins, below which nothing is wallpaper.
    guard let screen = NSScreen.screens.first else { fail("no screen") }
    let scale = screen.backingScaleFactor
    let bottom = screen.frame.maxY - screen.visibleFrame.minY
    let pixels = [frame.minX, frame.minY, frame.width, frame.height, bottom].map { Int(($0 * scale).rounded()) }
    print("{\"x\":\(pixels[0]),\"y\":\(pixels[1]),\"width\":\(pixels[2]),\"height\":\(pixels[3]),\"bottom\":\(pixels[4]),\"scale\":\(scale)}")
    exit(0)
}

let arguments = CommandLine.arguments.dropFirst()
switch (arguments.first, arguments.dropFirst().first) {
case ("preflight", nil):
    exit(CGPreflightScreenCaptureAccess() ? 0 : 1)
case ("wallpaper", let path?):
    wallpaper(path)
case ("panel", let pid?):
    guard let pid = Int32(pid) else { fail("not a process ID: \(pid)") }
    panel(pid)
default:
    fail("usage: stage preflight | stage wallpaper <image> | stage panel <pid>")
}
