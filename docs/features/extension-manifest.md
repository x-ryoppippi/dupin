# Extension manifest

An extension's manifest is the `dupin` key of its `package.json`, so a package keeps one file for Dupin and its own JavaScript tooling. Dupin reads it as plain JSON before running any code.

```json
{
	"name": "word-count",
	"version": "0.1.0",
	"dupin": { "title": "Word Count", "keyword": "wc" }
}
```

| Key                   | Default   | Meaning                                                                                                                                                                                                  |
| --------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                | Required  | The extension's id: 1–64 lowercase letters, digits or hyphens, unique among extensions. No npm scopes.                                                                                                   |
| `version`             | Required  | 1–32 characters without spaces.                                                                                                                                                                          |
| `dupin.title`         | Required  | Shown on the keyword chip and in Settings, up to 80 characters.                                                                                                                                          |
| `dupin.keyword`       | Required  | Up to 32 bytes, no spaces. Only Calculator, which a built-in hands what the main search types, declares none, so a copy of it leaves it out too.                                                         |
| `dupin.icon`          | None      | `sf:` and an SF Symbol's name, or an image file inside the folder; see [Icons](#icons).                                                                                                                  |
| `dupin.main`          | `main.js` | The module that exports `search` or `items`: a `.js`, `.mjs`, `.ts`, `.mts` or `.tsx` file inside the folder, such as `dist/index.js` or `src/main.ts`.                                                  |
| `dupin.defaultSearch` | `false`   | `true` lists the package's [`items()`](./extension-api.md#itemsctx) catalogue in the [main search](./extension-api.md#the-main-search) too, ranked with apps.                                            |
| `dupin.emptyQuery`    | `false`   | `true` calls [`search`](./extension-api.md#searchquery-ctx) with an empty query as soon as the keyword is typed, for a package with something to show before anything is typed, such as upcoming events. |
| `dupin.match`         | `"fuzzy"` | `"words"` matches the catalogue as a history is searched: see [Matching words](./extension-api.md#matching-words).                                                                                       |
| `dupin.preferences`   | None      | Choices every call receives as `ctx.preferences`; see [Preferences](#preferences).                                                                                                                       |
| `dupin.capabilities`  | None      | What the extension may ask to use; see [Extension capabilities](./extension-permissions.md).                                                                                                             |
| `dupin.provides`      | None      | Filters and kind actions for `[advanced]`; see [Advanced composition](./advanced.md#extensions).                                                                                                         |

> [!NOTE]
> Any other key inside `dupin` is an error. Every other key of `package.json`, such as `scripts`, `dependencies` or `devDependencies`, is ignored.

<details>
<summary>Folders and keywords Dupin skips</summary>

A folder whose `package.json` has no `dupin` key is not an extension, and Settings says so. A keyword that another command already uses (`in`, a Dictionary alias, a web search keyword from config.toml or another extension) is reported in Settings and the extension is skipped; extensions never take a keyword from anything else. Web Search alone still lists its web searches without its keyword.

</details>

### Icons

`dupin.icon` is the package's own icon, on its keyword chip, in Settings → Extensions and on its rows in the main search that name none: `sf:` and an SF Symbol's name (1–64 ASCII letters, digits and dots, such as `sf:puzzlepiece.extension`), or an image file inside the folder as an item's [`icon`](./extension-api.md#searchquery-ctx) takes. A file that isn't there, or a symlink that leads out of the folder, fails the package.

## Preferences

`"preferences"` declares up to 16 choices by name, each with a one-line `title` and a `default`:

```json
"preferences": {
	"compact": { "title": "Compact rows", "type": "checkbox", "default": true },
	"range": { "title": "Range", "type": "select", "options": ["1d", "7d"], "default": "7d" },
	"count": { "title": "Rows", "type": "number", "min": 1, "max": 100, "default": 20 }
}
```

| `type`     | Takes                   |
| ---------- | ----------------------- |
| `checkbox` | `default`               |
| `select`   | `options`, `default`    |
| `number`   | `min`, `max`, `default` |

Every `items`, action and event call receives them as `ctx.preferences`, frozen throughout. A change of preferences drops the catalogue, so the next use lists it with them.

<details>
<summary>Where the values come from</summary>

A built-in feature that ships as a package chooses their values from its own Settings rows; any other package gets its defaults for now. The package Dupin shipped for a built-in that reads config.toml, as [Snippets](./snippets.md) and [Web searches](./web-searches.md) do, also finds that built-in's settings there without declaring them, such as the list of snippets or of web searches; a copy of it gets them only while it is unchanged.

</details>

## Modules and TypeScript

A module can import other `.js` or `.mjs` files of its own folder with relative paths that include the extension (`import { rows } from "./lib/rows.js"`), also `.ts`, `.mts` and `.tsx` files when `dupin.main` is TypeScript, and the host module `dupin`. Nothing else resolves: no packages by name, no URLs, no absolute paths and no path or symlink that leads outside the folder.

TypeScript whose types can simply be erased runs as written; see [TypeScript without a build](#typescript-without-a-build). JSX, TypeScript that needs compiling and npm packages are built on your machine: bundle the extension into one JavaScript module (`dupin ext build` does it with Rolldown; see [Building a bundle](./extension-distribution.md#building-a-bundle)), point `dupin.main` at the output and install the folder. Dupin never type-checks and never runs a package manager or the package's scripts.

### TypeScript without a build

When `dupin.main` is a `.ts`, `.mts` or `.tsx` file, the package's TypeScript files are modules too, and the extension host erases their types as it compiles them. Nothing is type-checked; run `tsc --noEmit` for that.

- Import TypeScript files with their extension, `./util.ts`, as `allowImportingTsExtensions` lets `tsc` accept.
- Import types with `import type` or a `type` specifier. A type imported as a value is kept and fails to link, since nothing exports it; the scaffold's `verbatimModuleSyntax` and `erasableSyntaxOnly` make `tsc` and editors flag both mistakes.
- Declaration files (`.d.ts`, `.d.mts`) are not modules and are never compiled.
- With a JavaScript `dupin.main`, TypeScript files are ignored, so a bundled package can keep its sources beside the bundle.

Bundle with `dupin ext build` when the package uses npm packages, needs syntax that is rejected below, or should ship as one file.

<details>
<summary>What is erased</summary>

Each annotation, interface, type alias, type argument and other type-only part becomes spaces of the same length, line breaks kept, so the line and column of every error and stack frame are the author's own and no source map is needed. Erasing a 2,000-line file takes about 1.3 ms.

Erased: type annotations and return types, `interface`, `type`, type parameters and arguments, `as`, `satisfies`, non-null `!`, `import type`, `export type` and `type` specifiers, `declare`, `abstract` classes and members, overload signatures, `?` and `!` on names, `public`, `private`, `protected`, `readonly` and `override` on class members, index signatures, `implements`, `this` parameters and namespaces that hold only types.

</details>

<details>
<summary>What needs a build</summary>

Rejected with `typescript-needs-build`, the file, line and column, and a hint: `enum` (`const` or not, and `declare const enum`, whose uses a compiler inlines), a namespace or `module` holding values, parameter properties (`constructor(private x)`), decorators, JSX, `import x = require(…)` or `import A = B.C`, and `export =`. Each needs a compiler to produce JavaScript; replace it as the hint says, or bundle the package.

A `<T>value` assertion is rejected too, as `erasableSyntaxOnly` rejects it: blanking a type that ends its line after `return` would end the statement there. Write `value as T` instead. A module whose brackets nest more than 1,000 levels deep fails with `stack-limit` at the bracket that passes the limit.

</details>

## Wasm extensions

Dupin no longer runs WebAssembly extensions. A folder that still holds an `extension.toml`, and no `package.json` with a `dupin` key, is skipped, and Settings → Extensions and `dupin ext check` say `extension.toml: Wasm extensions were removed; port to JavaScript`.

<details>
<summary>Porting one</summary>

A `package.json` that isn't valid JSON or passes 64 KiB reports its own error instead, so a port in progress points at the line to fix. To port one, write its `search` in JavaScript as [Extensions](./extensions.md#a-javascript-extension) shows, or start from `dupin ext new`; once `package.json` has a `dupin` key, a leftover `extension.toml` is ignored. Pushed views have no JavaScript equivalent yet.

</details>
