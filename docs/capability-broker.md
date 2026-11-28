# Capability broker

`dupin-broker` is the authority model every extension host service goes through (#12, #96, #218). It decides whether an installed package may perform one operation, remembers the user's decisions, and invalidates work when authority is taken away. It performs no OS, network, file or process operations itself and never prompts. `dupin-extension-bridge` connects it to guest JavaScript, the host services and Settings; see [The broker bridge](#the-broker-bridge).

It replaces `dupin-plugin-policy`, which had the right invalidation semantics but the wrong shape: an in-memory registry of two unscoped capabilities keyed by opaque handles, with nothing persisted and no notion of a package revision. Its rules carry over: a new installation starts off and ungranted, enablement and grants are separate, OS access never grants extension authority, denied and restricted OS states don't prompt again, granting again never revives old permits, removed declarations lose their grants and uninstalling forgets everything.

## Declarations

A package declares the most it may do in `dupin.capabilities` of its `package.json`. `Capabilities::parse` validates it strictly: unknown capabilities and fields, wrong types, values out of range and duplicates are errors, every problem is reported at once, and any problem rejects the declaration. Each diagnostic has the extension runtime's `code`, `field`, `message` and optional `hint`.

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET", "POST"] }],
	"files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
	"exec": {
		"git-status": { "args": ["status", "--short", "{path}"], "output": true },
		"@script:open-in-zed": {}
	},
	"storage": { "quotaBytes": 1048576, "retentionDays": 30 },
	"clipboard": ["read", "write", "watch", "paste"],
	"dictionary": ["lookup"],
	"calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 },
	"windows": ["read", "arrange"],
	"context": ["frontmost", "document"]
}
```

| Capability   | Declares                                                                                                                                                                                                                                         | One scope per     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------- |
| `http`       | Up to 32 canonical `https://host[:port]` origins (lowercase, no path, no `:443`) with their methods: GET, HEAD, POST, PUT, PATCH or DELETE.                                                                                                      | origin            |
| `files`      | Up to 16 named scopes, each a `file` or `directory` with `read` and, for a directory, `list` access. Writes are rejected until they are designed.                                                                                                | name              |
| `exec`       | Up to 16 named profiles, each an argument template whose `{input}` placeholders the exec service fills, or `@script:<name>` targets for the user's script commands. `output` asks to return output to the guest, up to `outputBytes` per stream. | profile or script |
| `storage`    | A quota of 1 byte to 64 MiB and an optional retention of 1–3,650 days.                                                                                                                                                                           | the capability    |
| `clipboard`  | `read`, `write`, `watch` and `paste`, which lets Dupin carry out the package's `paste` row actions.                                                                                                                                              | access            |
| `dictionary` | `lookup`.                                                                                                                                                                                                                                        | the capability    |
| `calculator` | `evaluate`: arithmetic, units and, while currency conversion is on, currencies at the rates Dupin keeps.                                                                                                                                         | the capability    |
| `snippets`   | `write`: adding, changing and deleting the snippets the user stores in Dupin, which needs an activation. Reading them is not a scope; only the package Dupin ships for Snippets is handed them.                                                  | the capability    |
| `calendar`   | `read` within `pastDays` and `futureDays` of today, each 0–366.                                                                                                                                                                                  | the capability    |
| `windows`    | `read`, the displays and the focused window, and `arrange`, moving and resizing it, which needs an activation.                                                                                                                                   | access            |
| `context`    | `frontmost`, the app in front when the panel opened, and `document`, the file its focused window showed. Neither needs an activation or macOS access of its own.                                                                                 | access            |
| `apps`       | `read`, the apps that run now with a place in the Dock, and `control`, hiding, quitting and force quitting one of them, which needs an activation. Neither needs macOS access.                                                                   | access            |
| `rows`       | For each kind of row Dupin's own sources find (`text`, `url`, `file`, `app`), the fields the package's filters and kind actions may receive: `title`, and `text`, `url`, `name` or `path`.                                                       | kind              |
| `query`      | `read`: what is typed beyond the package's keyword: in the searches its matchers are composed into, and the main search for the built-in it answers there (Calculator).                                                                          | the capability    |

A scope is written as `http:https://api.github.com`, `files:notes`, `exec:git-status`, `script:open-in-zed`, `storage`, `clipboard:read`, `dictionary`, `calculator`, `snippets`, `calendar`, `windows:arrange`, `context:document`, `apps:control`, `rows:file` or `query` in grant files.

## Checking an operation

A host service describes the operation as a `Request` and calls `Broker::check(package, revision, request, trigger)`. The trigger is set by the host from input it received: `Query` while the user types, `Background` for work nobody asked for just now, and `Activation` for a deliberate action. The answer is one of:

- **Allow**, with a `Permit` carrying the declared scope, its bounds and, for files and exec, the path the scope is bound to: the one the user picked, or the one the manifest names (`path` or `program`), which the bridge resolves and binds when the user allows the scope.
- **Deny**, with a reason that renders as a user-facing sentence: not installed, a stale revision, unreadable grants, turned off, an invalid request (such as a path with `..`), an undeclared scope, beyond the declared scope (method, files access, script output, calendar window), a missing activation, revoked by the user, denied by the declarative base, or denied or restricted by the OS.
- **NeedsConsent**, carrying the revision and declared scope to show and whether it was never granted, was widened by a new revision, or needs OS access. `may_prompt` is true only for an activation, so the host never asks while the user types or in the background.

When the user grants it, the host passes that revision and scope back to `Broker::consent`. The broker refuses the approval if the package has since moved to another revision or declares the scope differently, so a consent never approves more than the user saw. Settings takes both from `Broker::revision` and `Broker::consents` when it renders the list.

HTTP methods other than GET and HEAD, exec profiles, script targets, clipboard reads, clipboard writes and pastes need an activation. Reads, storage of the package's own data, clipboard watching and an exec profile declared `background` don't: `background` is part of the scope the user approves, and consent says the program runs even while they type, so a revision that adds it asks again.

An effect the host performs itself on a package's behalf goes through the same check: a row's `paste` action is decided as `Request::Clipboard(Paste)` with the activation that chose it, through `Bridge::authorize`, and the host validates the permit again just before posting ⌘V. Pasting needs macOS Accessibility, so Settings → Permissions lists packages that declare it on that row. The package that ports a built-in feature that pastes, as `snippets` does, pastes as that feature did: Dupin asks macOS for its own Accessibility access on the first paste and explains a refusal with the feature's notice before the broker decides the scope.

A service acts only within its permit: the HTTP service connects to the permit's origin, and the files service opens paths beneath the permit's binding, so its parsing can't disagree with the broker's. Redirects, resolved addresses and symlinks are still the service's to check.

Before releasing a result or performing an effect, the service calls `Broker::validate(permit)`. A permit holds only while the package keeps the same revision and generation, is enabled, and still holds the scope with the same binding and OS access.

## Generations

Every installed package has a generation, which also stamps its cached results and actions. Anything that takes authority away moves the package to a new generation at once: a revocation, disabling, a new revision or declaration, a rebinding, uninstalling, OS access leaving Allowed for a permission it declares, or a changed declarative base. The bridge drops in-flight completions and cached results stamped with an older generation. A results cache may mix scopes, so one generation per package replaces the policy's per-capability permit revisions. Generations never repeat in a process, so granting again can't revive old work. Granting and enabling don't change the generation. Effects already performed may survive.

## Grants

The broker owns grants, keyed by package ID, scope and the approved revision (the package's content hash).

| Layer   | Where                                                                        | Role                                                                                              |
| ------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Base    | A read-only file, such as Home Manager writes                                | Declared grants, denials, bindings and enablement. Grants apply only to the `revision` they name. |
| Local   | The `extension-grants` namespace of [encrypted storage](features/storage.md) | The user's consents, with the scope as approved, and revocations and enablement.                  |
| Bundled | Registered by the app                                                        | First-party packages that ship in Dupin.                                                          |

A scope's standing is decided in this order: a local revocation, a base denial, a local grant for this revision that covers the declaration, a base grant for this revision, a bundled pre-grant, then undecided (or changed, if an older approval no longer covers it). The base can grant and deny; local choices can always take authority away and can add it only where the base is silent. A base denial is locked: consent to it is refused. Enablement follows the same rule: a base `enabled = false` locks the package off for every revision, like a denial, while `enabled = true` applies only to the `revision` it names, if it names one. `Broker::is_enabled` answers whether a package is on; `Broker::is_turned_off` only whether the user or the base turned it off, for packages that run until someone does, and it treats unreadable grants as off.

- **Revisions.** `Broker::install` loads or updates a package's declaration and never grants. Approvals the new declaration doesn't widen carry over, narrowed to what it declares. Widened approvals need consent again. Grants of scopes it no longer declares are forgotten, but revocations are kept, so a later revision that declares the scope again stays revoked rather than getting a base or bundled pre-grant back.
- **Bundled packages.** While the installed revision is exactly the bundled one, the package is on by default and its scopes are granted, except files and exec scopes, which are bound to a path when the user allows them, even when the manifest names the path. OS access is still requested on enable or first use. A modified copy has another revision, so it is third-party: off, with nothing granted. Because `install` carries approvals across revisions of one package ID, the app gives bundled packages IDs of their own (`bundled:<name>`), so a folder package reusing a bundled name never inherits a grant or binding made for it.
- **Reloading the base.** A base read while running takes away at once what it no longer grants, but grants nothing new until the next launch: a configuration reload must never grant. The one exception is a fixed file replacing an unusable one (below), which restores the bundled pre-grants the unusable file withheld.
- **Unreadable grants.** Until the local grants are read, and whenever they can't be (a locked Keychain, data sealed with another key, a record this build can't read), the broker holds `Local::unreadable()`: nothing is granted, bundled pre-grants are suspended, and saving is refused rather than overwriting what is stored.

The crate keeps no file of its own: the host hands it a `LocalStore`, which keeps one record per package with any decision. `Local::load` reads and checks every record; `Broker::save` writes the record of each package whose decisions changed since a given point, and deletes the record of one left with none. `Broker::change_saved` makes a change and saves it, and undoes the change if either fails, so the broker never enforces a decision the store doesn't hold. The app's store is the `extension-grants` namespace, which the running launcher owns; `dupin ext remove`, a fresh `dupin ext install` or `dupin ext eject` asks the launcher to forget a package's grants (`Bridge::forget_grants`) while it runs, and forgets them in the namespace itself otherwise.

```json
{
	"package": "bookmarks",
	"enabled": true,
	"scopes": {
		"http:https://api.github.com": {
			"decision": "granted",
			"revision": "sha256:…",
			"approved": { "type": "http", "origin": "https://api.github.com", "methods": ["GET"] }
		},
		"clipboard:read": { "decision": "revoked" }
	}
}
```

The base names packages the same way:

```toml
version = 1

[packages.bookmarks]
revision = "sha256:…"
enabled = true
grant = ["http:https://api.github.com", "files:notes"]
deny = ["clipboard:read"]
bindings = { "files:notes" = "/Users/me/Notes" }
```

An entry without a `revision` can only switch the package and deny its scopes. Home Manager writes such entries for bundled packages under `bundled:<name>`: only the sealed copy that shipped carries that id, so a revision would pin nothing more.

`Base::parse` rejects the whole file only for a version it doesn't support or TOML it can't parse. A package entry it can't read, such as one with an unknown scope or a grant without a binding, is set aside alone, and `Base::problems` says why for Settings. Whatever that entry meant to deny is unknown, so the package fails closed: it is off, can't be turned on, and every scope it declares is denied by the base. As with any denial, a rejection in the base read at startup lasts until the next launch, even once the file is fixed.

A base file that exists but can't be used at all, because it can't be read, isn't TOML of the expected shape or has an unsupported version, is `Base::unusable`; a missing file is an empty base. The unusable file carries the user's denials, none of which are known, so while it stands no package gets anything from the base or from a bundled pre-grant: only grants the user gave in Dupin, which Dupin recorded itself, apply. Packages still load, stay on as they would be and answer whatever needs no capability, and `Base::problems` names the file and the parse error for Settings. Reading a fixed file again restores the bundled pre-grants at once, an exception to a reload never granting: they restore only what the build ships with. The fixed file's own grants and enables still wait for the next launch.

## OS permissions

Calendar reads need macOS Calendars access, clipboard reads and watching need the pasteboard policy (Paste from Other Apps), and pasting and both `windows` scopes need Accessibility. The host reads each without prompting and reports it with `Broker::set_os_access`; macOS's write-only calendar access counts as denied. `Broker::os_consumers` lists every installed package scope that needs one, for Settings → Permissions to show beside the built-in features in `features::permission_uses`. `dupin_mac::privacy::Permission::of` names the row each joins: Calendars consumers join the Calendars row, pasteboard consumers the Paste from Other Apps row, which clipboard history also uses, and Accessibility consumers the Accessibility row. `Access::os_access` and `PermissionStatus::os_access` turn what macOS reports into the `OsAccess` the host passes to `set_os_access`.

## OS-backed services

`dupin-host-services` has a service for each OS-backed capability: `dictionary`, `calendar`, `clipboard`, `windows` and `context`. Each defines a narrow backend trait that `dupin-mac` implements (`SystemDictionary`, which backs the bundled Dictionary extension, `EventStore`, which backs the bundled Calendar search extension, `GeneralPasteboard`, and `window_management::Windows`, which reads and moves the focused window of the app the panel opened over through the Accessibility API) and a `Fixture` for tests, validates and bounds its inputs and results, and returns serialisable data or a `ServiceError` whose `code` a guest can branch on. The services read OS access themselves and never prompt: a call while macOS hasn't been asked returns `needs-permission`, which the host can turn into a Request Access row. The calendar service also checks the range against the permit's `Bounds`, the clipboard service refuses reads and writes without an activation trigger, beside the broker's own checks, and the windows service refuses a frame that isn't finite, is out of bounds or lies off every display. Clipboard watches share one subscription to the monitor thread clipboard history uses, read each copy once and deliver on threads of their own, so a guest slow to take a copy never holds up the monitor; reads and watches skip concealed, transient and auto-generated copies, and copies from ignored apps or, while any app is ignored, copies the monitor didn't see land, before reading anything. A clipboard write names files by the handles the `files` service issued, so it can only put entries inside a granted scope on the pasteboard.

## The broker bridge

`dupin-extension-bridge` is the only way a guest reaches a host service. The `dupin` module's functions queue each request in the guest; the extension host sends it to the app over the helper's socket (protocol version 3), and the app answers it with the `Session` it made for that one call. A session is stamped with the package, its installed revision and generation, and the trigger the app saw: `Query` for a search while the user types, `Activation` only for an item action the user chose, `Background` for a watched event. The guest never names any of them.

For each request the session:

1. decodes the arguments strictly (unknown fields, wrong types and oversized values are `invalid` before the broker sees them);
2. builds the broker `Request` and calls `Broker::check`; a call whose generation has moved on is `revoked`;
3. runs the service only within the permit (the HTTP origin it names, the files root and the program the user picked, the package's own storage namespace with its quota and retention, checked against stored data in the same step as each write, the calendar window, the windows of the app the panel opened over, and what `context` noted as it opened);
4. calls `Broker::validate` again before an effect and before releasing the result, and drops the result if the permit no longer holds or the call ended.

`NeedsConsent` fails the request with `needs-consent` and keeps a `ConsentRequest`, the revision and declared scope to show, for the panel and Settings; a request while the package is off fails with `disabled` and is remembered the same way. Allow passes that revision and scope back to `Broker::consent` and turns the package on; it re-runs nothing, and the panel only takes the answered row away. Deny revokes the scope. A guest result is shown only while the generation it started at is current, so a revocation mid-call drops the rows it produced.

The bridge owns the grants: the base `extension-grants.toml` beside `config.toml`, which Home Manager writes keyed by each package's `package.json` name with the revision `dupin ext revision` prints, and the local grants in the `extension-grants` namespace, saved after every decision and never overwritten when they couldn't be read. A Home Manager switch rewrites the base, so before each call, each request, each delivered clipboard change and each save the bridge reads it again if its modification time, size or inode moved, with `Broker::replace_base`. The launcher alone writes the local grants while it runs: it reads them once, with `Bridge::open_grants` on a thread of its own at launch (and again when the panel shows while they couldn't be read), and `Broker::replace_local` takes them in. The installed extensions list keeps no grants; `dupin ext list` asks the running launcher for each package's granted scopes, or reads the namespace itself when the launcher isn't running, and `dupin ext remove`, like a fresh `dupin ext install` of a name, forgets the package's the same way. A package's revision is a BLAKE3 hash of its `package.json` and every module Dupin runs.

The app reports OS access as each Settings → Permissions row reads it, at startup, when the panel shows and when Settings regains focus, without prompting, and lists `os_consumers` by name on those rows. Clipboard watches subscribe only while the package is on, holds `clipboard:watch` and macOS allows the pasteboard, and coalesce a burst of copies to the newest. Tests run guest JavaScript through the sandboxed helper against in-memory services and never touch the network, the clipboard, calendars or OS permissions.

Bundled packages go through the same bridge. The app tells it each bundled revision (`Bridge::set_bundled`, which calls `Broker::set_bundled`) before installing the active revision of every package, so a bundled package is pre-granted exactly while the active copy is the one that shipped, and on then unless it ships off (`extensions::bundled::OFF_UNTIL_TURNED_ON`, which holds Repositories, Processes, Ports, Window Management and Running Apps). It knows a bundled package as `bundled:<name>`, which no package.json name can be. A package runs unless `Bridge::is_turned_off` says the user or the base turned it off, or it ships off and nobody has turned it on: any other package never turned on answers its keyword but uses nothing. The bridge makes each decision with `Broker::change_saved`, so a switch, grant or revocation that can't be saved changes nothing.
