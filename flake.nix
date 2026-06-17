{
  description = "armázem — the any-storage→S3 gateway (M0: filesystem backend)";

  inputs = {
    nixpkgs.follows = "substrate/nixpkgs";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.fenix.follows = "fenix";
    };
    forge = {
      url = "github:pleme-io/forge";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.fenix.follows = "fenix";
      inputs.substrate.follows = "substrate";
      inputs.crate2nix.follows = "crate2nix";
    };
    crate2nix = {
      url = "github:nix-community/crate2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    devenv = {
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, substrate, forge, crate2nix, devenv, ... }:
    (import "${substrate}/lib/rust-service-flake.nix" {
      inherit nixpkgs substrate forge crate2nix devenv;
    }) {
      inherit self;
      serviceName = "armazem";
      registry = "ghcr.io/pleme-io/armazem";
      packageName = "armazem";
      namespace = "armazem-system";
      architectures = ["amd64" "arm64"];
      # S3 data plane (9000), health + metrics (9001). The flake-arg keys are
      # generic; armazem maps `graphql` → the primary S3 port.
      ports = { graphql = 9000; health = 9001; metrics = 9001; };
      # M0 ships no HM/NixOS module trio — the chart is the deploy surface.
      moduleDir = null;
      nixosModuleFile = null;
      # Build the committed crate2nix Cargo.nix (cargo-metadata feature
      # resolution) rather than the gen build-spec forward path; M0's small
      # dep set resolves cleanly under crate2nix and needs no gen tooling.
      useLockfileBuilder = false;
    };
}
