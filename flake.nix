{
  description = "A very basic flake";
  inputs = {
      nixpkgs.url = "github:nixos/nixpkgs/master";
      flake-utils.url = "github:numtide/flake-utils/master";
      rust-overlay.url = "github:oxalica/rust-overlay/master";
      rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
      rust-overlay.inputs.flake-utils.follows = "flake-utils";
  };
  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ rust-overlay.overlay ];
        pkgs = import nixpkgs { inherit system overlays; };
        my-rust = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" "llvm-tools-preview" ];
          targets = [ "thumbv7m-none-eabi" "x86_64-unknown-linux-gnu" ];
        };
      in {
        devShell = pkgs.mkShell {
          buildInputs = with pkgs; [
            my-rust
            cargo-watch
            cargo-bloat
            cargo-binutils
            gdb-multitarget
            openocd
            libusb1
            pkg-config
          ];
        };
      });
}
