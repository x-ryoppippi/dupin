# Script commands

Install executable scripts in `$XDG_CONFIG_HOME/dupin/scripts/`, or `~/.config/dupin/scripts/` when `XDG_CONFIG_HOME` is unset. Dupin searches their titles alongside applications. Scripts run only when you activate a result or choose Run again; discovery, searching and reloading do not execute them.

```sh
#!/bin/sh
# @dupin.title Echo a message
# @dupin.output inline
# @dupin.argument message
# @dupin.argument note?
printf '%s\n' "$@"
```

Make the file executable with `chmod +x`. Enter on a script with arguments opens an argument field. Tab or Right at the end of the search field opens the same screen. Enter runs valid arguments; Escape, or Backspace in an empty argument field, returns to the search.

## Headers and arguments

The optional header keys are `title`, `icon`, `output` and repeatable `argument`, each prefixed with `@dupin.` in a comment. The title defaults to the filename without its extension, the icon defaults to the script file, and output defaults to `inline`. Relative icon paths resolve beside the script; `~` resolves to your home directory.

Argument declarations use `name` for a required value, `name?` for an optional value, `name...` for zero or more values, and `name+` for one or more. Put required arguments before optional ones and a variadic argument last.

Quotes group input into arguments. Backslashes escape characters outside quotes; double quotes allow escaped quotes and backslashes. Dupin passes the resulting values directly as argv. It does not expand variables, globs or `~`, and it does not invoke a shell to interpret the argument field. The limit is 64 arguments and 64 KiB of argument text.

Headers use comment prefixes `#`, `//`, `--` or `;`. Scanning stops at code and reads at most 8 KiB or 64 lines. Dupin discovers up to 200 executable files at the top level, including symlinks, and ignores hidden files and editor backups. Settings → Search shows script discovery warnings for non-executable files, invalid headers and unknown metadata keys.

## Output and cancellation

| Mode     | Result                                                                                                                                                                               |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `silent` | Hide the panel. A failed start or non-zero exit opens Script Output with the error.                                                                                                  |
| `inline` | Keep the panel open. After completion, Enter copies the last non-empty stdout line and closes the panel. Command-O shows full output; typing a new search or hiding clears the card. |
| `full`   | Open Script Output and stream stdout and stderr.                                                                                                                                     |

Command-Period cancels a running script. If an inline run fails after its card is cleared, Script Output opens with the error. Run again (Command-R) selects the new run in Script Output. The window also offers Cancel and Copy output (Shift-Command-C) and retains this session's last 20 runs. Output is bounded to 1 MiB per run and 8 KiB per line; old lines are dropped with a line-count marker in the window and copied output, and terminal SGR sequences are removed. The window displays up to the last 200 lines or 64 KiB of a run. Copy output includes all retained lines. Non-zero exits retain the exit status and stderr.

At most eight scripts run concurrently, with no execution timeout. When the script process exits, Dupin collects remaining output for up to 100 milliseconds, closes its end of the output pipes and releases the run slot. Background processes can continue on their own. Redirect their output within the script if they need to write after it exits.

Cancellation sends TERM to the owned process group, then KILL when the script process exits or after two seconds, whichever comes first. Dupin immediately kills its owned groups when quitting or restarting via Relaunch or Restart to Update. Automatic updates wait for active work before restarting. Dupin owns a group until it reaps the script process. Processes that leave their group, outlive a normally completed script or survive a crash of Dupin are outside that cleanup boundary.

## Interpreters and environment

Scripts run directly using their shebang, with your home directory as the working directory and stdin closed. Dupin inherits its process environment; it does not run login-shell setup to discover PATH. Use an absolute interpreter path or set PATH explicitly when a tool is unavailable to a launched application.

Any installed interpreter can be used, including Nushell, Python and Bun. A script can also invoke a long-running tool such as `claude -p`; choose `full` to follow its output. These commands execute only on activation, with the same user access as Dupin.

The following examples use `env` to find installed tools. Replace the interpreter in the shebang, or the CLI command, with its absolute path if the application environment cannot find it. Save each example as its own executable file in the scripts directory.

### Nushell

```nu
#!/usr/bin/env nu
# @dupin.title Uppercase a message
# @dupin.argument message
def main [message: string] {
    $message | str upcase
}
```

### Python

```python
#!/usr/bin/env python3
# @dupin.title Count characters
# @dupin.argument message
import sys

print(len(sys.argv[1]))
```

### Bun

```javascript
#!/usr/bin/env bun
// @dupin.title Join words
// @dupin.argument words+
console.log(process.argv.slice(2).join(' '));
```

### Stream an AI command

```sh
#!/bin/sh
# @dupin.title Ask Claude
# @dupin.output full
# @dupin.argument prompt
exec claude -p "$1"
```

The prompt is one argument: quote a multiword prompt in the argument field. The command runs with your installed CLI's own configuration and access; Dupin supplies no credentials or additional permissions.

Scripts are trusted, unsandboxed native programs, separate from the extension permission system. Installing a script gives its code your user authority when you run it. Script discovery and enabling do not request macOS permissions.
