{
  description = "window-jump: switch between windows of the same app with hotkeys and touchpad swipes";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (
        system: pkgs: {
          default = self.packages.${system}.window-jump;
          window-jump = pkgs.callPackage ./nix/package.nix { };
          gnome-shell-extension = pkgs.callPackage ./nix/extension.nix { };
        }
      );

      # Building the package runs the unit tests (cargo test).
      checks = forAllSystems (
        system: _: {
          inherit (self.packages.${system}) window-jump gnome-shell-extension;
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.window-jump ];
            packages = with pkgs; [
              clippy
              rustfmt
              rust-analyzer
              nodejs # extension unit tests
              jq
              glib # gdbus
              dbus # dbus-run-session
              go-task
            ];
          };
        }
      );
    };
}
