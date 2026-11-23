# Permissions

Settings → Permissions shows which features use each grant and what macOS has allowed. Choose **Request Access** to ask for a grant for the first time, even while its feature is off. Choose **Open System Settings** to manage an existing decision. Dupin refreshes the status when the Calendar request finishes and when Settings regains focus.

Turning on **Calendar search**, **Window Management** or **Paste snippets**, or recording a **Selection hotkey** for the first time, requests the corresponding access when needed. You can also be prompted on first use if you enabled a feature through `config.toml`. Starting Dupin, opening Settings and reloading configuration only check access. File search uses the Spotlight index and needs no grant.
