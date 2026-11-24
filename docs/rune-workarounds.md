# Working around Rune

Rune is pre-release and doesn't take pull requests, so Dupin works around what it hits. Rune is pinned to a `dev` rev in `Cargo.toml` (`dev` is never rewritten); check each item again when bumping it, and patch every crate Rune's own workspace patches at the same rev.

- **Two `tiny-skia-path 0.11.4` packages.** Rune vendors a patched copy as a path dependency while `resvg` keeps the published one. nixpkgs' `importCargoLock` can't vendor both, so `nix/cargo-vendor.nix` uses crane, which vendors each source into its own directory.
- **Vendored crates keep `Cargo.toml.orig`.** Rune's `vendor/` copies published crates whole, and `cargo package --list`, which crane lists a Git crate's files with, rejects that file name, so `nix/cargo-vendor.nix` deletes it from the checkout.
- **Launcher panels fade in.** Rune leaves the panel's `animationBehavior` at AppKit's default, which kept the panel off screen for about 265 ms after its frame was ready. Dupin sets it to none once the panel exists (`dupin_mac::window::appear_instantly`), bringing hotkey-to-screen down to about 20 ms.
- **Hotkey conflicts with macOS's own shortcuts register successfully.** Rune names Spotlight as a chord's owner only after a `Conflict`, and Carbon reports none for it, so Dupin asks `shortcut_owner` before registering.
- **Without `embedded-fonts`, text goes through CoreText's fallback.** Rune treats only the fonts an app hands it as covering text, and a fallback lookup takes about a second cold, even for characters the system font has. Dupin builds Rune without its embedded Inter and hands it San Francisco's file instead (`dupin_mac::font::system_ui_font`), with the family name Rune reads from it (`FAMILIES` in `crates/dupin/src/main.rs`).
- **Headless snapshots don't follow `fit_content_height`.** `--snap` renders at the window's initial size, so `scripts/snap.nu` reads the panel's height from `--tree` first.
- **The headless clock outruns real providers.** Results come from Spotlight and the file system in real time, so headless renders pass `--realtime`, which `--help` lists but Rune's README doesn't.
