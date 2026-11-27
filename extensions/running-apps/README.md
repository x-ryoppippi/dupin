# Running Apps

Lists the apps that run now, and hides, quits or force quits one, from its own list or from any app row's actions.

| Keyword       | `quit`                                              |
| ------------- | --------------------------------------------------- |
| Command       | `running-apps`                                      |
| On by default | No: turn it on in Settings → Extensions             |
| User docs     | [Running apps](../../docs/features/running-apps.md) |

## How it's built

- **`items()`**: `apps.running()` as one row per app, with its bundle ID as the row's `id` and a `keywords` entry, its bundle as `file` for the icon, `Active` and `Hidden` tags, and a `ttl` of 2 seconds so Dupin refreshes it while the panel shows it.
- **Actions:** **Quit** (<kbd>↩</kbd>), **Hide** and **Force Quit…**, which asks to `confirm`, each a `run` action that calls `apps.quit()`, `apps.hide()` or `apps.forceQuit()` with the app's path, invalidates the catalogue and returns a `toast` with what happened. **Reveal in Finder** is a plain action.
- **Kind actions (`dupin.provides`):** `hide`, `quit` and `quit_forcibly` on the `app` kind are the same handlers. Each declares `default`, so Dupin composes it onto every app row while the package is on, and `when: "running"`, so the action screen offers it only while the app runs; `quit_forcibly` declares `confirm`. They receive the row's `path`.
- **Capabilities:** `apps: ["read", "control"]`, for listing and for hiding and quitting, which only an action you chose may do; `rows: { app: ["path"] }` for the kind actions. None needs macOS access.

## Files

| Path              | What                                                        |
| ----------------- | ----------------------------------------------------------- |
| `package.json`    | The manifest, its capabilities and its kind actions.        |
| `main.ts`         | `items` and the actions.                                    |
| `fixtures/*.json` | The list, consent, Hide, and Quit with an app that refuses. |

## Test it

```sh
dupin ext test extensions/running-apps
dupin ext render extensions/running-apps extensions/running-apps/fixtures/list.json list.png
```

Each fixture lists the running apps in `replies.apps.running`; hiding one marks it hidden and quitting one ends it, except an app whose PID `replies.apps.refuse` lists, which only Force Quit ends, so nothing on the Mac is touched. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
