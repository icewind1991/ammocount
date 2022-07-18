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

      pkgs-cross-mingw = import nixpkgs {
        crossSystem = {
          config = "x86_64-w64-mingw32";
        };
        inherit system overlays;
      };
      mingw_w64_cc = pkgs-cross-mingw.stdenv.cc;
      mingw_w64 = pkgs-cross-mingw.windows.mingw_w64;
      windows = pkgs-cross-mingw.windows;
    in rec {
      devShells.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          (rust-bin.stable.latest.default.override {
            targets = [ "x86_64-pc-windows-gnu" ];
          })
          bacon
          mingw_w64_cc
        ];
        depsBuildBuild = [ pkgs.wine64 ];
        buildInputs = [ windows.pthreads ];

        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${mingw_w64_cc.targetPrefix}cc";
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER = "wine64";
      };
    });
}
