# Repositories

An [example extension](./extensions.md#example-extensions) that lists the Git repositories on your Mac and opens one in your editor, your terminal, Finder or its page on the web. It doesn't ship with Dupin; install it from [`examples/extensions/repo`](../../examples/extensions/repo).

## Install it

```sh
git clone https://github.com/x-ryoppippi/dupin
dupin ext install ./dupin/examples/extensions/repo
```

Then turn on the extensions folder and **Repositories** in Settings → Extensions, allow the folders and programs it asks for, and type `repo` <kbd>Space</kbd> and part of a repository's name, such as `ccusage` or `ryoppippi/ccusage`.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/repo/list-dark.png" />
<img alt="Repositories before anything is typed: dupin, ccusage, notes and scratch, dupin selected with its branch feat/repo two commits ahead, two changes, and its path, remote and last commit." src="../images/repo/list-light.png" width="680" />
</picture>

## What it does

- It lists every repository `ghq list` prints, and every folder holding `.git` in `~/ghq`, `~/src`, `~/Developer` and `~/Projects`, newest commit first. A repository whose Git status it has read also shows its branch, its distance from its upstream and its changes.
- <kbd>↩</kbd> opens the repository in your editor and <kbd>⌘↩</kbd> in your terminal. <kbd>→</kbd> or <kbd>⌘K</kbd> lists **Reveal in Finder**, **Open on GitHub**, **Copy Path** and **Copy Remote URL**.
- Typing never runs Git: Dupin matches the list the extension handed it, and the extension lists again every five minutes, reading the Git status of a few repositories each time.
- Its settings in Settings → Extensions choose the editor, the terminal, the folders and how deep to look. Each folder and program is allowed on its own, and the extension keeps working without any one of them. The [package's README](../../examples/extensions/repo/README.md) lists each scope and how the extension is built.

> [!TIP]
> Used to `ghq`? Add an [alias](./keywords.md#aliases), and `ghq` <kbd>Space</kbd> opens it as `repo` does:
>
> ```toml
> [aliases]
> ghq = "repo"
> ```
