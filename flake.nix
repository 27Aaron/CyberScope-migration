{
  description = "Nix development environment for CyberScope";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
    ...
  }: let
    systems = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-linux"
    ];
    forAllSystems = function: nixpkgs.lib.genAttrs systems (system: function nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: {
      frontend = pkgs.stdenvNoCC.mkDerivation (finalAttrs: {
        pname = "cyberscope-console-frontend";
        version = "0.1.0";
        src = pkgs.lib.cleanSourceWith {
          src = ./frontend;
          filter = path: type: let
            name = pkgs.lib.baseNameOf (toString path);
            isSecretEnv = name == ".env" || (pkgs.lib.hasPrefix ".env." name && name != ".env.example");
          in
            !isSecretEnv && name != "dist" && name != "node_modules";
        };

        nativeBuildInputs = [
          pkgs.nodejs_22
          pkgs.pnpm_11
          pkgs.pnpmConfigHook
        ];

        pnpmDeps = pkgs.fetchPnpmDeps {
          inherit (finalAttrs) pname version src;
          pnpm = pkgs.pnpm_11;
          fetcherVersion = 4;
          hash = "sha256-+qZB14RdXCTQeR57LcCPM8td4N9s/uYgt6AkgezYv4k=";
        };

        buildPhase = ''
          runHook preBuild
          pnpm build
          runHook postBuild
        '';

        installPhase = ''
          mkdir -p $out
          cp -R dist $out/
        '';
      });

      default = pkgs.rustPlatform.buildRustPackage {
        pname = "cyberscope";
        version = "0.1.0";
        # Exclude runtime state, build output, and local secrets from the source.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type: let
            name = pkgs.lib.baseNameOf (toString path);
            isSecretEnv = name == ".env" || (pkgs.lib.hasPrefix ".env." name && name != ".env.example");
          in
            !isSecretEnv
            && name != "data"
            && name != "target"
            && name != "node_modules"
            && name != "dist"
            && name != ".pnpm-store";
        };
        cargoRoot = "backend";
        cargoLock.lockFile = ./backend/Cargo.lock;

        preBuild = ''
          mkdir -p frontend/dist
          cp -R ${self.packages.${pkgs.system}.frontend}/dist/. frontend/dist/
          cd backend
        '';

        meta = {
          description = "Multi-source internet asset intelligence console with an embedded web frontend";
          mainProgram = "cyberscope";
        };
      };
    });

    apps = forAllSystems (pkgs: {
      default = {
        type = "app";
        program = "${self.packages.${pkgs.system}.default}/bin/cyberscope";
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          cargo-nextest
          clippy
          nodejs_22
          pnpm_11
          pkg-config
          rust-analyzer
          rustc
          rustfmt
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });

    formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
  };
}
