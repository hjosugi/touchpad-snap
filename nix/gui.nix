{ lib, rustPlatform }:

let
  cargoToml = lib.importTOML ../crates/gui/Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "touchpad-snap-gui";
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../src
      ../schema
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The workspace has two packages; build/test only the settings app.
  cargoBuildFlags = [ "-p" "touchpad-snap-gui" ];
  cargoTestFlags = [ "-p" "touchpad-snap-gui" ];

  meta = {
    description = "Settings window for touchpad-snap";
    homepage = "https://github.com/hjosugi/touchpad-snap";
    license = lib.licenses.mit;
    mainProgram = "touchpad-snap-settings";
    platforms = lib.platforms.linux;
  };
}
