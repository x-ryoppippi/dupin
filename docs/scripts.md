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

Headers use comment prefixes `#`, `//`, `--` or `;`. Scanning stops at code and reads at most 8 KiB or 64 lines. An invalid header skips the script and reports a problem in the panel. Unknown metadata keys produce a warning. Dupin discovers up to 200 executable files at the top level, including symlinks, and ignores hidden files and editor backups.

## Output and cancellation

| Mode     | Result                                                                                                                                                          |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `silent` | Hide the panel. A failed start or non-zero exit opens Script Output with the error.                                                                             |
| `inline` | Keep the panel open. After completion, Enter copies the last non-empty stdout line. Command-O shows full output; typing a new search or hiding clears the card. |
| `full`   | Open Script Output and stream stdout and stderr.                                                                                                                |

Command-Period cancels a running script. Script Output offers Cancel, Run again (Command-R) and Copy output (Shift-Command-C). It retains this session's last 20 runs. Output is bounded to 1 MiB per run and 8 KiB per line; old lines are dropped with a line-count marker in the window and copied output, and terminal SGR sequences are removed. Non-zero exits retain the exit status and stderr.

At most eight scripts run concurrently, with no execution timeout. Cancellation sends TERM to the owned process group, then KILL after two seconds if needed. Quitting Dupin immediately kills its owned groups; an update relaunch waits for active work instead. Processes that leave their group, or survive a crash of Dupin, are outside that cleanup boundary.

## Interpreters and environment

Scripts run directly using their shebang, with your home directory as the working directory and stdin closed. Dupin inherits its process environment; it does not run login-shell setup to discover PATH. Use an absolute interpreter path or set PATH explicitly when a tool is unavailable to a launched application.

Any installed interpreter can be used, including Nushell, Python and Bun. A script can also invoke a long-running tool such as `claude -p`; choose `full` to follow its output. These commands execute only on activation, with the same user access as Dupin.

Scripts are trusted, unsandboxed native programs, separate from the extension permission system. Installing a script gives its code your user authority when you run it. Script discovery and enabling do not request macOS permissions.
