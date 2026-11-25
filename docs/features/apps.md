# Apps

Executable [script commands](./scripts.md) also appear in normal search. Scripts with arguments open a separate argument field; their output can stay inline or stream into the Script Output window.

Dupin finds the `.app` bundles in `/Applications`, `/System/Applications` and `~/Applications`, one folder deep (so `Utilities` is included). An app matches by any name macOS knows it by:

- the name Finder shows, localized (システム設定 for System Settings on a Japanese Mac);
- its file name, when that differs;
- Spotlight's alternate names and keywords: ChatGPT answers to "Codex", its former name, System Settings to "System Preferences", Safari to "browser".

When an app matches through one of those other names, the name shows before its folder in the result.

A name the query starts, or that has a word the query starts (`ar` for Ardia, `time` for FaceTime), ranks above every name that only holds the query's letters further apart or inside a word (`ar` in Calendar, or in Apple Radio by its initials). Within each of those, closer matches come first, and usage moves up the apps you open most and those you opened for the same query. However often you opened a looser match, it never hides an app whose name starts with what you typed.
