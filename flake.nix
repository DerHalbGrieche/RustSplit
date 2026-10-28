{
  description = "RustSplit - a native Rust rewrite of LiveSplit";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      version = cargoToml.workspace.package.version;

      runtimeDeps =
        pkgs: with pkgs; [
          libGL
          libx11
          libxcb
          libxcursor
          libxi
          libxinerama
          libxkbcommon
          libxrandr
          wayland
        ];
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          rustsplit = pkgs.rustPlatform.buildRustPackage {
            pname = "rustsplit";
            inherit version;

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              addDriverRunpath
              makeWrapper
              pkg-config
            ];
            buildInputs = runtimeDeps pkgs;

            postFixup = ''
              wrapProgram $out/bin/rustsplit \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeDeps pkgs)}
            '';

            meta = with pkgs.lib; {
              description = "A native Rust rewrite of LiveSplit";
              license = licenses.mit;
              mainProgram = "rustsplit";
              platforms = platforms.linux;
            };
          };
        in
        {
          inherit rustsplit;
          default = rustsplit;
        }
      );

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/rustsplit";
          meta.description = "Run RustSplit";
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [
            cargo
            clippy
            nixfmt
            pkg-config
            rustc
            rustfmt
          ];
          shellHook = ''
            export RUST_LOG=info
          '';
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
