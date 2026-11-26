// The stage scripts/docs-glass.nu records Dupin on:
//
//   stage preflight          exits 0 when this process may capture the screen, without asking
//   stage wallpaper <image>  covers every screen with <image> below all windows until killed
//   stage panel <pid>        prints the frame of <pid>'s largest on-screen window, in pixels
//   stage displays           prints each display's size in points and pixels, and its modes
//   stage hidpi              switches the main display to its largest 2x mode, if it has one
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

func modes(_ display: CGDirectDisplayID) -> [CGDisplayMode] {
    // Scaled modes are listed only when asked for, as duplicates of the low-resolution ones.
    let options = [kCGDisplayShowDuplicateLowResolutionModes: true] as CFDictionary
    return (CGDisplayCopyAllDisplayModes(display, options) as? [CGDisplayMode] ?? []).filter { $0.isUsableForDesktopGUI() }
}

func isHiDPI(_ mode: CGDisplayMode) -> Bool {
    mode.pixelWidth == 2 * mode.width && mode.pixelHeight == 2 * mode.height
}

func displays() -> Never {
    var count: UInt32 = 0
    var ids = [CGDirectDisplayID](repeating: 0, count: 16)
    CGGetActiveDisplayList(16, &ids, &count)
    for id in ids.prefix(Int(count)) {
        let current = CGDisplayCopyDisplayMode(id)
        print("display \(id) main=\(CGDisplayIsMain(id) != 0) builtin=\(CGDisplayIsBuiltin(id) != 0) points=\(CGDisplayPixelsWide(id))x\(CGDisplayPixelsHigh(id)) current=\(current?.width ?? 0)x\(current?.height ?? 0) pixels \(current?.pixelWidth ?? 0)x\(current?.pixelHeight ?? 0)")
        for mode in modes(id) {
            print("  mode \(mode.width)x\(mode.height) pixels \(mode.pixelWidth)x\(mode.pixelHeight) \(mode.refreshRate)Hz hidpi=\(isHiDPI(mode))")
        }
    }
    exit(0)
}

func hidpi() -> Never {
    let display = CGMainDisplayID()
    guard let mode = modes(display).filter(isHiDPI).max(by: { $0.width * $0.height < $1.width * $1.height }) else {
        fail("display \(display) has no 2x mode")
    }
    var configuration: CGDisplayConfigRef?
    guard CGBeginDisplayConfiguration(&configuration) == .success,
          CGConfigureDisplayWithDisplayMode(configuration, display, mode, nil) == .success,
          CGCompleteDisplayConfiguration(configuration, .forSession) == .success
    else { fail("cannot switch display \(display) to \(mode.width)x\(mode.height) at 2x") }
    print("\(mode.width)x\(mode.height) at 2x")
    exit(0)
}

let arguments = CommandLine.arguments.dropFirst()
switch (arguments.first, arguments.dropFirst().first) {
case ("preflight", nil):
    exit(CGPreflightScreenCaptureAccess() ? 0 : 1)
case ("wallpaper", let path?):
    wallpaper(path)
case ("displays", nil):
    displays()
case ("hidpi", nil):
    hidpi()
case ("panel", let pid?):
    guard let pid = Int32(pid) else { fail("not a process ID: \(pid)") }
    panel(pid)
default:
    fail("usage: stage preflight | displays | hidpi | wallpaper <image> | panel <pid>")
}
