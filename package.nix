# TODO: move this to nixpkgs
# This file aims to be a replacement for the nixpkgs derivation.

{
  buildFeatures ? [ ],
  buildNoDefaultFeatures ? false,
  buildPackages,
  fetchFromGitHub,
  installManPages ? stdenv.buildPlatform.canExecute stdenv.hostPlatform,
  installShellCompletions ? stdenv.buildPlatform.canExecute stdenv.hostPlatform,
  installShellFiles,
  lib,
  pkg-config,
  rustPlatform,
  sqlite,
  stdenv,
}:

let
  # `vendored` builds SQLite from source, so a binary carrying it needs no
  # library on the machine it lands on; without it the store links the
  # system one through pkg-config.
  vendored = builtins.elem "vendored" buildFeatures;

in
rustPlatform.buildRustPackage (finalAttrs: {
  __structuredAttrs = true;

  inherit buildFeatures buildNoDefaultFeatures;

  pname = "pimdir";
  version = "0.4.1";
  cargoHash = "";

  src = fetchFromGitHub {
    owner = "pimalaya";
    repo = "io-pimdir";
    tag = "v${finalAttrs.version}";
    hash = "";
  };

  nativeBuildInputs = [
    pkg-config
    installShellFiles
  ];

  buildInputs = lib.optional (!vendored) sqlite;

  # rustc hands the linker sqlite's `-L` and nix's wrapper writes no rpath
  # for it, so a non-vendored binary loads nothing at run time. A vendored
  # one links its own copy and needs none of this.
  env = lib.optionalAttrs (!vendored) {
    RUSTFLAGS = "-C link-arg=-Wl,-rpath,${lib.getLib sqlite}/lib";
  };

  postInstall =
    let
      exe =
        if stdenv.buildPlatform.canExecute stdenv.hostPlatform then
          "$out/bin/${finalAttrs.pname}"
        else
          lib.getExe buildPackages.${finalAttrs.pname};
    in
    ''
      mkdir -p $out/share/{completions,man,schemas}
      ${exe} manual -d "$out"/share/man
      ${exe} completion -d "$out"/share/completions bash elvish fish powershell zsh
      ${exe} json-schema -d "$out"/share/schemas
    ''
    + lib.optionalString installManPages ''
      installManPage "$out"/share/man/*
    ''
    + lib.optionalString installShellCompletions ''
      installShellCompletion --cmd ${finalAttrs.pname} \
        --bash "$out"/share/completions/${finalAttrs.pname}.bash \
        --fish "$out"/share/completions/${finalAttrs.pname}.fish \
        --zsh "$out"/share/completions/_${finalAttrs.pname}
    '';

  # the spec suites need the pimdir checkout beside this one, which a nix
  # build has not got; tests.yml is where they are proven
  cargoTestFlags = [ "--lib" ];

  meta = {
    description = "Rust implementation of the Pimdir standard: the store and the sync engine";
    mainProgram = finalAttrs.pname;
    homepage = "https://github.com/pimalaya/io-pimdir";
    changelog = "https://github.com/pimalaya/io-pimdir/releases/${finalAttrs.src.tag}";
    license = with lib.licenses; [
      asl20
      mit
    ];
    maintainers = with lib.maintainers; [ soywod ];
  };
})
