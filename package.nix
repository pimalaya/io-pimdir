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
  windows,
}:

let
  vendored = builtins.elem "vendored" buildFeatures;

  sqlite' =
    if stdenv.hostPlatform.isWindows then
      sqlite.overrideAttrs (old: {
        buildInputs = (old.buildInputs or [ ]) ++ [ windows.pthreads ];
      })
    else
      sqlite;

in
rustPlatform.buildRustPackage (finalAttrs: {
  __structuredAttrs = true;

  inherit buildNoDefaultFeatures;

  pname = "pimdir";
  version = "0.5.0";
  cargoHash = "";

  src = fetchFromGitHub {
    owner = "pimalaya";
    repo = "io-pimdir";
    tag = "v${finalAttrs.version}";
    hash = "";
  };

  # pkg-config hands the linker libsqlite3 but no rpath, leaving a binary that
  # cannot find it: not in postInstall, which runs it, nor once installed.
  env.NIX_LDFLAGS = lib.optionalString (!vendored) ("-rpath " + lib.getLib sqlite' + "/lib");

  nativeBuildInputs = [
    pkg-config
    installShellFiles
  ];

  buildInputs = lib.optional (!vendored) sqlite';

  buildFeatures = [ "cli" ] ++ buildFeatures;

  postInstall =
    let
      exe =
        if stdenv.buildPlatform.canExecute stdenv.hostPlatform then
          "$out/bin/${finalAttrs.meta.mainProgram}"
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
      installShellCompletion --cmd ${finalAttrs.meta.mainProgram} \
        --bash "$out"/share/completions/${finalAttrs.meta.mainProgram}.bash \
        --fish "$out"/share/completions/${finalAttrs.meta.mainProgram}.fish \
        --zsh "$out"/share/completions/_${finalAttrs.meta.mainProgram}
    '';

  cargoTestFlags = [ "--bins" ];

  meta = {
    description = "Rust implementation of the Pimdir standard: the store and the sync engine";
    mainProgram = "pimdir";
    homepage = "https://github.com/pimalaya/io-pimdir";
    changelog = "${finalAttrs.meta.homepage}/releases/tag/${finalAttrs.src.tag}";
    license = with lib.licenses; [
      asl20
      mit
    ];
    maintainers = with lib.maintainers; [ soywod ];
  };
})
