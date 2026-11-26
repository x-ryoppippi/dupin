# {{title}}

A [Dupin](https://github.com/x-ryoppippi/dupin) extension. Type `{{keyword}}`, a space and a name to greet it; Enter copies the greeting.

## Develop

```sh
dupin ext dev .
```

`dev` {{build_note}}checks the package and publishes a development copy to Dupin's extensions folder whenever a file changes. Turn on **Extensions** in Settings → Search to try it in the launcher. A change that fails to build or check is reported here, and Dupin keeps the last good copy.

```sh
{{build_command}}dupin ext check .
dupin ext run . --query Dupin
```

## Test

```sh
dupin ext test .
dupin ext render . fixtures/search.json search.png
```

`fixtures/search.json` types a query, chooses an action and states what the extension should answer. `test` replays every fixture in `fixtures/` through the extension host, with the fixture's fixed clock, random seed, locale and time zone, and shows what differs; `--update` rewrites the expectations from what the extension answers now. `render` draws the panel after a fixture's last step as a PNG.

`dupin.icon` in `package.json` is the icon on the keyword chip, in Settings and on rows without one of their own: an SF Symbol, such as `sf:puzzlepiece.extension`, or an image in the package, such as `assets/icon.png`.

[`dupin.d.ts`](./dupin.d.ts) describes the API. Dupin runs JavaScript, and TypeScript once it has erased the types: no Node built-ins, files, network or processes.
{{build_section}}
## Share

Push the repository to GitHub and add the `dupin-extension` topic, which Dupin's discovery lists. Dupin installs from Git, pinned to a commit, and never runs a build or a package manager, so commit the module that `dupin.main` names.
