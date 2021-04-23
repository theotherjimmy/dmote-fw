let
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz);
  pkgs = import <nixos> { overlays = [moz_overlay]; };
  my-rustc = (pkgs.rustChannelOf {
    channel = "nightly";
    date = "2021-04-19";
  }).rust.override {
    targets = ["thumbv7m-none-eabi"];
    extensions = ["rust-src"];
  };
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    my-rustc
    rustfmt
    cargo
    cargo-crev
    cargo-watch
    rls
    rust-analyzer
    gcc-arm-embedded
    # debug tools
    openocd
    gdb-multitarget
    cargo-binutils
    cargo-bloat
  ];
}
