{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    foundry.url = "github:shazow/foundry.nix/stable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs:
    let
      system = "x86_64-linux";
      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [
          inputs.foundry.overlay
          (import inputs.rust-overlay)
        ];
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          # Build
          (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
          openssl
          pkgconf
          protobuf
          sgxs-tools
          sgx-azure-dcap-client

          # Development
          foundry
          jq
          bc
        ];
      };
    };
}
