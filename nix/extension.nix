{ lib, stdenvNoCC }:

let
  uuid = "window-jump@hjosugi.github.io";
  cargoToml = lib.importTOML ../Cargo.toml;
in
stdenvNoCC.mkDerivation {
  pname = "gnome-shell-extension-window-jump";
  inherit (cargoToml.package) version;

  # The directory name carries an "@", which a store path cannot, so name it.
  src = builtins.path {
    path = ../gnome-extension + "/${uuid}";
    name = "window-jump-gnome-extension-src";
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
    description = "GNOME Shell bridge for window-jump: window list, global hotkeys and touchpad swipes on the session bus";
    homepage = "https://github.com/hjosugi/window-jump";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
