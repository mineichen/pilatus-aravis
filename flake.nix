{
  description = "Deterministic Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    devtools.url = "github:mineichen/nix-devtools";
  };

  outputs = inputs @ { flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];

      perSystem = { system, ... }: let
        pkgs = import inputs.nixpkgs {
          inherit system;
          config.allowUnfreePredicate = pkg: builtins.elem (inputs.nixpkgs.lib.getName pkg) [ "claude-code" ];
        };

        # Stable toolchain for regular tests
        stableToolchain = with inputs.fenix.packages.${system}; combine [
          stable.toolchain
        ];

        # Nightly toolchain for Miri
        nightlyToolchain = inputs.fenix.packages.${system}.latest.withComponents [
          "cargo"
          "rustc"
          "rustfmt"
          "miri"
        ];

        # Common build inputs
        commonBuildInputs = [
          pkgs.pkg-config
          pkgs.mold
          pkgs.bashInteractive
          pkgs.glib
          pkgs.aravis
          pkgs.openssl
          pkgs.gcc
          pkgs.binutils
        ];

        # Path lists are separated by ':' (not ';')
        defaultLdLibraryPath = "${pkgs.openssl.out}/lib:${pkgs.aravis.lib}/lib:${pkgs.glib.out}/lib";
        defaultPkgConfigPath = "${pkgs.openssl.dev}/lib/pkgconfig:${pkgs.aravis.dev}/lib/pkgconfig:${pkgs.glib.dev}/lib/pkgconfig";

        # `nix run .#isolated -- ..` starts a sandbox, see isolated.nix of nix-devtools.
        # `..` makes the parent the workspace, so the path-dependencies are available.
        isolated = inputs.devtools.lib.isolated {
          inherit pkgs inputs;
          name = "pilatus-aravis-isolated-dev";
          extraMounts = [ "../pilatus" "../imbuf" ];
          packages = commonBuildInputs ++ [
            stableToolchain
            pkgs.busybox
            pkgs.git
            pkgs.ripgrep
            pkgs.curl
            pkgs.opencode
            pkgs.mdbook
          ];
          claude.pluginDirs = [
            (inputs.devtools.lib.claudeRustAnalyzer {
              inherit pkgs;
              rust-analyzer = stableToolchain;
            })
          ];
          env = {
            LD_LIBRARY_PATH = defaultLdLibraryPath;
            PKG_CONFIG_PATH = defaultPkgConfigPath;
          };
        };

        # Helper function to create shell scripts with error handling
        # Sets up common build environment (gcc, binutils, mold, pkg-config, openssl)
        # Note: PKG_CONFIG_PATH should be sufficient for openssl-sys to find OpenSSL
        writeShellScriptWithError = name: script:
          toString (pkgs.writeShellScript name ''
            set -e
            export PATH="${pkgs.gcc}/bin:${pkgs.binutils}/bin:${pkgs.mold}/bin:${pkgs.pkg-config}/bin"
            export LD_LIBRARY_PATH="${defaultLdLibraryPath}"
            export PKG_CONFIG_PATH="${defaultPkgConfigPath}"
            ${script}
          '');

        # Pure package for miri-test (built in sandbox)
        miriTestPure = pkgs.writeShellApplication {
          name = "miri-test-pure";
          runtimeInputs = [
            nightlyToolchain
          ] ++ commonBuildInputs;

          text = ''
            set -e
            # Additional environment setup (runtimeInputs handles PATH for binaries)
            export LD_LIBRARY_PATH="${defaultLdLibraryPath}"
            export PKG_CONFIG_PATH="${defaultPkgConfigPath}"
            # MIRI_SYSROOT needs to be writable - use a cache directory
            # cargo miri setup will create the sysroot here
            export MIRI_SYSROOT="''${MIRI_SYSROOT:-$HOME/.cache/miri}"
            mkdir -p "$MIRI_SYSROOT"
            cargo miri setup
            cargo miri test miri
          '';
        };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            stableToolchain
          ] ++ commonBuildInputs ++ [
            pkgs.mdbook
          ];

          LD_LIBRARY_PATH = defaultLdLibraryPath;
          PKG_CONFIG_PATH = defaultPkgConfigPath;

	  shellHook = ''
            echo "===================================="
            echo " Welcome to the deterministic dev shell! "
            echo "===================================="
            echo "Rust toolchain:"
            rustc --version
            echo "Cargo version:"
            cargo --version
            echo "LD_LIBRARY_PATH: $LD_LIBRARY_PATH"
            echo "PKG_CONFIG_PATH: $PKG_CONFIG_PATH"
            echo "===================================="
            echo "Ready to develop! 🦀"
          '';
        };

        # Miri devShell with nightly
        devShells.miri = pkgs.mkShell {
          buildInputs = [
            nightlyToolchain
            pkgs.cargo-udeps
          ] ++ commonBuildInputs;

          LD_LIBRARY_PATH = defaultLdLibraryPath;
          PKG_CONFIG_PATH = defaultPkgConfigPath;

          shellHook = ''
            echo "Miri dev shell (nightly)"
            rustc --version
          '';
        };

        # Apps for CI/local execution
        apps.test = {
          type = "app";
          program = writeShellScriptWithError "test" ''
            export PATH="${stableToolchain}/bin:$PATH"
            cargo test
          '';
        };

        apps.test-all-features = {
          type = "app";
          program = writeShellScriptWithError "test-all-features" ''
            export PATH="${stableToolchain}/bin:$PATH"
            cargo test --all-features
          '';
        };

        # Pure package for miri-test (built in sandbox)
        packages.miri-test-pure = miriTestPure;

        # App that uses the pure package (built in sandbox when run)
        # When you run `nix run .#miri-test`, it will build the pure package first
        apps.miri-test = {
          type = "app";
          program = "${miriTestPure}/bin/miri-test-pure";
        };

        apps.build-book = {
          type = "app";
          program = writeShellScriptWithError "build-book" ''
            export PATH="${pkgs.mdbook}/bin"
            cd book
            mdbook build
          '';
        };

        apps.isolated = {
          type = "app";
          program = pkgs.lib.getExe isolated;
        };
      };
    };
}
