# Script commands

Dupin runs your own executable scripts, with arguments, from search or a hotkey.

## Open it

Put an executable script (`chmod +x`) in `$XDG_CONFIG_HOME/dupin/scripts/`, or `~/.config/dupin/scripts/` when `XDG_CONFIG_HOME` is unset, then type its title in the main search, press its [hotkey](#hotkeys), or choose Run again.

```sh
#!/bin/sh
# @dupin.title Echo a message
# @dupin.output inline
# @dupin.argument message
# @dupin.argument note?
printf '%s\n' "$@"
```

## What you see

- The script's title in search, alongside applications.
- For a script with arguments, a separate argument field.
- Its output, by its `output` mode: an inline card in the panel (`inline`), the Script Output window (`full`), or nothing unless it fails (`silent`).

## What you can do

In search and the argument field:

| Key / action                                                | What it does                                                     |
| ----------------------------------------------------------- | ---------------------------------------------------------------- |
| <kbd>↩</kbd>                                                | Run the script, or open its argument field if it takes arguments |
| <kbd>⇥</kbd> or <kbd>→</kbd> at the end of the search field | Open the argument field                                          |
| <kbd>↩</kbd> in the argument field                          | Run with valid arguments                                         |
| <kbd>Esc</kbd>, or <kbd>⌫</kbd> in an empty argument field  | Return to the search                                             |

With an inline card:

| Key / action            | What it does                                                                    |
| ----------------------- | ------------------------------------------------------------------------------- |
| <kbd>↩</kbd>            | After a successful run, copy the last non-empty stdout line and close the panel |
| <kbd>⌘O</kbd>           | Show the full output in Script Output                                           |
| <kbd>⌘.</kbd>           | Cancel the running script                                                       |
| Type, or hide the panel | Clear the card                                                                  |

In the Script Output window:

| Key / action                     | What it does                        |
| -------------------------------- | ----------------------------------- |
| <kbd>⌘.</kbd> (**Cancel**)       | Cancel the running script           |
| <kbd>⌘R</kbd> (**Run again**)    | Run it again and select the new run |
| <kbd>⌘⇧C</kbd> (**Copy output**) | Copy all retained lines             |
| <kbd>Esc</kbd>                   | Close the window                    |

Other keys: [Keys](../usage.md#keys).

## Notes

- Scripts are trusted, unsandboxed native programs, separate from the extension permission system. Installing a script gives its code your user authority when you run it. Script discovery and enabling do not request macOS permissions.
- Discovery, searching and reloading never execute a script.
- Scripts run with your home directory as the working directory and stdin closed. Dupin inherits its process environment; it does not run login-shell setup to discover PATH. Use an absolute interpreter path or set PATH explicitly when a tool is unavailable to a launched application.
- At most eight scripts run at once, with no execution timeout.
- Settings → Search shows script discovery warnings for non-executable files, invalid headers and unknown metadata keys.

## Configuration

Executable script commands live beside the configuration in the `scripts` directory. Their headers define their titles, arguments and output mode; no `config.toml` key is required.

### Headers and arguments

Header keys are optional, each prefixed with `@dupin.` in a comment (`#`, `//`, `--` or `;`).

| Key        | Default                            | Meaning                                                                               |
| ---------- | ---------------------------------- | ------------------------------------------------------------------------------------- |
| `title`    | The filename without its extension | The title search matches                                                              |
| `icon`     | The script file                    | An icon path; relative paths resolve beside the script, `~` to your home directory    |
| `output`   | `inline`                           | `silent`, `inline` or `full`; see [Output and cancellation](#output-and-cancellation) |
| `argument` |                                    | Repeatable; one per argument, as below                                                |

| Argument  | Takes               |
| --------- | ------------------- |
| `name`    | A required value    |
| `name?`   | An optional value   |
| `name...` | Zero or more values |
| `name+`   | One or more values  |

Put required arguments before optional ones and a variadic argument last.

### Hotkeys

Use a [script binding](./hotkeys.md#configuration) to run a script by its filename. Each string in `args` becomes one argument. Omit `args` for a script that can run without arguments. Dupin checks the filename and arguments whenever the scripts or configuration change.

The script's output mode controls what the shortcut opens: `inline` opens the panel with the inline card, `full` opens Script Output, and `silent` opens Script Output on failure.

### Output and cancellation

| Mode     | Result                                                                                                                   |
| -------- | ------------------------------------------------------------------------------------------------------------------------ |
| `silent` | Hide the panel. A failed start or non-zero exit opens Script Output with the error.                                      |
| `inline` | Keep the panel open with an inline card. If the run fails after its card is cleared, Script Output opens with the error. |
| `full`   | Open Script Output and stream stdout and stderr.                                                                         |

Script Output retains this session's last 20 runs.

<details>
<summary>How the argument field is parsed</summary>

Quotes group input into arguments. Backslashes escape characters outside quotes; double quotes allow escaped quotes and backslashes. Dupin passes the resulting values directly as argv. It does not expand variables, globs or `~`, and it does not invoke a shell to interpret the argument field. The limit is 64 arguments and 64 KiB of argument text.

</details>

<details>
<summary>Discovery limits</summary>

Header scanning stops at code and reads at most 8 KiB or 64 lines. Dupin discovers up to 200 executable files at the top level, including symlinks, and ignores hidden files and editor backups.

</details>

<details>
<summary>Output limits</summary>

Output is bounded to 1 MiB per run and 8 KiB per line; old lines are dropped with a line-count marker in the window and copied output, and terminal SGR sequences are removed. The window displays up to the last 200 lines or 64 KiB of a run. Copy output includes all retained lines. Non-zero exits retain the exit status and stderr.

</details>

<details>
<summary>Concurrency and process cleanup</summary>

When the script process exits, Dupin collects remaining output for up to 100 milliseconds, closes its end of the output pipes and releases the run slot. Background processes can continue on their own. Redirect their output within the script if they need to write after it exits.

Cancellation sends TERM to the owned process group, then KILL when the script process exits or after two seconds, whichever comes first. Dupin immediately kills its owned groups when quitting or restarting via Relaunch or Restart to Update. Automatic updates wait for active work before restarting. Dupin owns a group until it reaps the script process. Processes that leave their group, outlive a normally completed script or survive a crash of Dupin are outside that cleanup boundary.

</details>

<details>
<summary>Interpreters and examples</summary>

Scripts run directly using their shebang. Any installed interpreter can be used, including Nushell, Python and Bun. A script can also invoke a long-running tool such as `claude -p`; choose `full` to follow its output. These commands execute only on activation, with the same user access as Dupin.

The examples below use `env` to find installed tools. Replace the interpreter in the shebang, or the CLI command, with its absolute path if the application environment cannot find it. Save each example as its own executable file in the scripts directory.

Nushell:

```nu
#!/usr/bin/env nu
# @dupin.title Uppercase a message
# @dupin.argument message
def main [message: string] {
    $message | str upcase
}
```

Python:

```python
#!/usr/bin/env python3
# @dupin.title Count characters
# @dupin.argument message
import sys

print(len(sys.argv[1]))
```

Bun:

```javascript
#!/usr/bin/env bun
// @dupin.title Join words
// @dupin.argument words+
console.log(process.argv.slice(2).join(' '));
```

Stream an AI command:

```sh
#!/bin/sh
# @dupin.title Ask Claude
# @dupin.output full
# @dupin.argument prompt
exec claude -p "$1"
```

The prompt is one argument: quote a multiword prompt in the argument field. The command runs with your installed CLI's own configuration and access; Dupin supplies no credentials or additional permissions.

</details>
