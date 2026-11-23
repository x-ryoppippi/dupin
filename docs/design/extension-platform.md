# Dupin extension platform

## Problem and goals

Dupin has three ways to add features. Native features register commands and providers, trusted script commands run external programs, and the experimental Wasm runtime exposes a keyword, a bounded data protocol and a few host actions. The Wasm path requires a guest toolchain and has no host imports. None offers a small, typed package that an agent can write, run, inspect and test from text alone.

Give authors Raycast-style typed views, hot reload and useful diagnostics with a small native launcher footprint. Keep typing independent of extension work. A new extension starts with a query and a list. Built-ins remain complete with an empty config, and advanced users can combine them with extensions. Dupin renders every view and brokers access to the system. [Current extensions](../features/extensions.md) · [Script commands](../features/scripts.md) · [Raycast developer docs](https://developers.raycast.com/)

## Decision summary

- **Recommend native QuickJS-ng via `rquickjs` as the single guest runtime.** A package is a directory with a `package.json` whose `dupin` key is the manifest, a JavaScript ES module and optional local assets. The default entry is `main.js`. The loader resolves relative package imports and the host's `dupin` module. A name, a version, a keyword and `search(query)` are sufficient. The first runtime PR must prove the signed size, responsiveness and isolation gates below.
- **Guest JavaScript runs in a sandboxed extension host.** The host is Dupin's own executable, started in a hidden mode on the first query. It applies a deny-by-default macOS sandbox before it runs any guest code, receives compiled packages over a socket and never reads the extensions folder. Dupin stops it when a call overruns, and starts another for the next query. QuickJS-ng is built at `-O3`.
- **TypeScript builds on the author's machine.** `dupin ext dev` and `dupin ext build` invoke Bun or esbuild and ship a `dupin.d.ts` declaration file to authors. Installed packages contain runnable JavaScript. The host executes only JavaScript and never type-checks.
- **Use one host contract.** Sources produce items, filters match, sort and convert them, and kinds supply actions. Built-ins implement those interfaces in Rust. Dupin owns the command registry, navigation, rendering and permissions. Settings shows feature switches and keywords. Recomposition lives in `[advanced]`.
- **Retire wasmi after JavaScript reaches parity.** The existing implementation creates no release compatibility obligation. Keep it during the prototype, then port the examples and remove the Wasm guest loader, runtime and guest ABI together. Carry forward useful host schemas, limits and adversarial fixtures. Trusted script commands remain available for any-language automation.

All three designs converge on native QuickJS-ng. Fable and Opus remove wasmi at parity; Astra would keep it for compute guests, but the owner has no compatibility obligation to it. Guest code has no ambient system APIs, but an engine or binding memory-safety flaw could compromise the Dupin process and its OS access. The [adoption experiment](../../experiments/js-runtime/README.md) found that in-process controls are not enough: a native `String.prototype.indexOf` never polls the interrupt handler, and the process footprint overshoots the heap limit. Guest code therefore runs in a confined helper process that Dupin can kill, so a flaw or a stuck native call while a guest runs reaches only that process. Every JavaScript extension shares that process, so a flaw exploited by one can read the others' bytecode and queries and change their answers until the helper is replaced. Dupin still parses and compiles packages in its own process when it loads them; moving compilation into the helper would close that gap too. The helper makes the size gate's `-Oz` trade-off pointless, and removing wasmi offsets the larger `-O3` engine. [Runtime discussion](https://github.com/x-ryoppippi/dupin/issues/6) · [Extension direction](https://github.com/x-ryoppippi/dupin/issues/96)

## Runtime comparison

Fable measured size deltas in stripped, standalone Rust binaries built for size with fat LTO on an Apple M5 Max. In-process means cover 2,000 runs of a 400-candidate module that returns 50 rows. Astra's separate FFI probe on the same Mac model reports p95 for a tiny expression, and its subprocess probe exchanges one JSON line over pipes. Opus measured standalone size at two compiler profiles and timed bare process spawns. The workloads and memory metrics differ. Only the historical wasmi figures are signed Dupin app deltas. The adoption gate needs fresh signed-app, ZIP and end-to-end data.

| Candidate                                    | Size evidence                                                                                                                                                                                               | First result / warm call                                                                                                                                            | Memory evidence                                                                  | Authoring, isolation and maintenance                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JavaScriptCore, system C API                 | ~0 engine bytes on macOS. Fable's binding added 688 B.                                                                                                                                                      | Fable 0.61 ms / 0.049 ms. Unsigned with JIT available. Astra tiny probe 0.046 ms / 0.0030 ms p95, 3.68 ms first VM.                                                 | Astra +3.45 MiB peak RSS through first result.                                   | Familiar JS and promises. Needs a module adapter and supported resource controls. Apple supplies updates on macOS. Linux needs JavaScriptCoreGTK and Windows has no equivalent system framework. [Apple](https://developer.apple.com/documentation/javascriptcore) · [JavaScriptCoreGTK](https://webkitgtk.org/reference/jsc-glib/stable/index.html)                                                                                                                                               |
| **QuickJS-ng, native**                       | Fable +0.59 MiB, Opus +0.59 MiB for size or +1.06 MiB for speed. Astra's dylib was 1.09 MiB.                                                                                                                | Fable 0.87 ms / 0.25 ms. A 25 KB module took 2.12 ms fresh from source. Astra tiny probe 0.145 ms / 0.0029 ms p95, 3.92 ms first VM.                                | Fable 209 KB live context. Astra +0.77 MiB peak RSS through first result.        | Familiar ECMAScript modules and promises, with TS compiled by authors. Heap, stack and interrupt controls and a host promise bridge. One portable embedding, with Dupin responsible for patching the C engine. [ECMAScript support](https://quickjs-ng.github.io/quickjs/es_features/) · [Runtime controls](https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html)                                                                                                                          |
| Embedded V8 via `deno_core`                  | Roughly +20–40 MiB planning range. Opus estimated +25–35 MB.                                                                                                                                                | Opus estimated 5–20 ms / 0.05 ms.                                                                                                                                   | Opus estimated 5–20 MiB.                                                         | Familiar JS and mature tooling. Dupin would supply the host API and enforce I/O permissions. Portable, with a larger native build and JIT integration. Size exceeds the budget. [Deno Core](https://github.com/denoland/deno_core)                                                                                                                                                                                                                                                                 |
| Lua 5.4 via `mlua`                           | Fable +0.30 MiB.                                                                                                                                                                                            | Fable 0.36 ms / 0.12 ms.                                                                                                                                            | Fable ~0.25 MiB.                                                                 | Closest small-runtime alternative. Restrict standard libraries and use allocator limits, instruction hooks and a coroutine bridge. Portable, with an additional language and a table-to-JSON contract for authors and agents. [Lua](https://www.lua.org/about.html) · [mlua](https://docs.rs/mlua/latest/mlua/)                                                                                                                                                                                    |
| Luau via `mlua`                              | Fable +1.23 MiB.                                                                                                                                                                                            | Fable 0.56 ms / 0.13 ms.                                                                                                                                            | Fable ~0.7 MiB.                                                                  | Portable VM with sandboxing, interrupts, coroutines and gradual typing. Costs more than the size-focused QuickJS-ng build and requires a different dialect and tools. [Luau sandbox](https://luau.org/sandbox/)                                                                                                                                                                                                                                                                                    |
| wasmi / Wasmtime                             | Historical wasmi +1,136,272 B signed app / +396,414 B ZIP, ~1.2 MiB in current docs. Wasmtime standalone host 7.27 MiB vs wasmi 1.99 MiB.                                                                   | wasmi ~1–6.5 ms module load and ~0.1–1.2 ms warm. Wasmtime/Cranelift 373 ms to compile a 766 KB calculator and 0.017–0.130 ms warm.                                 | Paired calculator peak physical footprint 22.1 / 40.6 MiB.                       | Strong memory isolation. Wasmi already has fuel and limits and exposes no imports in the production ABI. Guest toolchains and a broker bridge add authoring work. Wasmtime components add WIT tooling and a new host contract. Hardened-runtime JIT requirements need verification. [Wasm app size](../../experiments/wasm-app-size/results/2026-11-22-aarch64-darwin.json) · [Calculator comparison](../../experiments/wasm-calculator/results/2026-11-22-runtime-comparison-aarch64-darwin.json) |
| JavaScript inside Wasm                       | Javy documents ≥869 KB static guests or 1–16 KB with a shared dynamic engine. ComponentizeJS documents ~8 MB for the SpiderMonkey embedding. Opus estimated a ~1 MiB shared QuickJS-ng resource plus wasmi. | No compatible integrated measurement. Opus estimated 2–8 ms cold plus one-off engine compilation, and 0.4–1.5 ms warm. Fable estimated ~14 ms cold / ~4 ms warm.    | Opus estimated 2–6 MiB linear memory per live guest, excluding host allocations. | Familiar JS with an additional Wasm build and stronger isolation. Shared engine imports or a Component Model host change the current ABI. Both latency estimates extrapolate from calculator ratios and need a direct benchmark. [Javy](https://github.com/bytecodealliance/javy) · [ComponentizeJS](https://github.com/bytecodealliance/ComponentizeJS)                                                                                                                                           |
| Out-of-process Bun / Node / Python over JSON | 0 bundled interpreter bytes if user-supplied.                                                                                                                                                               | Astra cold p95 to first reply 19.9 / 67.0 / 89.6 ms. Resident p95 IPC 0.071 / 0.042 / 0.049 ms. Fable measured 38.6 / 72.3 / 37.6 ms to first reply in its harness. | Astra median child peak RSS 21.8 / 39.6 / 15.0 MiB.                              | Familiar languages and ecosystem access. The child runs with native user authority unless separately sandboxed. Runtime discovery, supervision and process cleanup vary by platform. Keep this model for trusted script commands.                                                                                                                                                                                                                                                                  |

## TypeScript strategy

| Strategy                                 | Authoring and cost                                                                                                                                                                                                                                                                                                                                  | Decision                                                                                                                                                                              |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Author-side build with Bun or esbuild | `dupin ext dev/build` emits one plain ES module, optionally with source maps. The scaffold supplies `dupin.d.ts` and TS authors can run `tsc --noEmit` locally. Pure JS dependencies can be bundled. The launcher has zero TS tooling cost.                                                                                                         | **Decided.** Plain JS needs no build. The installer and MCP fixture runner consume ready-to-run JS. [esbuild TypeScript support](https://esbuild.github.io/content-types/#typescript) |
| B. Host-side oxc type erasure            | Later, if a build-free TS path proves worthwhile, accept erasable syntax only. Fable measured +0.43 MiB for a parser and span-blanking skeleton, or +1.06 MiB for a full transform. Opus measured +1.08 MiB for a full oxc transform. Opus also measured +0.92 MiB for `swc_ts_fast_strip`. A custom eraser needs correctness and diagnostic tests. | Possible later addition. Requires a new signed-size gate and never performs type checking.                                                                                            |

## Architecture

| Concept              | Placement in Dupin                                                                                                                                                                                                                                                                                                                                                                                                    |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source               | Adapt today's `Provider` behind a common interface. An `items()` source refreshes a catalogue on first use, invalidation or expiry. Rust filters cached candidates as the user types. A `search(query)` source owns matching for each query, as for a calculator or an OS index. Native providers keep their platform APIs.                                                                                           |
| Filter               | Run a matcher, then a sorter, then a display converter per source. Reuse the native fuzzy scorer and usage history. Run guest stages only on bounded batches. Matchers select row IDs and highlights, sorters reorder existing IDs, and converters edit display fields. The host retains identity, payload and action authority. Reject matcher overrides that the source cannot support before candidate truncation. |
| Kind                 | Register a typed payload and actions for it. Text, URL and file kinds centralize the action lists currently assembled by providers. The host resolves the action against the current row, installation and page. File actions use host-validated resource references.                                                                                                                                                 |
| Host UI and commands | Compile feature defaults and advanced overrides into the existing command registry. `Session` still merges results, applies display limits, owns focus, restores selection on back navigation and drops stale generations. Rune renders the same bounded view model for built-ins and extensions.                                                                                                                     |

`features::MANIFEST` continues to own configure, prepare, start and reload. A registration supplies its providers, supported composition and default routes, plus the existing permission-use records. Migrate native features incrementally and preserve command IDs, keywords, usage history, pinned web results, fallback order and empty-query routes. The guest adapter registers a top-level `search` or `items` automatically. An advanced extension can declare named contributions in `[provides]` and export `sources`, `filters` or `kinds` maps. Dupin qualifies those local names with the installation's package ID. [ddu.vim architecture](https://raw.githubusercontent.com/Shougo/ddu.vim/main/doc/ddu.txt) · [Feature manifest](../../crates/dupin/src/features/mod.rs) · [Dupin architecture](../architecture.md)

The default `config.toml` can be empty. With `[advanced]` absent, the feature manifest supplies the full composition and the existing Settings toggles determine which features participate. For example, the files route keeps its native file actions and the main search uses the built-in app matcher. An advanced user can add a static bookmarks list, use prefix matching for apps and add an action without changing the other defaults:

```toml
[advanced.commands.default]
append_sources = ["bookmarks"]

[advanced.sources.apps]
matcher = "builtin.prefix"

[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"
```

The configured script resolves to a user-installed command and receives the selected file only when invoked. Invalid references, conflicting keywords and incompatible payloads leave the last valid composition active. An extension that receives general search input needs separate consent. An extension filtering or acting on another producer's results receives only explicitly approved fields. The host keeps private payloads and file handles. A guest filter has no external effects or persistent state.

## Example extensions

### A query in, a list out

`package.json`

```json
{
	"name": "word-count",
	"version": "0.1.0",
	"dupin": { "title": "Word Count", "keyword": "wc" }
}
```

`main.js`

```js
export function search(query) {
	const input = query.trim();
	if (!input) return [];

	const count = input.split(/\s+/u).length;
	const title = `${count} words`;
	const text = String(count);

	return [{ title, text }];
}
```

Type `wc two words` to see **2 words**. Enter copies `2`. The two files above are the complete extension.

### A list source

Use the same manifest shape with `"name": "bookmarks"`, `"title": "Bookmarks"` and `"keyword": "bm"`. Replace the entry with `items()`. Dupin caches the rows and supplies URL actions and native matching.

```js
export function items() {
	return [
		{ title: 'Dupin', url: 'https://github.com/x-ryoppippi/dupin' },
		{ title: 'Rune', url: 'https://github.com/x-ryoppippi/rune' },
	];
}
```

### A pushed view with a capability

Add the following `capabilities` entry to the bookmarks `dupin` key and append the handler below to its entry. Add `actions: [{ title: "Repository status", push: { view: "repository" } }]` to the Dupin row. Selecting that action opens a host-rendered Detail page. The entry requests HTTP access only for the declared origin and method.

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }]
}
```

```js
import { http } from 'dupin';

export const views = {
	async repository() {
		const response = await http.fetch('https://api.github.com/repos/x-ryoppippi/dupin');
		if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
		const repo = await response.json();
		return {
			type: 'detail',
			title: 'Dupin',
			markdown: repo.archived ? '**Archived**' : 'Active repository',
		};
	},
};
```

An offline fixture drives the selected action and returns a synthetic `200` with `{"archived": false}`. The expected view contains `Active repository`. The same fixture can exercise denial and revocation without touching GitHub.

## API and UI components

| API                                  | Contract                                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `search(query, ctx)` or `items(ctx)` | A synchronous or async array of items or a List object with optional sections and empty-state text. Items have a title, optional stable ID, subtitle, icon, keywords and kind data. The host applies schema and output limits and preserves the relative order of query results by default.                                                                                               |
| `views[name](ctx)`                   | Returns List, Detail or Form. `ctx` holds read-only preferences, the current page query and bounded JSON state. A fresh invocation makes module globals transient.                                                                                                                                                                                                                        |
| `actions[name](data, ctx)`           | Runs on explicit activation, including form submit. Returns a bounded effect such as push, pop, refresh, replace state, toast, or field errors. Named handlers and their data are resolved against the active page. Simple Copy, Open URL and Push can be declared directly as data.                                                                                                      |
| Preferences and state                | Manifest fields support text, checkbox and single-select with defaults. Missing required values lead to Settings. `Session` owns per-page state and commits only the current generation. Saved presentation is inert. Resuming a page revalidates package identity, state compatibility and grants. Defer arbitrary persistent storage and secret preferences until a feature needs them. |

The initial component set is deliberately small:

| Component    | Supported surface                                                                                                                                              |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| List         | Labeled sections, title/subtitle, built-in icons, up to three text or tag accessories, two-value answer rows and an optional selected-item Detail pane.        |
| Detail       | Title, labeled metadata and bounded Markdown with headings, paragraphs, emphasis, lists, inline/fenced code and HTTP(S) links. Dupin handles link activation.  |
| Form         | Text, multiline text, checkbox and single-select. Dupin owns drafts, tab order, required-field validation and submit/cancel. Guest validation runs on submit.  |
| Action panel | Primary and secondary actions, up to eight guest actions per item. Enter invokes the primary. Right or ⌘K opens the panel. Dupin confirms destructive actions. |
| Status       | Empty text, host loading and error states, access recovery, one transient toast after an action and a brief HUD when the action closes the panel.              |

Grids, charts, custom styling, JSX/React, webviews, HTML and remote media would add another rendering or network surface. Background execution, menu-bar items, arbitrary key bindings and guest focus control would add separate lifecycle and interaction policies. Defer them. A component needs a keyboard interaction and a deterministic renderer fixture before it is added. [View and state discussion](https://github.com/x-ryoppippi/dupin/issues/7)

## Capabilities and consent

V1 exposes exactly three capabilities, `http`, `exec` (including `@script` targets) and `files`. The manifest declares the maximum scope. A Dupin-owned store persists user grants against the installation and approved revision. A manifest or config reload cannot grant access. The broker checks enablement and scopes at dispatch and before delivering results. Query handlers may use approved bounded reads. Mutations and trusted native tool invocations require a host-verified user activation. The broker owns user-facing errors, cancellation and output bounds.

| Capability | Scope and enforcement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `exec`     | A named profile binds a user-selected executable to an argument template and working directory. Validate typed inputs, use argv without a shell and supply a minimal environment. Reuse script-runner process cleanup. An `@script:<name>` target binds to a specific user-installed script command and invokes the trusted runner. Returning script output to the guest requires explicit approval. Both targets run with native user authority and may use their own credentials or hooks. Show the command and that consequence in consent. |
| `http`     | Explicit HTTPS origins and methods. Validate redirects and resolved destinations, block private addresses by default and cap time and response size. Supply no ambient cookies or authentication.                                                                                                                                                                                                                                                                                                                                              |
| `files`    | Named user-picked file or directory scopes. Start with read/list. Validate the opened resource within the allowed root, including symlink and race handling. Add writes separately when needed.                                                                                                                                                                                                                                                                                                                                                |

Plain HTTP(S) Open, Copy and Push actions use existing host validation and a deliberate selection. Later manifest additions can declare distinct programmatic clipboard read and write scopes and OS-backed reads such as calendar access. An outbound `mcp` capability can bind an exact tool on a user-configured server to the approved server identity and schema. Require activation for each call, validate input and bound output and lifetime. The host owns connection credentials and transport. Default-search input and cross-producer fields each require a separate scoped consent decision before guest composition can expose them.

1. **Enable.** New installations start off. Deliberately enabling one shows its requested scopes and lets the user grant them individually. Dupin.app requests any still-undetermined macOS access needed for a selected scope. First deliberate use can request access if configured outside Settings. Startup, Settings open, reload, background search and fixtures only inspect status.
2. **Explain.** Build Settings → Permissions from the existing `features::permission_uses()` and validated extension declarations. Show each OS permission once and list its consumers and extension grants. **Request Access** can ask macOS on behalf of Dupin with a feature off. That OS decision grants no extension authority by itself. Offer **Open System Settings** after denial and refresh access on focus and prompt completion.
3. **Revoke.** Scope each grant to the installed package and approved revision. Disable or revoke immediately, cancel owned work where possible, drop pending completions and invalidate derived caches and actions. Reapproving a grant cannot revive old completions. Previously completed external effects may survive. Reuse the existing policy's invalidation semantics behind the broker. Keep installation IDs and revisions out of the author API. [Current permission policy](../features/permissions.md) · [Guest policy semantics](../plugin-permissions.md) · [Owner decision](https://github.com/x-ryoppippi/dupin/issues/12)

## Agent workflow and packaging

| Proposed interface                                                 | What it does                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `dupin ext new <dir>`                                              | Scaffolds the manifest, JS or TS entry, `dupin.d.ts` and a fixture.                                                                                                                                                                                                            |
| `dupin ext dev <dir>` / `dupin ext build <dir>`                    | Validates JS or runs the author-side TS build. Dev watches source edits and publishes a valid replacement atomically to a development catalogue. A failed build keeps the previous revision active. Build emits the installable directory.                                     |
| `dupin ext check <dir>` / `dupin ext run <dir> --query …`          | Checks a ready-to-run manifest and module, or invokes it and prints validated view data. `--json` errors include a code, file or field, source location where available and a fix hint. Failures exit non-zero.                                                                |
| `dupin ext test <dir>` / `dupin ext snap <dir> <fixture> <output>` | Replays input, actions and synthetic broker replies. Fix the clock, random seed, locale, fonts, dimensions and theme. Compare canonical JSON first and use the production renderer for PNG snapshots on a pinned platform.                                                     |
| `dupin mcp`                                                        | Expose `ext_check`, `ext_run`, `ext_test` and `ext_snap` over the same offline fixture runner for prebuilt packages in a user-selected workspace. No package build hooks, live grants or OS prompting through these tools. Outbound `mcp` remains a possible later capability. |

See [Packaging and distribution](#packaging-and-distribution). [Existing CLI](../../crates/dupin/src/cli.rs) · [MCP direction](https://github.com/x-ryoppippi/dupin/issues/34)

## Packaging and distribution

The manifest is the `dupin` key of `package.json`. It is static JSON, so Dupin reads the requested capabilities, keyword and entry without running guest code, and authors keep the file their JS tooling already uses. There is no separate manifest file and no manifest generated from code.

Authors use npm dependencies freely and bundle them at build time. The installed entry is one self-contained ES module. A build fails when a dependency needs Node built-ins such as `fs` or `net`, which do not exist in the guest.

Packages are distributed through Git, not a package registry:

| Source | Example                                                  | Behavior                                                         |
| ------ | -------------------------------------------------------- | ---------------------------------------------------------------- |
| Git    | `dupin ext install github:user/repo@v1` or `@<commit>`   | Fetches the tag or commit and records the resolved commit SHA.   |
| Local  | `dupin ext install ./word-count`, or `dupin ext dev ./…` | Loads the folder in place, for authoring and private extensions. |

- **Built JS is committed.** Plain JS needs no build. A TypeScript package commits its built entry, as GitHub Actions do. The `dupin ext new` scaffold includes a workflow that rebuilds and commits it on push. An install never runs a package manager, install scripts or a build. A package without its built entry fails with a diagnostic.
- **Pinning.** A lock file records each package's source, resolved commit SHA, content hash and granted scopes. Installed contents change only through `dupin ext update`, which moves to the newest tag, or the default branch when the repository has no tags. An update that adds scopes asks again. Removed scopes are dropped.
- **Nix.** Home Manager declares packages with `fetchFromGitHub` and their grants. That configuration is the read-only base, and local grant choices overlay it in the writable data directory, as `config.toml` and `settings.toml` do.
- **Discovery.** Dupin lists GitHub repositories with the `dupin-extension` topic through the GitHub API. There is no reviewed store, hosted index or publishing account. Listings show each package's requested capabilities, because the broker, not a review, protects users.

npm publishing, a curated index repository and a reviewed store stay out of scope until real use shows a need.

## Performance budget and gates

Use the existing provider workers and generation checks. Lazily load a bounded module on use. Provider workers send calls to one resident extension host process, which queues them and runs one at a time; a waiting call that is superseded or cancelled leaves the queue. Each invocation in the helper gets a fresh QuickJS runtime and context with no ambient filesystem, process or network APIs, and the helper's sandbox denies those operations to the process as well. A host watchdog stops the helper when a call outlives its deadline or ignores cancellation, or when the helper's footprint passes its allowance. Since every abandoned call stops the helper, the helper speaks only to answer the call in flight; any other frame stops it, and replies wait in a bounded queue so a helper that writes unasked stalls instead of growing Dupin's memory. A helper still starting when its first call gives up is kept for the next call. Cache validated source and static results in the host view format. Start with 16 MiB heap and 512 KiB stack limits per invocation, four concurrent guest calls across the app and bounded request, result and job queues. Enforce a monotonic deadline while loading the module, executing code, pumping promise jobs and encoding results. Start with the existing two-second search deadline as the outer containment limit and tune the active execution budget against the prototype. Resolve broker operations asynchronously. Coalesce superseded jobs, cancel pending operations and drop late results, including those from replaced revisions. Shed guest load independently of native providers.

| Metric                | Initial acceptance gate                                                                                                                                                                                                                                                                                               |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Input and local query | Input dispatch p95 ≤1 ms and no waits on guest work. Warm local pipeline p95 ≤5 ms and p99 ≤10 ms, including host validation and filtering. Static lists run zero guest calls per key. Measure keypress to frame separately and compare with the native baseline.                                                     |
| Cold and I/O          | First local result p95 ≤20 ms. Measure broker latency separately and verify that typing and native results still progress with a stalled operation.                                                                                                                                                                   |
| Size and memory       | Target ≤1 MiB incremental signed-app cost for the JS engine and adapter, with wasmi present in both comparison builds. Report the replacement delta after removal and the ZIP and Linux closure separately. No live idle contexts. Measure idle and peak physical footprint, cache eviction and repeated create/drop. |
| Containment           | Prove that busy loops, expensive native-backed JS operations, allocation spikes, promise storms and oversized values terminate within the applicable limits. Test cancellation, denied access, stale actions and revocation during work. Do not enable third-party packages until these controls pass.                |

Check in a pinned experiment modeled on `experiments/wasm-app-size`. Compare signed release builds on the same host, toolchain and configuration. Replay a word counter, a realistic mocked broker flow and a large cached list. Sample at least 1,000 warm calls, cold calls from fresh processes, and idle states with no extensions and with 64 installed packages. Report p50/p95/p99, dispatch, queuing, execution, validation, rendering, peak footprint and bundle bytes. Include a clean-checkout agent exercise using only the scaffold, declarations and diagnostics. [Measurement precedent](../../experiments/wasm-app-size/README.md)

## Cross-platform notes

Use the same QuickJS-ng release, schema, fixtures and limits on macOS and Linux X11. Build and run the fixture suite on the Linux port before expanding the API. The extension host refuses to run guests on a platform without a sandbox; Linux needs its own confinement for the helper, such as seccomp-bpf with Landlock or namespaces, before it runs third-party packages. Keep executable resolution, file handles, process cleanup, clipboard and permissions in platform adapters. Report unsupported capabilities clearly. macOS uses Dupin.app for TCC. Windows comes later and uses the same guest contract with its own host adapters. [Linux PR #166](https://github.com/x-ryoppippi/dupin/pull/166) · [QuickJS-ng supported platforms](https://quickjs-ng.github.io/quickjs/supported_platforms/)

## Rollout

1. Measure native QuickJS-ng in `experiments/js-runtime`. Gate on size, isolation, end-to-end latency and Linux compatibility. Keep the experiment separate from adopting the runtime.
2. Ship keyworded `search(query)` with the existing row and push semantics, JS scaffold and directory-based `check`/`run`. Add adversarial fixtures and the word-count port. Verify parity for copy, URL open, pushed lists and details, navigation and diagnostics.
3. Port the remaining Wasm examples and fixture coverage and remove wasmi and the guest ABI once parity and release gates pass.
4. Add the author-side `dev`/`build` pipeline and declarations. Share the renderer with `snap` and add fixture-driven `test` as a separate PR. Expose those fixture operations through MCP once they are stable.
5. Adapt apps and files to native sources and kinds. Add cached `items()` and shared matchers with default-route regression coverage. Port other built-ins individually.
6. Add the fixed UI in small slices, starting with sections and Detail, then host-held page state, Form and action outcomes.
7. Ship the broker with a scoped HTTP read and its full consent, cancellation, revocation and Permissions page flow. Add scoped files and executable profiles with `@script` targets in separate PRs. Consider clipboard and outbound MCP after v1.
8. Add advanced configuration with native filters and actions first. Enable third-party kinds and filters only after cross-producer scopes and limits are enforced. Add Git installs, the lock file, updates, the Home Manager module and topic-based discovery.

## Open questions

- Who owns security patching for the QuickJS-ng pin? The engine cannot bound every native-backed operation, so the confined helper is the boundary; an engine flaw reached while a guest runs now needs a sandbox escape as well to reach the user's data. The parser, regular-expression compiler and module linker still run on package source inside Dupin when it loads the folder, so a front-end flaw is not contained until compilation moves into the helper.
- Which real extensions need large or incremental static catalogues, secure credential references, or curation beyond topic search? Use those use cases to set the next API boundaries.

## Appendix · Agreement and differences

| Topic           | Fable                                               | Astra, revised                                                                                 | Opus                                                                                                       | Synthesis                                                                                                                                       |
| --------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime         | Native QuickJS-ng, remove wasmi at parity.          | Native QuickJS-ng, retain Wasm for existing and compute guests.                                | Native QuickJS-ng via `rquickjs` at `-O3`, remove wasmi before release (net +0.84 MiB with a TS stripper). | Native QuickJS-ng with a measured isolation gate, remove wasmi at parity. Revisit the Wasm boundary if native controls fail.                    |
| TypeScript      | Host-side oxc erasure.                              | Author-side build and pinned declarations.                                                     | Host-side erasure using `swc_ts_fast_strip`.                                                               | Owner decided author-side Bun/esbuild and `dupin.d.ts`. Optional host stripping (oxc or `swc_ts_fast_strip`) only if a later need justifies it. |
| Views and state | Named views and actions with host-held context.     | Optional event reducer with correlated broker completion.                                      | Named views/actions with host-held JSON and a guest job queue.                                             | Start with named handlers, state in `Session` and a bounded promise bridge.                                                                     |
| Authority       | Simple manifest grants, trusted tools, inbound MCP. | Scoped executable profiles, resource binding, cross-producer grants and optional outbound MCP. | Reuse policy invalidation, allow named executables, inbound MCP.                                           | V1 HTTP, exec including scripts, and files. Dupin stores per-install grants and invalidates outstanding work. Outbound MCP is later.            |
| Product shape   | Host-owned data views and native built-ins.         | Same, with explicit field projections and additive advanced configuration.                     | Same, including cached static lists and shared native filters.                                             | One simple entry point, native matching for cached `items()`, feature-oriented Settings and opt-in composition.                                 |
