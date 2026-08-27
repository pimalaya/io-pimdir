{
  description = "Rust implementation of Pimdir standard";

  inputs = {
    nixpkgs = {
      url = "github:nixos/nixpkgs/nixos-25.11";
    };
    fenix = {
      url = "github:nix-community/fenix/monthly";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pimalaya = {
      url = "github:pimalaya/nix";
      flake = false;
    };
  };

  outputs =
    inputs:
    (import inputs.pimalaya).mkFlakeOutputs inputs {
      shell = ./shell.nix;
    };
}
