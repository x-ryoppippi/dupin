import AppKit

final class Delegate: NSObject, NSApplicationDelegate {
    func application(_ sender: NSApplication, openFiles filenames: [String]) {
        let marker = Bundle.main.bundleURL.deletingLastPathComponent().appendingPathComponent("opened.json")
        do {
            try JSONEncoder().encode(filenames).write(to: marker, options: .atomic)
            sender.reply(toOpenOrPrint: .success)
        } catch {
            sender.reply(toOpenOrPrint: .failure)
        }
    }
}

let application = NSApplication.shared
let delegate = Delegate()
application.delegate = delegate
application.run()
