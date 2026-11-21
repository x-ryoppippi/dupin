# Releasing

[tagpr](https://github.com/Songmu/tagpr) keeps a release pull request open against `main`. Each push to `main` updates it with the next version in `VERSION`; merging it tags the merge commit and creates the GitHub release from GitHub's generated notes. There is no changelog file.

Add a `tagpr:minor` or `tagpr:major` label to the release pull request for a bigger bump than a patch, or edit `VERSION` on its branch.

The crates stay at version 0.0.0 and the Nix package reads `VERSION`, so a release changes neither `Cargo.lock` nor the vendored-dependency hash in `package.nix`.

A pull request that tagpr pushes with `GITHUB_TOKEN` starts no workflows, so the tagpr workflow dispatches CI on the release branch itself.

## CI

CI runs `nix flake check` on Blacksmith's Apple silicon macOS runners. `.github/actions/setup-nix-cache` installs Nix and its store cache through [`x-ryoppippi/setup-nix`](https://github.com/x-ryoppippi/setup-nix), and a daily workflow garbage-collects the cache.

## Signed builds

The Nix package is unsigned and never updates itself. The release builds come from `scripts/package-app.nu`, which copies the Nix-built `Dupin.app` out of the store, points its libiconv at the one macOS ships, writes the channel's version, build number (the commit count) and update feed into `Info.plist`, signs it with the Developer ID under the hardened runtime, notarizes and staples it, and writes `Dupin.zip` and `update.json`.

`.github/workflows/package.yml` runs it for both channels:

- **stable**: merging the tagpr pull request packages the tag and uploads both files to its release. `releases/latest/download/update.json` is the stable feed.
- **nightly**: `.github/workflows/nightly.yml` runs daily (and on demand) when `main` has moved since the last nightly, moves the `nightly` tag to it and replaces the files on the `nightly` prerelease.

Try a signed bundle locally with `nu scripts/package-app.nu --channel nightly --identity "Developer ID Application: …"` (add `--notarize` with the notary variables set), or `--identity -` for an ad-hoc signature, which runs but never updates.

### Secrets

| Secret                      | Value                                                                                                            |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `DEVELOPER_ID_P12`          | The Developer ID Application certificate and private key exported from Keychain Access as `.p12`, base64-encoded |
| `DEVELOPER_ID_P12_PASSWORD` | The `.p12` export password                                                                                       |
| `NOTARY_KEY`                | The contents of an App Store Connect API key (`.p8`, Developer role)                                             |
| `NOTARY_KEY_ID`             | That key's ID                                                                                                    |
| `NOTARY_ISSUER`             | The App Store Connect issuer ID                                                                                  |
