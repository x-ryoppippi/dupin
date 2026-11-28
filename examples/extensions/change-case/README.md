# Change Case

A [Dupin](https://github.com/x-ryoppippi/dupin) extension that uses an npm package. Type `case`, a space and some text to see it in camelCase, PascalCase, snake_case, kebab-case and CONSTANT_CASE; Enter copies the selected one.

It is `dupin ext new change-case --bundle` with [`change-case`](https://github.com/blakeembrey/change-case) added by `bun add change-case`, bundled into `dist/main.js` by `dupin ext build .`. `dist/main.js` includes `change-case`, whose MIT licence is in [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md).

## Develop

```sh
bun install
dupin ext dev .
```

`dev` bundles `src/main.ts` with Rolldown, checks the package and publishes a development copy to Dupin's extensions folder whenever a file changes. Turn on **Extensions folder** in Settings → Extensions to try it in the launcher. A change that fails to build or check is reported here, and Dupin keeps the last good copy.

```sh
dupin ext build .
dupin ext check .
dupin ext run . --query 'hello dupin world'
```

## Test

```sh
dupin ext test .
dupin ext render . fixtures/search.json search.png
```

`fixtures/search.json` types a query, chooses an action and states what the extension should answer. `test` replays every fixture in `fixtures/` through the extension host, with the fixture's fixed clock, random seed, locale and time zone, and shows what differs; `--update` rewrites the expectations from what the extension answers now. `render` draws the panel after a fixture's last step as a PNG.

`dupin.icon` in `package.json` is the icon on the keyword chip, in Settings and on rows without one of their own: an SF Symbol, such as `sf:puzzlepiece.extension`, or an image in the package, such as `assets/icon.png`.

[`dupin.d.ts`](./dupin.d.ts) describes the API. Dupin runs JavaScript, and TypeScript once it has erased the types: no Node built-ins, files, network or processes.

## Build

```sh
dupin ext build .
```

Run `bun install` once first: it installs the Rolldown that `package.json` pins and writes `bun.lock`, which you commit. `build` runs that Rolldown with `rolldown.config.ts`, bundling `src/main.ts` and its npm dependencies into one module at `dist/main.js`, and fails if a dependency needs a Node built-in. Commit `dist/main.js`: Dupin runs it as committed. `.github/workflows/build.yml` rebuilds it with the same configuration whenever a push to `main` changes its sources, installing only what `bun.lock` pins, and commits it if it changed.

## Share

Push the repository to GitHub and add the `dupin-extension` topic, which Dupin's discovery lists. Dupin installs from Git, pinned to a commit, and never runs a build or a package manager, so commit the module that `dupin.main` names.
