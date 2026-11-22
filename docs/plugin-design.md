# Plugin design proposal

Status: proposed contract, not an implemented API. Tracks #6, #7, #17 and #96.

## Product boundary

Dupin provides an input field, input routing, result selection, host-rendered views,
actions and plugin execution. App search, file search, dictionary, calculator,
currency and calendar are feature plugins. First-party distribution does not give
a plugin an implicit permission grant.

The everyday experience stays as simple as Alfred: summon, type, choose. Rich
views appear when a task needs them. Saved snapshots have a separate searchable
window; an always-visible tab system is not a prerequisite.

## Runtime decision gate

Wasm is the first candidate, not a settled dependency. Compare the native
calculator prototype with the same calculation compiled to Wasm. Measure the
release executable and complete app bundle, packaged plugin bytes, cold load,
warm query latency, cancellation and peak/idle memory. Use the repository's
release settings and record exact runtime versions and features.

A small standalone runtime host can reject a clearly unsuitable runtime, but its
size delta does not prove the app's final bundle delta. The adoption decision
requires an app-level measurement. Compare an interpreter with a compiled runtime
before choosing compilation caching or shipping a compiler in the bundle.

Before adoption measurements, pin the baseline commit, toolchain, architecture and
release flags in a checked-in script. Measure stripped executable, complete `.app`
and shipped archive bytes for: no runtime; runtime without guests; smallest guest;
calculator guest; and a second separately packaged guest. Count duplicated guest
libraries. A later JS/TS engine needs its own app-level measurement. JIT/executable
memory and hardened-runtime entitlement changes are adoption criteria.

The initial candidate guest target has no ambient WASI filesystem, network,
process or environment access. Host imports provide declared operations. Fuel,
memory limits, message limits and cancellation must be verified for the chosen
runtime. A worker thread alone does not isolate native crashes or enforce grants.
Guest traps are contained; native host-import bugs remain host failures.

## Manifest and registration

The launcher uses an internal host-owned command registry for bundled providers.
Qualified command IDs identify the feature plugin and its command; query routing,
recall, result limits and query presentation use those registrations. The host
also owns the default command, the empty-query Space route and keyword ownership.
Conflicting configured aliases are reported without replacing another command's
trigger. This replaces the fixed search-mode table; it is not an external plugin
installation API or a capability grant.

A versioned manifest identifies the plugin and its version, entry module, commands,
default-query providers, keyword triggers, state schema version and capabilities.
User aliases are configuration layered over manifest defaults. The host owns
registration, reports clashes and uses explicit user aliases to resolve them.
Plugins cannot silently claim triggers or replace reserved host commands.

The host assigns plugin identity to every invocation; guests cannot nominate
another identity. Installation validates the manifest and imports before enabling
execution. Compatibility failures are reported before the plugin receives data.

Bundled Rust implementations can exercise this contract during migration, but
must be identified as trusted in-process code. Their isolation properties are not
equivalent to a restricted guest. The public contract must not depend on native
Rust ABI or arbitrary dynamic-library loading.

Transitional bundled plugins also receive broker handles rather than direct OS
services. This is a trusted-code boundary, not a sandbox against malicious native
code. Migration ends only when the shell has no feature-specific modes, result
kinds or payload fields and feature code has no direct OS access outside the
broker. Upgrade migration explicitly records the enabled state and per-plugin
grants for previously available app/file/dictionary operations. Settings shows
these migrated grants and lets the user revoke them; global OS consent alone
does not create a grant. New permission-dependent capabilities and user-installed
plugins start disabled. Bundled permission-free plugins may be enabled by default,
with their state visible in Settings; adding a privileged capability still needs
an explicit grant.

## Requests and results

Each invocation carries a request ID, generation, command/provider ID, query,
result limit and optional versioned state. Replacing a query cancels its older
generation. The host discards late responses and late capability completions.

Plugins publish keyed result/view updates and a terminal completion or error.
Result ordering is explicit: a calculator answer can be pinned before ranked app
candidates without disabling candidate ranking. Errors do not replace another
plugin's notice or masquerade as an empty result.

Only host routing policy and user settings choose the single pinned result.
Plugins cannot give themselves priority or take over Enter. Each query has a host
deadline; slow providers cannot delay others. Settings shows per-plugin resource
use and failures; repeated budget failures visibly suspend a plugin until resumed.

Slow host operations return a pending operation ID rather than blocking a query.
The host later sends a correlated completion event and re-enters the guest with
its state. The protocol must work without an async guest runtime. Cancellation
and grant revocation invalidate pending completions as well as visible results.
Guest code does not own the wall clock: host budgets and fuel enforce execution
limits, with an explicit host import if an evaluator also needs cooperative checks.

The [experimental query protocol](plugin-protocol.md) exercises bounded UTF-8
messages, versions, correlation and per-request guest-memory ownership with a
real calculator guest. It is not yet used by the launcher. Registration,
capability brokerage, cancellation/deadlines, action invocation and streaming
still need implementation and review before external plugins run.

## Views, actions and saved state

The host renders a versioned data model for lists, details, forms and source/result
cards. Plugins supply content and stable action IDs, not native UI objects or
arbitrary code inside the view. Text selection and keyboard focus belong to the
host renderer.

Enter invokes a declared primary action. Right Arrow and Cmd+K open the shared
action menu. Dictionary's external-open operation moves there when that mechanism
is available. Privileged host actions are checked when invoked, not trusted
because they appeared in a rendered result. Enter is not a grant. Open-path
actions use broker-issued resource handles scoped to that plugin, not arbitrary
guest paths. URLs require granted schemes/destinations. Clipboard read and write
are separate capabilities.

State is bounded, versioned serialisable data held by the host. A snapshot stores
captured display data that remains readable without the plugin, plus an optional
resume token identifying the plugin, state version and query. Restoring a snapshot
does not restore an old grant or silently execute an action. Disabling a plugin
offers deletion of its snapshots. Retained snapshots stay readable historical
content, but actions/resume remain inert until their required grants are restored.

## Capabilities and consent

Keep four states distinct: manifest declaration, plugin enabled state, user grant
and macOS consent. The broker checks them for every host operation.

Permission-dependent plugins are enabled explicitly in Settings or through the
upgrade migration described above. Enabling alone,
launching Dupin and opening Settings do not request OS access. An enabled plugin's
first relevant invocation can request consent through host UI. Settings shows the
declared capabilities and records per-plugin grants. The first invocation prompts
macOS only when consent for Dupin has not already been decided. Existing macOS
consent never substitutes for another plugin's grant. Denial or revocation
produces an actionable status, without repeated automatic prompts.

Capability scopes cover application catalogues, file queries/access, dictionary
lookups, calendar queries, clipboard operations, external URLs/apps, namespaced
storage and constrained network requests. Actual API names and response schemas
must be driven by the first implementations. Native API access stays in the host;
feature-specific parsing, ranking, caching policy and presentation stay in plugins.

Network grants constrain destinations and redirects and cap response bytes.
Resolved loopback and private-network addresses are denied unless explicitly
granted. Storage is namespaced and bounded. New capabilities from an update stay
unavailable until approved; the plugin can continue its existing granted
operations. A required unavailable capability
produces an explicit unavailable state rather than an implicit grant.
Disabling a plugin cancels its work and prevents further broker access. macOS may
grant access to Dupin globally; each plugin still needs its own grant.

## First implementation cases

1. Calculator: local expressions and compound units, no permission request;
   source/result card and copy action. Compare native and guest execution.
2. Currency: expression parsing uses the calculator engine; rate data is read from
   plugin storage and refreshed through constrained HTTP. Show the observation
   date separately from fetch time. Cached rates remain usable offline.
3. Calendar: explicit enablement, permission on first invocation, bounded EventKit
   query through the broker, plugin-owned search/display and an external-open
   action. Tests use synthetic events and never request real permissions.
4. Migrate app, file and dictionary providers through the same registration,
   lifecycle, view and action contracts, retaining their existing input interactions.

Runtime selection, guest ABI, package format and measurable performance budgets
remain open. More feature-specific built-ins should not ship before this boundary
is exercised and reviewed.

## Runtime references

- [Rust's bare Wasm target](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)
- [Wasmi configuration and limits](https://docs.rs/wasmi/latest/wasmi/struct.Config.html)
- [Wasmtime resource limits](https://docs.wasmtime.dev/api/wasmtime/runtime/limits/trait.ResourceLimiter.html)
- [Wasmtime compilation cache](https://docs.wasmtime.dev/examples-fast-compilation.html)
