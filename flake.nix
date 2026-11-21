{
  description = "touchpad-snap: switch between windows of the same app with hotkeys and touchpad swipes";

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
          default = self.packages.${system}.touchpad-snap;
          touchpad-snap = pkgs.callPackage ./nix/package.nix { };
          gnome-shell-extension = pkgs.callPackage ./nix/extension.nix { };
        }
      );

      # Building the package runs the unit tests (cargo test). vm-touchpad
      # boots GNOME in a VM and swipes on a virtual touchpad (needs KVM).
      checks = forAllSystems (
        system: pkgs: {
          inherit (self.packages.${system}) touchpad-snap gnome-shell-extension;
          vm-touchpad = import ./nix/tests/touchpad.nix {
            inherit pkgs;
            touchpad-snap = self.packages.${system}.touchpad-snap;
            extension = self.packages.${system}.gnome-shell-extension;
          };
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.touchpad-snap ];
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
