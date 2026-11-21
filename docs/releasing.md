# Releasing

[tagpr](https://github.com/Songmu/tagpr) keeps a release pull request open against `main`. Each push to `main` updates it with the next version in `VERSION`; merging it tags the merge commit and creates the GitHub release from GitHub's generated notes. There is no changelog file.

Add a `tagpr:minor` or `tagpr:major` label to the release pull request for a bigger bump than a patch, or edit `VERSION` on its branch.

The crates stay at version 0.0.0 and the Nix package reads `VERSION`, so a release changes neither `Cargo.lock` nor the vendored-dependency hash in `package.nix`.

A pull request that tagpr pushes with `GITHUB_TOKEN` starts no workflows, so the tagpr workflow dispatches CI on the release branch itself.

## CI

CI runs `nix flake check` on Blacksmith's Apple silicon macOS runners. `.github/actions/setup-nix-cache` installs Nix and its store cache through [`x-ryoppippi/setup-nix`](https://github.com/x-ryoppippi/setup-nix), and a daily workflow garbage-collects the cache.
