{
  description = "rustcast — a Raycast-class GTK4 launcher for Linux with clipboard history, file search and tldr";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        rustcast = pkgs.rustPlatform.buildRustPackage {
          pname = "rustcast";
          version = "0.2.0";
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            # Wraps the GTK4 binary so icons, schemas and typelibs resolve at runtime.
            wrapGAppsHook4
          ];

          buildInputs = with pkgs; [
            glib
            gtk4
            gtk4-layer-shell
          ];

          # The suite includes a wall-clock timing assertion that can be flaky on a
          # loaded Nix builder; tests run in CI/dev instead.
          doCheck = false;

          meta = with pkgs.lib; {
            description = "A Raycast-class GTK4 launcher for Linux with clipboard history, file search and tldr";
            homepage = "https://github.com/zer0bav/rustcast";
            license = licenses.mit;
            mainProgram = "rustcast";
            platforms = platforms.linux;
          };
        };
      in
      {
        packages.default = rustcast;
        packages.rustcast = rustcast;

        # `nix run github:zer0bav/rustcast`
        apps.default = {
          type = "app";
          program = "${rustcast}/bin/rustcast";
        };

        # `nix develop` — a shell with the toolchain and GTK libs for hacking.
        devShells.default = pkgs.mkShell {
          inputsFrom = [ rustcast ];
          nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy pkg-config ];
        };
      });
}
