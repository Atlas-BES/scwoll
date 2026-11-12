{
  description = "Scwoll — native YouTube-style media player in Rust (egui + mpv)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems =
        f: nixpkgs.lib.genAttrs systems (system: f system (import nixpkgs { inherit system; }));
    in
    {
      packages = forAllSystems (
        system: pkgs: {
          default = self.packages.${system}.scwoll;
          scwoll = pkgs.callPackage ./nix/package.nix { };
        }
      );

      apps = forAllSystems (
        system: pkgs: {
          default = {
            type = "app";
            program = "${self.packages.${system}.scwoll}/bin/scwoll";
            meta.description = "Run Scwoll";
          };
          scwoll = self.apps.${system}.default;
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.callPackage ./nix/devshell.nix { };
        }
      );

      overlays.default = final: prev: {
        scwoll = final.callPackage ./nix/package.nix { };
      };

      homeManagerModules = {
        default = ./nix/hm-module.nix;
        scwoll = ./nix/hm-module.nix;
      };

      formatter = forAllSystems (system: pkgs: pkgs.nixfmt);
    };
}
