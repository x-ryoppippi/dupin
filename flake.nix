{
  description = "Dupin: a lightweight launcher for Apple silicon Macs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # agent-desktop for the E2E job; its own nixpkgs keeps cache.numtide.com hits.
    llm-agents.url = "github:numtide/llm-agents.nix";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      # Apple silicon macOS, and x86_64 Linux on X11.
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      imports = [
        inputs.treefmt-nix.flakeModule
        inputs.git-hooks.flakeModule
        ./nix/git-hooks.nix
        ./nix/packages.nix
        ./nix/pnpm.nix
        ./nix/home-manager.nix
        ./nix/treefmt.nix
        ./nix/checks.nix
        ./nix/dev-shell.nix
        ./nix/hawk.nix
      ];
    };
}
