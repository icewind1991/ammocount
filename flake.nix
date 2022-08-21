{
  inputs = {
    nixpkgs.url = "nixpkgs/release-22.05";
    nixpkgs-unstable.url = "nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    rust-overlay,
    nixpkgs-unstable,
  }:
    utils.lib.eachDefaultSystem (system: let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      pkgs-unstable = import nixpkgs-unstable {
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
          cargo-edit
          cargo-outdated
          pkgs-unstable.wine64Packages.staging
          pkg-config
        ];

        buildInputs = with pkgs; [openssl];
        OPENSSL_NO_VENDOR = 1;
      };

      devShells.windows = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          (rust-bin.stable.latest.default.override {
            targets = [ "x86_64-pc-windows-gnu" ];
          })
          bacon
          mingw_w64_cc
          windows.pthreads
          pkg-config
        ];
        depsBuildBuild = with pkgs; [ pkgs-unstable.wine64Packages.staging ];
        buildInputs = with pkgs; [windows.pthreads openssl];

        OPENSSL_NO_VENDOR = 1;
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "${mingw_w64_cc.targetPrefix}cc";
        CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER = "wine64";
      };
    });
}
