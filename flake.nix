{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    foundry.url = "github:shazow/foundry.nix/stable";
  };

  outputs =
    inputs:
    let
      system = "x86_64-linux";
      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [ inputs.foundry.overlay ];
      };
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
        ];
      };
    };
}
