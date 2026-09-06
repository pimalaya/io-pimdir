{
  pimalaya ? import (fetchTarball "https://github.com/pimalaya/nix/archive/master.tar.gz"),
  ...
}@args:

let
  pimdir = import ./default.nix (
    removeAttrs args [
      "crossPkgs"
      "isStatic"
      "target"
    ]
  );

in
pimalaya.mkDefault (
  {
    src = ./.;
    version = "0.4.1";
    mkPackage = (
      {
        lib,
        pkgs,
        rustPlatform,
        defaultFeatures,
        features,
        buildPackages,
      }:

      pkgs.callPackage ./package.nix {
        inherit lib rustPlatform;
        buildPackages = buildPackages // {
          inherit pimdir;
        };
        installShellCompletions = false;
        installManPages = false;
        buildNoDefaultFeatures = !defaultFeatures;
        # The binary sits behind `cli`, which is deliberately not a default
        # so a library consumer compiles no terminal dependency, and behind
        # `vendored`, so a released binary carries its own SQLite instead of
        # needing one on the machine it lands on.
        buildFeatures =
          if features == "" then
            [
              "cli"
              "vendored"
            ]
          else
            lib.splitString "," features;
      }
    );
  }
  // removeAttrs args [ "pimalaya" ]
)
