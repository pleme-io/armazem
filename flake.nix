{
  description = "armázem — the any-storage→S3 gateway (M0: filesystem backend)";

  # substrate.rust.service dispatches over Cargo.gen.lock (the slim gen delta,
  # reconstructed to the full BuildSpec in pure Nix) — no crate2nix, no Cargo.nix.
  inputs.substrate.url = "github:pleme-io/substrate";

  outputs = { substrate, ... }: substrate.rust.service {
    src = ./.;
  };
}
