{ pkgs ? import <nixpkgs> {} }:

# Development environment for the Prime Agent Rust workspace.
pkgs.mkShell {
  packages = with pkgs; [
    # Rust toolchain and workspace quality gates.
    rustc
    cargo
    rustfmt
    clippy
    cargo-deny

    # Build and release scripts.
    gnumake
    python3
    git
    binutils

    # Native build-script support and TLS/crypto headers used by dependencies.
    pkg-config
    openssl

    # Keep this shell interactive when entered from bash.
    bashInteractive
  ];

  # Let pkg-config find OpenSSL without requiring a global environment setup.
  PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
}
