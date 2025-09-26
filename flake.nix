{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    foundry.url = "github:shazow/foundry.nix/stable";
    dcap-nixpkgs.url = "github:ozwaldorf/nixpkgs/sgx-dcap-default-qpl-1.21";
  };

  outputs =
    inputs:
    let
      system = "x86_64-linux";
      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [ inputs.foundry.overlay ];
      };
      dcapPkgs = import inputs.dcap-nixpkgs { inherit system; };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          # Build
          rustup
          openssl
          pkgconf
          protobuf

          # Development
          foundry
          jq
          bc
          dcapPkgs.sgx-dcap-default-qpl
        ];
      };
    };
}
