// The stage scripts/docs-glass.nu records Dupin on:
//
//   stage preflight          exits 0 when this process may capture the screen, without asking
//   stage wallpaper <image>  covers every screen with <image> below all windows until killed
//   stage panel <pid>        prints the frame of <pid>'s largest on-screen window, in pixels
//   stage displays           prints each display's size in points and pixels, and its modes
//   stage windows            prints the on-screen windows, front to back
//   stage scale            prints the main display's backing scale factor
//   stage hidpi              makes a 2x virtual display the main one until killed
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
    }
    let workspace = NSWorkspace.shared
    print("reduceTransparency=\(workspace.accessibilityDisplayShouldReduceTransparency) increaseContrast=\(workspace.accessibilityDisplayShouldIncreaseContrast) reduceMotion=\(workspace.accessibilityDisplayShouldReduceMotion)")
    exit(0)
}

func windows() -> Never {
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else {
        fail("cannot list windows")
    }
    for info in list {
        let owner = info[kCGWindowOwnerName as String] as? String ?? "?"
        let layer = info[kCGWindowLayer as String] as? Int ?? 0
        let alpha = info[kCGWindowAlpha as String] as? Double ?? 0
        let bounds = (info[kCGWindowBounds as String] as? NSDictionary).flatMap { CGRect(dictionaryRepresentation: $0) } ?? .zero
        print("window owner=\(owner) layer=\(layer) alpha=\(alpha) bounds=\(Int(bounds.minX)),\(Int(bounds.minY)) \(Int(bounds.width))x\(Int(bounds.height))")
    }
    exit(0)
}

func scale() -> Never {
    guard let screen = NSScreen.screens.first else { fail("no screen") }
    print(screen.backingScaleFactor)
    exit(0)
}

// A runner's display is a 1x framebuffer with no 2x mode, so this adds a virtual display (the
// private CGVirtualDisplay, which needs no permission), makes it the main one and mirrors the
// others to it. The display lasts as long as this process.
func hidpi() -> Never {
    func make(_ name: String) -> NSObject.Type {
        guard let found = NSClassFromString(name) as? NSObject.Type else { fail("this macOS has no \(name)") }
        return found
    }
    func allocated(_ type: NSObject.Type) -> AnyObject {
        guard let object = (type as AnyObject).perform(NSSelectorFromString("alloc"))?.takeUnretainedValue() else {
            fail("cannot allocate \(type)")
        }
        return object
    }
    let descriptor = make("CGVirtualDisplayDescriptor").init()
    descriptor.setValue("Dupin recording", forKey: "name")
    descriptor.setValue(UInt32(2880), forKey: "maxPixelsWide")
    descriptor.setValue(UInt32(2000), forKey: "maxPixelsHigh")
    descriptor.setValue(NSValue(size: NSSize(width: 286, height: 179)), forKey: "sizeInMillimeters")
    descriptor.setValue(UInt32(0x4455), forKey: "vendorID")
    descriptor.setValue(UInt32(0x0001), forKey: "productID")
    descriptor.setValue(UInt32(1), forKey: "serialNum")
    descriptor.setValue(DispatchQueue.main, forKey: "queue")

    let modeClass = make("CGVirtualDisplayMode")
    let initialiser = class_getMethodImplementation(modeClass, NSSelectorFromString("initWithWidth:height:refreshRate:"))
    typealias ModeInit = @convention(c) (AnyObject, Selector, UInt32, UInt32, Double) -> Unmanaged<AnyObject>
    let mode = unsafeBitCast(initialiser, to: ModeInit.self)(
        allocated(modeClass),NSSelectorFromString("initWithWidth:height:refreshRate:"), 1440, 1000, 60
    ).takeUnretainedValue()
    let settings = make("CGVirtualDisplaySettings").init()
    settings.setValue(UInt32(1), forKey: "hiDPI")
    settings.setValue([mode], forKey: "modes")

    let displayClass = make("CGVirtualDisplay")
    guard let display = allocated(displayClass).perform(NSSelectorFromString("initWithDescriptor:"), with: descriptor)?.takeUnretainedValue(),
          display.perform(NSSelectorFromString("applySettings:"), with: settings) != nil,
          let id = display.value(forKey: "displayID") as? UInt32, id != 0
    else { fail("cannot create a virtual display") }

    var count: UInt32 = 0
    var ids = [CGDirectDisplayID](repeating: 0, count: 16)
    CGGetActiveDisplayList(16, &ids, &count)
    var configuration: CGDisplayConfigRef?
    guard CGBeginDisplayConfiguration(&configuration) == .success else { fail("cannot configure displays") }
    CGConfigureDisplayOrigin(configuration, id, 0, 0)
    for other in ids.prefix(Int(count)) where other != id {
        CGConfigureDisplayMirrorOfDisplay(configuration, other, id)
    }
    guard CGCompleteDisplayConfiguration(configuration, .forSession) == .success else { fail("cannot apply the display configuration") }
    signal(SIGTERM) { _ in exit(0) }
    signal(SIGINT) { _ in exit(0) }
    print("virtual display \(id)")
    fflush(stdout)
    dispatchMain()
}

let arguments = CommandLine.arguments.dropFirst()
switch (arguments.first, arguments.dropFirst().first) {
case ("preflight", nil):
    exit(CGPreflightScreenCaptureAccess() ? 0 : 1)
case ("wallpaper", let path?):
    wallpaper(path)
case ("displays", nil):
    displays()
case ("windows", nil):
    windows()
case ("hidpi", nil):
    hidpi()
case ("scale", nil):
    scale()
case ("panel", let pid?):
    guard let pid = Int32(pid) else { fail("not a process ID: \(pid)") }
    panel(pid)
default:
    fail("usage: stage preflight | displays | scale | hidpi | wallpaper <image> | panel <pid>")
}
