# Repositories

Lists the Git repositories on your Mac and opens one in your editor, your terminal, Finder or its page on the web.

| Keyword       | `repo`                                      |
| ------------- | ------------------------------------------- |
| Command       | `repo`                                      |
| On by default | No: turn it on in Settings → Extensions     |
| User docs     | [Repositories](../../docs/features/repo.md) |

Never native: it was written as an optional bundled extension.

## How it's built

The bundled example of `files` scopes, declared preferences and state kept between refreshes.

- **`items(ctx)`**: gathers the repositories `ghq list -p` prints and the folders holding `.git` under the four `files` scopes, reads Git's status for up to `checks` of them within the refresh's time budget, and lists them in parts of 500 with a `ttl` of 300 seconds. Dupin matches each keystroke against the catalogue it holds and asks again in the background once the `ttl` passes.
- **`storage`**: one record, under the key `state`, holds every repository listed and what Git last said of each, so a refresh keeps the status of repositories it didn't read this time and the actions can check that a path is one it listed.
- **Actions:** **Open in Editor** (<kbd>↩</kbd>) and **Open in Terminal** (<kbd>⌘↩</kbd>, `alternate`) are `run` handlers that start the app chosen in preferences; **Reveal in Finder**, **Open on GitHub** (or the remote's host), **Copy Path** and **Copy Remote URL** are plain actions.
- **Preferences:** the editor and terminal, whether to list what ghq lists, which of the four folders to scan, how deep, and how many repositories' Git status to read per refresh.
- **Capabilities:**
  - `files`: `ghq`, `src`, `developer` and `projects`, `list` access to `~/ghq`, `~/src`, `~/Developer` and `~/Projects`. A folder the Mac doesn't have is skipped without asking.
  - `exec`, each run without a shell:

    | Profile           | Runs                                                                                 | Why                                                                    |
    | ----------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- |
    | `ghq-list`        | `ghq list -p`; no `program`, so you pick ghq when you allow it                       | The repositories ghq manages. `background`.                            |
    | `git-status`      | `/usr/bin/git … status --porcelain=v2 --branch`, without optional locks or fsmonitor | Branch, ahead and behind, and changed files. `background`.             |
    | `git-last-commit` | `/usr/bin/git -C {path} log -1 --format=%ct`                                         | When the last commit was. `background`.                                |
    | `git-remote`      | `/usr/bin/git -C {path} config --get remote.origin.url`                              | The remote, for Open on the web and Copy Remote URL. `background`.     |
    | `open-app`        | `/usr/bin/open -b {app} {path}`                                                      | Opens the repository in the chosen editor or terminal, from an action. |

    `background` lets a profile run in `items()` while you type, not only in an action you chose.

  - `storage: { quotaBytes: 4194304 }`: 4 MiB for the state record.

  Each folder and program is allowed on its own, since each is bound to a path, and a source that isn't allowed is left out rather than failing the list.

## Files

| Path              | What                                                                                                        |
| ----------------- | ----------------------------------------------------------------------------------------------------------- |
| `package.json`    | The manifest, its preferences and its scopes.                                                               |
| `main.ts`         | Discovery, the Git reads, the rows, the actions and the stored state.                                       |
| `fixtures/*.json` | `repos.json`: ghq and a folder, with Git read. `folders.json`: folders alone, with ghq and Git not allowed. |

## Test it

```sh
dupin ext test extensions/repo
dupin ext render extensions/repo extensions/repo/fixtures/repos.json repos.png
```

`replies.files` gives each scope's folder contents, reported under `/Users/dupin`; `replies.exec` answers each profile by name and arguments, so nothing runs; `folders.json` leaves ghq undecided and denies Git with `replies.consent`. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
