# Capability broker

`dupin-broker` is the authority model every extension host service goes through (#12, #96, #218). It decides whether an installed package may perform one operation, remembers the user's decisions, and invalidates work when authority is taken away. It performs no OS, network, file or process operations itself and never prompts. It is not yet connected to the extension runtime, Settings or any host service; the broker bridge does that.

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
	"clipboard": ["read", "write", "watch"],
	"dictionary": ["lookup"],
	"calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }
}
```

| Capability   | Declares                                                                                                                                                                                                         | One scope per     |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- |
| `http`       | Up to 32 canonical `https://host[:port]` origins (lowercase, no path, no `:443`) with their methods: GET, HEAD, POST, PUT, PATCH or DELETE.                                                                      | origin            |
| `files`      | Up to 16 named scopes, each a `file` or `directory` with `read` and, for a directory, `list` access. Writes are rejected until they are designed.                                                                | name              |
| `exec`       | Up to 16 named profiles, each an argument template whose `{input}` placeholders the exec service fills, or `@script:<name>` targets for the user's script commands. `output` asks to return output to the guest. | profile or script |
| `storage`    | A quota of 1 byte to 64 MiB and an optional retention of 1–3,650 days.                                                                                                                                           | the capability    |
| `clipboard`  | `read`, `write` and `watch`.                                                                                                                                                                                     | access            |
| `dictionary` | `lookup`.                                                                                                                                                                                                        | the capability    |
| `calendar`   | `read` within `pastDays` and `futureDays` of today, each 0–366.                                                                                                                                                  | the capability    |

A scope is written as `http:https://api.github.com`, `files:notes`, `exec:git-status`, `script:open-in-zed`, `storage`, `clipboard:read`, `dictionary` or `calendar` in grant files.

## Checking an operation

A host service describes the operation as a `Request` and calls `Broker::check(package, revision, request, trigger)`. The trigger is set by the host from input it received: `Query` while the user types, `Background` for work nobody asked for just now, and `Activation` for a deliberate action. The answer is one of:

- **Allow**, with a `Permit` carrying the declared scope, its bounds and, for files and exec, the path the user picked.
- **Deny**, with a reason that renders as a user-facing sentence: not installed, a stale revision, unreadable grants, turned off, an invalid request (such as a path with `..`), an undeclared scope, beyond the declared scope (method, files access, script output, storage quota, calendar window), a missing activation, revoked by the user, denied by the declarative base, or denied or restricted by the OS.
- **NeedsConsent**, carrying the revision and declared scope to show and whether it was never granted, was widened by a new revision, or needs OS access. `may_prompt` is true only for an activation, so the host never asks while the user types or in the background.

When the user grants it, the host passes that revision and scope back to `Broker::consent`. The broker refuses the approval if the package has since moved to another revision or declares the scope differently, so a consent never approves more than the user saw. Settings takes both from `Broker::revision` and `Broker::consents` when it renders the list.

HTTP methods other than GET and HEAD, exec profiles, script targets, clipboard reads and clipboard writes need an activation. Reads, storage of the package's own data and clipboard watching don't.

A service acts only within its permit: the HTTP service connects to the permit's origin, and the files service opens paths beneath the permit's binding, so its parsing can't disagree with the broker's. Redirects, resolved addresses and symlinks are still the service's to check.

Before releasing a result or performing an effect, the service calls `Broker::validate(permit)`. A permit holds only while the package keeps the same revision and generation, is enabled, and still holds the scope with the same binding and OS access.

## Generations

Every installed package has a generation, which also stamps its cached results and actions. Anything that takes authority away moves the package to a new generation at once: a revocation, disabling, a new revision or declaration, a rebinding, uninstalling, OS access leaving Allowed for a permission it declares, or a changed declarative base. The bridge drops in-flight completions and cached results stamped with an older generation. A results cache may mix scopes, so one generation per package replaces the policy's per-capability permit revisions. Generations never repeat in a process, so granting again can't revive old work. Granting and enabling don't change the generation. Effects already performed may survive.

## Grants

The broker owns grants, keyed by package ID, scope and the approved revision (the package's content hash).

| Layer   | Where                                         | Role                                                                                              |
| ------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Base    | A read-only file, such as Home Manager writes | Declared grants, denials, bindings and enablement. Grants apply only to the `revision` they name. |
| Local   | `grants.toml` in the writable data directory  | The user's consents, with the scope as approved, and revocations and enablement.                  |
| Bundled | Registered by the app                         | First-party packages that ship in Dupin.                                                          |

A scope's standing is decided in this order: a local revocation, a base denial, a local grant for this revision that covers the declaration, a base grant for this revision, a bundled pre-grant, then undecided (or changed, if an older approval no longer covers it). The base can grant and deny; local choices can always take authority away and can add it only where the base is silent. A base denial is locked: consent to it is refused. Enablement follows the same rule: a base `enabled = false` locks the package off for every revision, like a denial, while `enabled = true` applies only to the `revision` it names, if it names one.

- **Revisions.** `Broker::install` loads or updates a package's declaration and never grants. Approvals the new declaration doesn't widen carry over, narrowed to what it declares. Widened approvals need consent again. Grants of scopes it no longer declares are forgotten, but revocations are kept, so a later revision that declares the scope again stays revoked rather than getting a base or bundled pre-grant back.
- **Bundled packages.** While the installed revision is exactly the bundled one, the package is on by default and its scopes are granted, except files and exec scopes, which need a path the user picks. OS access is still requested on enable or first use. A modified copy has another revision, so it is third-party: off, with nothing granted.
- **Reloading the base.** A base read while running takes away at once what it no longer grants, but grants nothing new until the next launch: a configuration reload must never grant.
- **Unreadable grants.** If the local file exists but can't be read, the bridge passes `Local::unreadable()`: nothing is granted, bundled pre-grants are suspended, and saving is refused rather than overwriting the file.

`Broker::save` writes the local layer atomically.

```toml
version = 1

[packages.bookmarks]
enabled = true

[packages.bookmarks.scopes."http:https://api.github.com"]
decision = "granted"
revision = "sha256:…"
approved = { type = "http", origin = "https://api.github.com", methods = ["GET"] }

[packages.bookmarks.scopes."clipboard:read"]
decision = "revoked"
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

## OS permissions

Calendar reads need macOS Calendars access, and clipboard reads and watching need the pasteboard policy (Paste from Other Apps). The host reads each without prompting and reports it with `Broker::set_os_access`; macOS's write-only calendar access counts as denied. `Broker::os_consumers` lists every installed package scope that needs one, for Settings → Permissions to show beside the built-in features in `features::permission_uses`. `dupin_mac::privacy::Permission::of` names the row each joins: Calendars consumers join the Calendars row and pasteboard consumers the Paste from Other Apps row, which clipboard history also uses. `Access::os_access` and `PermissionStatus::os_access` turn what macOS reports into the `OsAccess` the host passes to `set_os_access`.

## OS-backed services

`dupin-host-services` has a service for each OS-backed capability: `dictionary`, `calendar` and `clipboard`. Each defines a narrow backend trait that `dupin-mac` implements (`DictionaryProvider` and `CalendarProvider`, which also back the `define` and `cal` searches, and `GeneralPasteboard`) and a `Fixture` for tests, validates and bounds its inputs and results, and returns serialisable data or a `ServiceError` whose `code` a guest can branch on. The services read OS access themselves and never prompt: a call while macOS hasn't been asked returns `needs-permission`, which the host can turn into a Request Access row. The calendar service also checks the range against the permit's `Bounds`, and the clipboard service refuses reads and writes without an activation trigger, beside the broker's own checks. A clipboard watch subscribes to the same monitor thread as clipboard history and skips concealed, transient and auto-generated copies and ignored apps before reading anything.

## Remaining integration

The broker bridge registers bundled packages, installs each package's declaration with its content hash, reads and saves the grant files, maps OS status, passes host-verified triggers, stamps caches with generations, and adds consent, revocation and the Permissions listing to Settings. Each host service then checks and validates its operations through it. Tests use synthetic state only and never request or change OS permissions.
