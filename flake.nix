{
  inputs = {
    nixpkgs.url = "nixpkgs/release-22.05";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    rust-overlay,
  }:
    utils.lib.eachDefaultSystem (system: let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
    in rec {
      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [rust-bin.beta.latest.default bacon cargo-cross];
      };
    });
}
