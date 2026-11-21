{ lib, stdenvNoCC }:

let
  uuid = "touchpad-snap@hjosugi.github.io";
  cargoToml = lib.importTOML ../Cargo.toml;
in
stdenvNoCC.mkDerivation {
  pname = "gnome-shell-extension-touchpad-snap";
  inherit (cargoToml.package) version;

  # The directory name carries an "@", which a store path cannot, so name it.
  src = builtins.path {
    path = ../gnome-extension + "/${uuid}";
    name = "touchpad-snap-gnome-extension-src";
  };

  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/gnome-shell/extensions/${uuid}
    cp -r . $out/share/gnome-shell/extensions/${uuid}/
    runHook postInstall
  '';

  # Read by NixOS / Home Manager to enable the extension.
  passthru.extensionUuid = uuid;

  meta = {
    description = "GNOME Shell bridge for touchpad-snap: window list, global hotkeys and touchpad swipes on the session bus";
    homepage = "https://github.com/hjosugi/touchpad-snap";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
