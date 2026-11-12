{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scwoll;
in
{
  options.programs.scwoll = {
    enable = lib.mkEnableOption "Scwoll, a native YouTube-style media player";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = lib.literalMD ''
        `scwoll` built from the flake source with your own `pkgs`
      '';
      description = ''
        The Scwoll package to install. Override this to use the flake's
        pinned build instead of rebuilding with your nixpkgs, e.g.
        `inputs.scwoll.packages.''${pkgs.stdenv.hostPlatform.system}.default`.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];
  };
}
