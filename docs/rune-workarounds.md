# Working around Rune

Rune is pre-release and doesn't take pull requests, so Dupin works around what it hits. Rune is pinned to a `dev` rev in `Cargo.toml` (`dev` is never rewritten); check each item again when bumping it, and patch every crate Rune's own workspace patches at the same rev.

- **`rune-text` embeds fonts from outside its crate.** It uses `include_bytes!("../../../rune/assets/Inter-*.ttf")`, a path that exists in a Git checkout of the whole repository but not once crates are vendored one by one. The Nix build symlinks the vendored `rune` crate into place (`postPatch` in `package.nix`).
- **Two `tiny-skia-path 0.11.4` packages.** Rune vendors a patched copy as a path dependency while `resvg` keeps the published one. nixpkgs' `importCargoLock` can't vendor both, so `package.nix` uses `fetchCargoVendor`.
- **Launcher panels fade in.** Rune leaves the panel's `animationBehavior` at AppKit's default, which kept the panel off screen for about 265 ms after its frame was ready. Dupin sets it to none once the panel exists (`dupin_mac::window::appear_instantly`), bringing hotkey-to-screen down to about 20 ms.
- **Hotkey conflicts with macOS's own shortcuts register successfully.** Rune names Spotlight as a chord's owner only after a `Conflict`, and Carbon reports none for it, so Dupin asks `shortcut_owner` before registering.
- **Headless snapshots don't follow `fit_content_height`.** `--snap` renders at the window's initial size, so `scripts/snap.nu` reads the panel's height from `--tree` first.
- **The headless clock outruns real providers.** Results come from Spotlight and the file system in real time, so headless renders pass `--realtime`, which `--help` lists but Rune's README doesn't.
