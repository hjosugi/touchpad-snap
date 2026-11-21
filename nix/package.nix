{ lib, rustPlatform }:

let
  cargoToml = lib.importTOML ../Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "window-jump";
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../examples
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # window-jumpw is the console-less daemon for Windows; on Linux it is the
  # same as `window-jump daemon`.
  postInstall = ''
    rm $out/bin/window-jumpw
  '';

  meta = {
    description = "Switch between windows of the same app with hotkeys and touchpad swipes";
    homepage = "https://github.com/hjosugi/window-jump";
    license = lib.licenses.mit;
    mainProgram = "window-jump";
    platforms = lib.platforms.linux;
  };
}
