<p align="center">
    <img src="./assets/logo/dupin.svg" alt="Dupin" width="128" height="128">
</p>

<h1 align="center">Dupin</h1>

A lightweight, resident launcher for Apple silicon Macs, built with [Rune](https://github.com/anomalyco/rune).

Homepage: <https://dupin.app>

<a href="https://dupin.app"><img src="./docs/public/website.png" alt="dupin.app: Press ⌘Space. It's there." width="720"></a>

Named after C. Auguste Dupin, Poe's detective: the source of the line of Mac launchers that runs Sherlock → Watson → Alfred.

- Fuzzy-search and open apps, by any name macOS knows them by
- Space on an empty query searches files with Spotlight; `in` and Space search inside them
- A hotkey (⌘Space by default) shows the panel at once, on Liquid Glass

## Quick start

Free ⌘Space first: turn off Spotlight's shortcut and unassign it in any other launcher ([how](./docs/configuration.md#freeing-the-hotkey)). Then install it with Homebrew (`dupin@nightly` for builds of `main`); it [updates itself](./docs/configuration.md#updates):

```sh
brew install --cask x-ryoppippi/tap/dupin
```

Or build it with Nix:

```sh
nix build .#dupin
open result/Applications/Dupin.app
```

## Documentation

- [Using Dupin](./docs/usage.md): keys, app search, file and content search
- [Configuration](./docs/configuration.md): `config.toml`, the home-manager module, the hotkey and what happens when it's taken
- [Development](./docs/development.md): building, the Nix layout, formatting, headless renders, peak memory
- [Architecture](./docs/architecture.md): crates, providers, the session and the panel
- [Working around Rune](./docs/rune-workarounds.md)
- [Releasing](./docs/releasing.md): tagpr and CI

## Major Sponsors

<p align="center">
    <a href="https://coderabbit.link/ryoppippi">
        <picture>
            <source media="(prefers-color-scheme: dark)" srcset="./docs/public/coderabbit-logo-dark.svg">
            <img src="./docs/public/coderabbit-logo.svg" alt="CodeRabbit" width="320">
        </picture>
    </a>
    &nbsp;&nbsp;&nbsp;&nbsp;
    <a href="https://blacksmith.sh">
        <picture>
            <source media="(prefers-color-scheme: dark)" srcset="./docs/public/blacksmith-logo-dark.svg">
            <img src="./docs/public/blacksmith-logo.svg" alt="CI powered by Blacksmith" width="320">
        </picture>
    </a>
</p>

## GitHub Sponsors

<p align="center">
    <a href="https://github.com/sponsors/ryoppippi">
        <img src="https://sponsors.ryoppippi.com/sponsors.png" alt="Sponsors">
    </a>
</p>

## License

Dupin is licensed under [GPL-3.0-or-later](./LICENSE). Contributions need the [Contributor License Agreement](./CLA.md); see [CONTRIBUTING.md](./CONTRIBUTING.md).
