# Remote

Dupin lists the processes, listening ports, recent log lines and files of the machines you reach over SSH, from what it last fetched, so typing never waits on the network.

## Open it

Switch on **Remote** in Settings → Extensions and allow its scopes there (see [Configuration](#configuration)), then type `remote` <kbd>Space</kbd>. Type a host's name after it, such as `remote web1`, and a sub-scope after that: `ps`, `ports`, `logs` or `files`, then any words, such as `remote web1 ps nginx`.

## What you see

- **Hosts**: each host you enabled, with how it answered last (`online`, `unreachable`, `auth failed`, `host key`, `timed out`) and how long ago Remote fetched it.
- **Other Hosts**: the hosts in `~/.ssh/config` and its `Include` files that you haven't enabled, and those you added in Remote Settings. Patterns such as `*.internal` are left out.
- **Remote**: Remote Settings, Refresh All Hosts and Forget Fetched Results.
- Then each enabled host's sections, such as **web1 · Processes**, **web1 · Ports**, **web1 · nginx.service** and **web1 · /srv/app**. Every row starts its subtitle with how long ago it was fetched, and a host that didn't answer last time marks its rows `unreachable` while they still show what it had.

Typing keeps the rows that hold every word, in this order: `remote web1 ports 5432`, `remote db logs error`.

## What you can do

| Key / action                        | On                              | What it does                                                                                                    |
| ----------------------------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| <kbd>↩</kbd> **Show _host_**        | A host                          | Opens the host on a page of its own; type there to narrow it, as after `remote web1`                            |
| <kbd>⌘↩</kbd> **Refresh _host_**    | A host                          | Fetches the host again over SSH; the panel stays usable while it runs                                           |
| <kbd>⌘↩</kbd> **Open SSH Session**  | A process, port, log line, file | Opens `ssh://host` in Terminal or iTerm, as Remote Settings choose; a host offers it too                        |
| <kbd>↩</kbd> **Copy**               | A process, port, log line, file | Copies the command line, the address, the line or the path                                                      |
| **Copy PID**, **Copy as host:path** | A process, a file               | Copies the PID, or `host:/path` for `scp`                                                                       |
| **Show Details**                    | Any row                         | Shows everything Remote has of it, a command line or log line in full                                           |
| **Terminate**                       | A process                       | Sends `TERM` to that PID after you confirm; offered only on hosts whose settings allow it                       |
| **Configure _host_…**               | A host                          | Turns fetching on or off and chooses log units, log files, folders, log length and whether Terminate is offered |
| <kbd>↩</kbd> **Enable _host_**      | An other host                   | Turns the host on and fetches it                                                                                |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off by default.
- Remote never connects on its own: a host is fetched when you choose Enable, Refresh or Refresh All Hosts, and typing only matches what it fetched last.
- It uses your own `ssh`, configuration, keys and SSH agent, in batch mode: a host that needs a password, a passphrase prompt or a host key you haven't accepted fails instead of asking. Connect once in Terminal to accept a new host key.
- Processes come from `ps`, which Linux and macOS hosts both have; ports from `ss` and units from `journalctl`, which are Linux's. A host without one shows why under its section.
- Remote keeps what it fetched in its own encrypted storage until the next fetch replaces it, or Forget Fetched Results or Configure deletes it.

## Configuration

Remote has no setting in `config.toml`. Turn it on in Settings → Extensions, then allow its scopes there; each scope you leave off only turns off what needs it.

| Scope               | Pick                                                            | Lets Remote                                                |
| ------------------- | --------------------------------------------------------------- | ---------------------------------------------------------- |
| `files:ssh-config`  | `~/.ssh/config` (press <kbd>⌘⇧G</kbd> and type the path)        | List its hosts                                             |
| `files:ssh-include` | The folder your `Include` lines name, such as `~/.ssh/config.d` | List the hosts in the files those lines match              |
| `exec:processes`    | `/usr/bin/ssh`                                                  | Run `ps` on a host                                         |
| `exec:ports`        | `/usr/bin/ssh`                                                  | Run `ss` on a host                                         |
| `exec:journal`      | `/usr/bin/ssh`                                                  | Run `journalctl -u` on a host for a unit you configured    |
| `exec:log-file`     | `/usr/bin/ssh`                                                  | Run `tail` on a host for a file you configured             |
| `exec:files`        | `/usr/bin/ssh`                                                  | Run `find` on a host in a folder you configured            |
| `exec:terminate`    | `/usr/bin/ssh`                                                  | Run `kill -s TERM` on a host, only for a process you chose |
| `exec:terminal`     | `/usr/bin/open`                                                 | Open `ssh://host` in your terminal app                     |
| `storage`           |                                                                 | Keep your host settings and what it fetched, encrypted     |

**Remote Settings** choose the terminal app and add hosts beside those in `~/.ssh/config`, one a line, such as `deploy@build.example.com`. **Configure _host_** chooses, for that host, up to four systemd units, four log files and four folders, how many log lines to keep (1–200, 50 by default) and whether Terminate is offered. Unit names and paths take only letters, digits and `. _ / @ : + , = -`, and paths start with `/`.

You can turn Remote on ahead of time in `extension-grants.toml` beside `config.toml` ([Declaring them with Nix](./extensions.md#declaring-them-with-nix)); its files and programs are still picked in Settings:

```toml
version = 1

[packages."bundled:remote"]
enabled = true
```

<details>
<summary>Exactly what runs on a host</summary>

Every command is fixed in Remote's `package.json`, so consent shows each one in full; Remote supplies only the values in braces, and checks each before Dupin does. `ssh` gets the same options first each time:

```text
ssh -n -T -a -x -oBatchMode=yes -oConnectTimeout=5 -oServerAliveInterval=5 -oServerAliveCountMax=2
    -oStrictHostKeyChecking=yes -oUpdateHostKeys=no -oClearAllForwardings=yes -oPermitLocalCommand=no
    -oRemoteCommand=none -oControlMaster=no -oLogLevel=ERROR {host} -- <command>
```

| Profile     | `<command>`                                                   |
| ----------- | ------------------------------------------------------------- |
| `processes` | `ps -A -o pid= -o user= -o pcpu= -o pmem= -o etime= -o args=` |
| `ports`     | `ss -H -l -n -t -u -p`                                        |
| `journal`   | `journalctl --no-pager -q -o short-iso -n {lines} -u {unit}`  |
| `log-file`  | `tail -n {lines} -- {path}`                                   |
| `files`     | `find {dir} -xdev -mindepth 1 -maxdepth 2 -type f`            |
| `terminate` | `kill -s TERM {pid}`                                          |

`ssh` joins everything after `--` into one line that the host's login shell reads, so Remote never builds one from what you type: `{host}` is a host name or `user@host` of letters, digits and `. _ @ -` that can't start with `-`, `{unit}` a unit name, `{path}` and `{dir}` absolute paths of letters, digits and `. _ / @ : + , = -`, and `{lines}` and `{pid}` whole numbers. None of these characters means anything to a shell. Dupin runs `ssh` without a shell, with only `PATH`, `HOME`, `TMPDIR`, `LANG`, `USER` and `SSH_AUTH_SOCK`, stops it after 30 seconds and hands Remote at most 64 KiB of its output.

`terminal` runs `/usr/bin/open -a {app} ssh://{host}`, where `{app}` is `Terminal` or `iTerm`.

</details>

<details>
<summary>What a host can send back</summary>

Remote treats everything a host prints as untrusted text. It reads at most 300 processes, 100 ports, 200 lines of each log and 200 files of each folder, looks at no line past 4,096 characters, drops escape sequences, control characters and invisible or reordering formatting characters, cuts each row to fit and shows a log line in Show Details inside a code block no backtick in it can close, so nothing a host prints becomes a link or formatting. Lines that don't parse are skipped.

</details>

<details>
<summary>How it works and changing it</summary>

Remote is TypeScript in [`extensions/remote`](../../extensions/remote). Its `items()` catalogue reads the SSH configuration through the [`files` capability](./extension-permissions.md#files) and the last fetch from its [`storage`](./extension-permissions.md#storage), and Dupin matches it as you type, so a keystroke runs nothing in the extension. Fetching is an action, run through the [`exec` capability](./extension-permissions.md#exec) with each profile's `sshAgent` set, so `ssh` reaches your agent while Remote never sees a key; each action ends within 30 seconds, and a host that doesn't answer its first command isn't asked the rest.

To change what it runs, eject it and edit the copy:

```sh
dupin ext eject remote
```

A changed copy is your own extension, without the bundled one's switch, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes.

</details>
