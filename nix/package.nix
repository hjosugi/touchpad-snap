{ lib, rustPlatform }:

let
  cargoToml = lib.importTOML ../Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "touchpad-snap";
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../src
      ../examples
      ../contrib
      ../schema
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The workspace member (core) holds most tests; the daemon alone is not enough.
  cargoTestFlags = [ "--workspace" "--exclude" "touchpad-snap-gui" ];

  # touchpad-snapw is the console-less daemon for Windows; on Linux it is the
  # same as `touchpad-snap daemon`.
  postInstall = ''
    rm $out/bin/touchpad-snapw
  '';

  meta = {
    description = "Switch between windows of the same app with hotkeys and touchpad swipes";
    homepage = "https://github.com/hjosugi/touchpad-snap";
    license = lib.licenses.mit;
    mainProgram = "touchpad-snap";
    platforms = lib.platforms.linux;
  };
}
