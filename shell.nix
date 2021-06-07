let
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz);
  pkgs = import <nixos> { overlays = [moz_overlay]; };
  my-rustc = (pkgs.rustChannelOf {
    channel = "nightly";
    date = "2021-04-19";
  }).rust.override {
    targets = [ "thumbv7m-none-eabi" "x86_64-unknown-linux-gnu" ];
    extensions = ["rust-src"];
  };
  formpkg = { stdenv, fetchFromGitHub, rustPlatform }:
    with rustPlatform; buildRustPackage rec {
      pname = "form";
      version = "0.8.0";

      src = fetchFromGitHub {
        owner = "djmcgill";
        repo = pname;
        rev = "fcb397a39d633ba7fbda057483e0587cef05f25d";
        hash = "sha256-P4RYIeruUvv4dVEBNIWkReUlj+qd4L+aPMcKejkGUMs=";
      };
      cargoSha256 = "sha256-NCgPXWPH9bhkrTNJK7G7rQewH2/5IKaWrnva+8bJJYo=";

      doCheck = false;

      meta = with stdenv.lib; {
        description = "Split apart a large file with multiple modules into the idiomatic rust directory structure.";
        homepage = "https://github.com/djmcgill/${pname}";
        license = with licenses; [ mit asl20 ];
      };
    };
  svdtoolspkg = { lib, python3Packages}:
    with python3Packages; buildPythonApplication rec {
      pname = "svdtools";
      version = "0.1.13";

      src = python3Packages.fetchPypi {
        inherit pname version;
        sha256 = "sha256-SytDvaKMhaaCrgioOr+gkVzVQW8RCU4t7P/EPYiIUAM=";
      };

      doCheck = false;
      propagatedBuildInputs = [ click pyyaml lxml ];

      meta = with lib; {
        homepage = "http://github.com/stm32-rs/${pname}";
        description = "Modify vendor-supplied, often buggy SVD files.";
      };
    };
  svd2rustpkg = { stdenv, lib, fetchFromGitHub, rustPlatform }:
    with rustPlatform; buildRustPackage rec {
      pname = "svd2rust";
      version = "0.18.0";

      src = fetchFromGitHub {
        owner = "rust-embedded";
        repo = "svd2rust";
        rev = "v${version}";
        hash = "sha256-NhkXVL9j6rA0FDHUKemkKWCseQv73K3gA5mmR/DAH9w=";
      };
      cargoPatches = [ ./0001-Add-lock-file.patch ];

      cargoSha256 = "sha256-hNOTF/aPokUGuKqT9U4VZ19uy629l61Gi/LMGBUZSAg=";

      # doc tests fail due to missing dependency
      doCheck = false;

      meta = with lib; {
        description = "Generate Rust register maps (`struct`s) from SVD files";
        homepage = "https://github.com/rust-embedded/svd2rust";
        license = with licenses; [ mit asl20 ];
      };
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
    # svd2rust tools
    (pkgs.callPackage svd2rustpkg {})
    gnumake
    (pkgs.callPackage formpkg {})
    (pkgs.callPackage svdtoolspkg {})
    unzip
    # debug tools
    openocd
    gdb-multitarget
    cargo-binutils
    cargo-bloat
  ];
}
