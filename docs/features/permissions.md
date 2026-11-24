# Permissions

Settings → Permissions shows which features use each grant and what macOS has allowed. Choose **Request Access** to ask for a grant for the first time, even while its feature is off. Choose **Open System Settings** to manage an existing decision. Dupin refreshes the status when the Calendar request finishes and when Settings regains focus.

Turning on **Calendar search**, **Window Management** or **Paste snippets**, or recording a **Selection hotkey** for the first time, requests the corresponding access when needed. You can also be prompted on first use if you enabled a feature through `config.toml`. Starting Dupin, opening Settings and reloading configuration only check access. File search uses the Spotlight index and needs no grant.

## Extensions

macOS grants access to Dupin as a whole, but that never lets an extension use it. Each extension also needs your own grant for each thing it declares, such as one website, a folder you pick or reading the clipboard. Dupin keeps these grants itself, separate from macOS:

- An extension you add starts off, with nothing granted. Turning it on lists what it asks for so you can grant each item.
- Dupin asks only when you turn an extension on, choose Request Access, or deliberately use something that needs a grant, never at startup, on opening Settings, on reload or while you type.
- Turning an extension off or revoking a grant takes effect at once: work it was doing is cancelled and its results are dropped. Granting again doesn't bring them back.
- A grant covers the exact version you approved. An update that asks for more asks you again; anything it no longer needs is dropped.
- Extensions that ship with Dupin are on by default with their own grants in place, but still ask macOS for access on first use. A copy you change is treated like any other extension.

An extension that reads calendar events needs Calendars access, and one that reads or watches the clipboard needs Dupin allowed to paste from other apps. Settings → Permissions lists extensions among the uses of Calendars and Paste from Other Apps, and will name each extension that declares them. Turning extensions on never asks macOS for either. Extensions can't use any of this yet; [Capability broker](../capability-broker.md) describes the rules.

## Paste from Other Apps

Settings → Permissions has a **Paste from Other Apps** row, used by clipboard history and by extensions that read the clipboard. On macOS versions that report the pasteboard access policy, clipboard history records in the background only while it is **Always Allow**; earlier versions have no policy, and the row shows Allowed. Request Access reads the clipboard once so macOS asks and lists Dupin in System Settings; turning clipboard history on only checks the policy. If macOS is set to ask or deny, Open System Settings goes to Privacy & Security, where you can always allow Dupin to paste from other apps.
