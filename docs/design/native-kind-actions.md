# Native kind actions: what moves

## Problem and goals

The owner wants a minimal core: anything that can be an extension should be one, so built-in features ship as bundled extensions and users and agents extend actions the same way. Packages already add kind actions (`provides.kinds.<kind>`, an `actions` export, gated by `rows:<kind>` and composed through `[advanced]`; [Kind](./extension-platform.md#architecture), rollout step 9). This audit lists every action the native `text`, `url`, `file` and `app` kinds offer, and the other built-ins that are still native, and decides for each whether it moves to a bundled package, stays native, or needs a host service first. It also names the platform gaps that block several moves at once and proposes one mechanism for each.

Two mechanisms are being built for [#381](https://github.com/x-ryoppippi/dupin/issues/381) (Quit, Hide and Force Quit as a bundled `running-apps` package): a bundled package composing its kind actions by default, and actions declaring their availability. This design depends on both and does not specify them again; [Platform gaps](#platform-gaps) only records what the moves below need from them.

## Criteria

From [Built-ins as bundled extensions](./extension-platform.md#built-ins-as-bundled-extensions):

- **Move** when the action can be a bundled package's kind action with capabilities the broker already has, and the guest owns some of the work.
- **Stay native** when the action needs native UI, timing or privileges a sandboxed guest can't reasonably get, or when, behind a broker service, the package would hold only the action's name and forward the row. The second test is the one System commands applied: such a port moves no work out of the host and adds a capability.
- **Needs a host service first** when a narrow service would let the guest own the action's policy and the service is useful to third-party kind actions too.

Two facts about how a kind action runs decide most verdicts:

- A kind action runs in a helper of its own, as every action does, so each run pays a helper launch and compile: 12.0 ms at p95 on a Blacksmith M4 runner (rollout step 3), against a cold gate of 20 ms. That is invisible for an operation that already reports its outcome in a notice, and too slow for a key the user presses to see something at once.
- A kind action receives one field as a string (`{ path }`, `{ name }`, `{ text }` or `{ url }`). Nothing a guest can call accepts that path as the row's host-validated file; only `exec` can act on it, as a program argument.

## Verdicts

| Action                                                                 | Kind / place         | Verdict                    | Reason                                                                                                                                                                                                | Prerequisite                                                                       |
| ---------------------------------------------------------------------- | -------------------- | -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Open (<kbd>↩</kbd>)                                                    | file, app            | Stay native                | The default route: Enter can't wait on a helper launch, and the guest would only forward the path to `NSWorkspace`.                                                                                   | —                                                                                  |
| Reveal in Finder (<kbd>⌘↩</kbd>)                                       | file, app            | Stay native                | The alternate route, with the same latency and nothing for a guest to decide.                                                                                                                         | —                                                                                  |
| Quick Look (<kbd>⌘Y</kbd>)                                             | file, app            | Stay native                | See [Quick Look](#quick-look): the shortcut is bound to the action's ID, the panel must be shown while the launcher is key, and a `quicklook` service would leave the guest only the path to forward. | —                                                                                  |
| Browse                                                                 | file (folders)       | Stay native                | Navigation into the native `folder` source, whose rows are host-validated `file` rows.                                                                                                                | —                                                                                  |
| Open with…                                                             | file                 | Stay native                | A page listing the file's LaunchServices handlers, then `OpenWith`; both are native, and the guest would hold only the title.                                                                         | —                                                                                  |
| Copy name                                                              | file                 | **Move**                   | `clipboard:write` and `rows:file` with `name` are enough, and the package decides what it copies.                                                                                                     | Default composition with a position, availability (#381)                           |
| Rename…                                                                | file                 | Stay native                | Edits the name in the query field with its stem selected, validates it by APFS rules and replaces the row in place: session-level UI and file-system safety the guest would only label.               | —                                                                                  |
| Move to…, Copy to…                                                     | file                 | Stay native                | A native Spotlight folder search picks the destination, and the cross-volume move (copy, verify, then trash the original) is safety logic that belongs to the host.                                   | —                                                                                  |
| Duplicate                                                              | file                 | Needs a host service first | Finder-style naming and the outcome notice can be the package's; the copy needs a write on the row's file, which no service offers.                                                                   | Row file handles; `fileActions: ["duplicate"]`                                     |
| Compress                                                               | file                 | Needs a host service first | As Duplicate, with `ditto` run by the host. `exec` with `/usr/bin/ditto` would take a path string and race on a name that is not taken.                                                               | Row file handles; `fileActions: ["compress"]`                                      |
| Move to Trash                                                          | file                 | Needs a host service first | Recoverable, so the package can own it once the host confirms first and removes the row afterwards.                                                                                                   | Row file handles; `fileActions: ["trash"]`; kind-action confirmations; row effects |
| Quit, Hide, Force Quit                                                 | app                  | **Move**                   | Being built as the bundled `running-apps` package (#381).                                                                                                                                             | #381                                                                               |
| Copy (<kbd>↩</kbd>)                                                    | text                 | Stay native                | The text kind's default route, also taken by extension rows' text, which by the cross-producer rule no package's kind action may receive.                                                             | —                                                                                  |
| Open URL (<kbd>↩</kbd>)                                                | url                  | Stay native                | The URL kind's default route, also taken by extension rows' URLs, as Copy is.                                                                                                                         | —                                                                                  |
| Send to… → Copy                                                        | text payload         | Stay native                | It serves the payloads of extension rows (dictionary, calculator), which no package's kind action may receive.                                                                                        | —                                                                                  |
| Send to… → Share…, AirDrop, New Mail with Attachment                   | file and app payload | Stay native                | `NSSharingServicePicker` is anchored to the launcher's view and holds focus while it is open (`dismiss_if_unfocused`).                                                                                | —                                                                                  |
| Send to… → Copy path                                                   | file and app payload | **Move**                   | `clipboard:write` and `rows:file` with `path` are enough.                                                                                                                                             | Send to… contributions; default composition                                        |
| Send to… → web searches                                                | text payload         | **Move** (done)            | The bundled `web-search` package already supplies them as data that Dupin fills natively.                                                                                                             | —                                                                                  |
| Sleep, Lock Screen, Sleep Displays, Toggle Dark Mode                   | system commands      | Stay native                | Already decided under [Built-ins](./extension-platform.md#built-ins-as-bundled-extensions): a package would hold only their names.                                                                    | —                                                                                  |
| Restart, Shut Down, Log Out, Empty Trash                               | system commands      | Stay native                | The same, and a port would add a capability to restart or log out of the Mac that no package needs.                                                                                                   | —                                                                                  |
| Dupin Settings, Snapshots (open, resume), Restart Dupin, Update Dupin  | Dupin's own rows     | Stay native                | They drive Dupin itself: its windows, its saved results and its own process.                                                                                                                          | —                                                                                  |
| Run a script command, its arguments, configured `script` actions       | script commands      | Stay native                | The trusted runner is host infrastructure that `exec` and `@script` targets broker for packages.                                                                                                      | —                                                                                  |
| Allow, Deny, Request Access, Turn On, Review; Request Clipboard Access | consent rows         | Stay native                | The broker's own consent UI: a package can't grant itself a scope.                                                                                                                                    | —                                                                                  |
| Keyword and alias rows                                                 | command registry     | Stay native                | The registry decides routing and conflicts for every command, a package's included.                                                                                                                   | —                                                                                  |
| Selected-text search, hotkeys, Settings search, taking a snapshot      | host infrastructure  | Stay native                | Already decided: each drives Dupin itself.                                                                                                                                                            | —                                                                                  |
| Recent items, folder browsing                                          | native sources       | Stay native                | Already decided: usage history and host-validated `file` rows.                                                                                                                                        | —                                                                                  |

Of 26 rows, 4 move (one done, one in flight), 3 need a host service first and 19 stay native. Dupin has no drag-and-drop today; one would be native, since a drag session belongs to AppKit.

[`[[advanced.kinds.<kind>.actions]]`](../features/advanced.md#kind-actions) script and URL actions are already composition, not built-ins, so they are not listed.

### Quick Look

The preview panel is `QLPreviewPanel`, so it is native in any design. The question is whether the action could be a kind action calling a `quicklook` service with the row's file. It stays native:

- **Latency.** <kbd>⌘Y</kbd> runs the action whose ID is `quick-look` (`ui/keys.rs`) and the user expects the preview at once, as Space does in Finder. A kind action pays a helper launch and compile on every press, about 12 ms at p95 and up to the 20 ms gate, before the service is even asked.
- **Focus.** `Previewer::show` must run while the launcher's view is key and in the responder chain, or Quick Look adopts no controller and the panel closes again; `dismiss_if_unfocused` keeps the launcher open only because it can ask whether the preview owns focus. An answer arriving a helper launch later can find the launcher dismissed, another row selected or another window key, and the failure is either no preview or a preview that dismisses the launcher.
- **Nothing moves.** The package would receive the path and hand it back. That is the System commands case: a capability and a round trip for no work out of the host.

What a user would want from a port, hiding the action or putting it elsewhere in the list, comes from [native actions in `[advanced]`](#native-actions-in-advanced) without one.

## Platform gaps

Each gap is generic: it blocks more than one move, or makes stay-native actions as composable as moved ones.

### Default composition with a position (#381)

#381 lets a bundled package compose its kind actions by default. Copy name sits between Quick Look and Rename…, and Copy path in Send to…, so a default composition needs a place relative to the kind's own actions, not only after them: an anchor such as `after = "quick-look"`, naming a native action's ID, which the parity tests for the file kind's order (`files_offer_ordered_choices_and_confirmed_trash` in `dupin-core/src/kind.rs`) then hold.

### Availability (#381)

#381 lets an action declare when it is available. Rows are resolved on every keystroke, so availability must be decided by the host from what it already knows of the subject, never by a guest call per row. The moves above need: a file or a folder, a name that is UTF-8 (Rename…, Move to…, Copy to…, Duplicate and Compress hide without one today), and, for #381, whether an app is running.

### Native actions in `[advanced]`

**Done.** `[advanced.kinds.file]` and `[advanced.kinds.app]` take `hide` and `order`, lists of action IDs, validated like the rest of `[advanced]`; [Hide and order](../features/advanced.md#hide-and-order) lists every ID (`kind::FILE_ACTIONS` and `kind::APP_ACTIONS`, held to each kind's list by a test). The one rule for every action a kind lists: `order` puts the actions it names first, in its order, and the rest follow in their default place; `hide` takes the actions it names out of the action screen. A package's kind action is named `<package>/<action>`, whether `[[actions]]` composes it or #381's default composition does, so the same two keys place and hide it; a `script` or `url` action has no ID and keeps its place in `actions`. Hiding never disables: Enter, ⌘Enter and ⌘Y still run `open`, `reveal` and `quick-look`, because a route or shortcut is an explicit binding the user pressed, and refusing to hide the Enter route or moving Enter elsewhere would make a list setting change what a key does. `text` and `url` take neither key: their rows come from extensions and list the extension's actions. Send to… stays last, and its receivers are not covered: they belong to the payload, not the kind, and come with [Send to… contributions](#send-to-contributions).

### Row file handles

A kind action receives the row's path as text, which no service accepts as the row's file. Add a `handle` field to `rows:file` and `rows:app`: an opaque token the host binds to the row's host-validated path for the one activation that runs the action, as clipboard files already carry a `handle`. Services that act on a row's file take the handle, never a path, so a guest can act only on the file the user chose, and only while that run lasts.

### A `fileActions` service

With handles, a `fileActions` capability offers the operations a moved action needs, each a scope of its own, each needing an activation and each refusing to replace an existing item: `duplicate` and `compress` (beside the original, under a Finder-style name the package may suggest and the host makes unique atomically) and `trash`. The host runs them on its file worker, as today, and returns the new item's name. Third-party kind actions get the same operations, instead of an `exec` profile over a path string.

### Kind-action confirmations

Move to Trash opens a host-drawn confirmation with Cancel selected, which a held <kbd>↩</kbd> can't confirm. A guest kind action has no confirmation before it runs. Let `provides.kinds.<kind>.<name>` declare `confirm: { prompt, button }`, with `{title}` filled in by the host, shown by the host before the action is authorized, so the guest never runs on an unconfirmed choice.

### Row effects

After Move to Trash or Move to… the row leaves the list, and after Rename… it is replaced; a kind action's effects reach only the package's own pages. Add `removeRow` and `refreshRow` effects that apply only to the row the action ran on: Dupin removes it, or resolves it again from its own source, so the package never supplies a row.

### Send to… contributions

Send to… is built from each kind's `send_to` and the web searches, and no package can add a receiver. Let a kind action declare `sendTo: true`, so composing it also lists it in Send to… for a payload of that kind.

### Undo

No native file action can be undone today; Move to Trash relies on the Trash. Undo is therefore not needed for parity, but when it comes it belongs in the host: each `fileActions` operation returns an undo the host can run (put the item back, delete the copy), offered in its notice, so no package implements its own inverse.

## Migration order

Smallest and safest first. Each PR keeps the action list, routes and ranking unchanged unless it says otherwise.

1. **Copy name** to a bundled `file-actions` package, once #381's default composition and availability land, with the position anchor. It proves the path from a native kind action to a bundled one against the existing order tests, with only `clipboard:write` and `rows:file`.
2. **Native actions in `[advanced]`** (done): `hide` and `order` over native IDs. No guest code; it makes every stay-native action composable, Quick Look included.
3. **Send to… contributions**, then **Copy path** into `file-actions`.
4. **Row file handles and `fileActions` with `compress`**, then **Compress** into `file-actions`. Compress creates a new item and replaces nothing, so it is the safest write.
5. **`duplicate`**, then **Duplicate**.
6. **Kind-action confirmations and row effects**, then **`trash`** and **Move to Trash**.
7. **Undo** in `fileActions`, when the owner wants it.

[Extension platform](./extension-platform.md) · [File actions](../features/file-actions.md) · [Send to](../features/send-to.md) · [Advanced composition](../features/advanced.md)
