#!/usr/bin/env nu

# Refresh the vendored-dependency hash in package.nix after Cargo.lock changes.
#
# nix-update evaluates the flake from its store path, which Nix's lazy trees don't create
# until something asks for it, so archive the flake first.
def main []: nothing -> nothing {
    cd ($env.FILE_PWD | path dirname)
    nix flake archive --json | from json | ignore
    nix-update --flake --version=skip default
}
