{ lib, rustPlatform, makeWrapper, wayland, libxkbcommon, libglvnd, mesa, xorg }:

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

  nativeBuildInputs = [ makeWrapper ];
  buildInputs = [ wayland libxkbcommon libglvnd mesa xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi ];

  # winit/glutin load Wayland, X11 and EGL with dlopen, which does not use the
  # binary's linked libraries. Put them on the wrapper's LD_LIBRARY_PATH so the
  # settings window starts from the desktop entry and from the tray.
  postInstall = ''
    wrapProgram $out/bin/touchpad-snap-settings \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [
        wayland libxkbcommon libglvnd mesa
        xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi
      ]}
  '';

  meta = {
    description = "Settings window for touchpad-snap";
    homepage = "https://github.com/hjosugi/touchpad-snap";
    license = lib.licenses.mit;
    mainProgram = "touchpad-snap-settings";
    platforms = lib.platforms.linux;
  };
}
