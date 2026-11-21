# Evaluates the NixOS module and checks the generated config and user unit.
# It builds only the generated TOML file and this assertion derivation, not a
# NixOS system, and it needs no KVM. The VM check is separate (vm-touchpad).
{
  pkgs,
  nixpkgs,
  self,
  system,
}:

let
  evaluated = nixpkgs.lib.nixosSystem {
    modules = [
      self.nixosModules.default
      {
        nixpkgs.hostPlatform = system;
        system.stateVersion = "26.05";

        services.touchpad-snap = {
          enable = true;
          settings = {
            cycle = {
              order = "mru";
            };
            bindings = [
              {
                trigger = {
                  type = "hotkey";
                  keys = "Super+Alt+N";
                };
                actions = [
                  {
                    type = "window.next";
                  }
                ];
              }
            ];
          };
        };
      }
    ];
  };

  cfg = evaluated.config;

  configFile = cfg.environment.etc."touchpad-snap/config.json".source;
  execStart = cfg.systemd.user.services.touchpad-snap.serviceConfig.ExecStart;

  package = self.packages.${system}.touchpad-snap;
  extension = self.packages.${system}.gnome-shell-extension;

  dconfSettings = map (database: database.settings or { }) cfg.programs.dconf.profiles.user.databases;
  enabledExtensions = nixpkgs.lib.concatMap (
    settings: settings."org/gnome/shell".enabled-extensions or [ ]
  ) dconfSettings;
in
assert nixpkgs.lib.assertMsg (builtins.elem package cfg.environment.systemPackages)
  "services.touchpad-snap did not add the package to environment.systemPackages";
assert nixpkgs.lib.assertMsg (builtins.elem extension cfg.environment.systemPackages)
  "services.touchpad-snap did not add the extension to environment.systemPackages";
assert nixpkgs.lib.assertMsg (builtins.elem extension.extensionUuid enabledExtensions)
  "services.touchpad-snap did not add the extension to dconf enabled-extensions";
assert nixpkgs.lib.assertMsg (
  execStart == "${package}/bin/touchpad-snap daemon --config /etc/touchpad-snap/config.json"
) "unexpected ExecStart: ${execStart}";
pkgs.runCommand "touchpad-snap-module-eval" { nativeBuildInputs = [ pkgs.jq ]; } ''
  jq -e '.cycle.order == "mru"' ${configFile}
  jq -e '.bindings | length == 1' ${configFile}
  jq -e '.bindings[0].trigger.type == "hotkey"' ${configFile}
  jq -e '.bindings[0].trigger.keys == "Super+Alt+N"' ${configFile}
  jq -e '.bindings[0].actions[0].type == "window.next"' ${configFile}
  touch $out
''
