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

`fixtures/search.json` holds a query and the items `search` should answer with.

[`dupin.d.ts`](./dupin.d.ts) describes the API. Dupin runs JavaScript only: no Node built-ins, files, network or processes.
{{build_section}}
## Share

Push the repository to GitHub and add the `dupin-extension` topic, which Dupin's discovery lists. Dupin installs from Git, pinned to a commit, and never runs a build or a package manager, so commit the JavaScript that `dupin.main` names.
