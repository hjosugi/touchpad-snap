# NixOS module: services.touchpad-snap
#
#   services.touchpad-snap = {
#     enable = true;
#     settings = {
#       cycle = { order = "mru"; };
#       bindings = [
#         {
#           trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
#           actions = [ { type = "window.next"; } ];
#         }
#       ];
#     };
#   };
#
# settings is rendered to /etc/touchpad-snap/config.json. The user service
# reads that file explicitly, so the module never touches
# ~/.config/touchpad-snap/config.json.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.touchpad-snap;

  system = pkgs.stdenv.hostPlatform.system;

  json = pkgs.formats.json { };
  configFile = json.generate "touchpad-snap-config.json" cfg.settings;

  extensionUuid =
    cfg.gnome.extensionPackage.extensionUuid or "touchpad-snap@hjosugi.github.io";
in
{
  options.services.touchpad-snap = {
    enable = lib.mkEnableOption "touchpad-snap, switching between windows of the same app with hotkeys and touchpad swipes";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.touchpad-snap;
      defaultText = lib.literalExpression "touchpad-snap.packages.\${pkgs.stdenv.hostPlatform.system}.touchpad-snap";
      description = "The touchpad-snap package to use.";
    };

    settings = lib.mkOption {
      type = json.type;
      default = { };
      example = lib.literalExpression ''
        {
          cycle = { order = "mru"; workspace = "all"; };
          bindings = [
            {
              trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
              actions = [ { type = "window.next"; } ];
            }
            {
              trigger = { type = "hotkey"; keys = "Super+Alt+P"; };
              actions = [ { type = "window.prev"; } ];
            }
          ];
        }
      '';
      description = ''
        The contents of {file}`/etc/touchpad-snap/config.json`, in the same
        JSON schema as a hand-written config file (see
        `schema/config.schema.json`). An empty attribute set writes an empty
        file, which makes touchpad-snap use its built-in defaults.
      '';
    };

    systemd.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Run `touchpad-snap daemon` as a systemd user service for the graphical session.";
    };

    gnome = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Install the GNOME Shell extension and make it part of the system
          default for `org.gnome.shell.enabled-extensions`.
        '';
      };

      extensionPackage = lib.mkOption {
        type = lib.types.package;
        default = self.packages.${system}.gnome-shell-extension;
        defaultText = lib.literalExpression "touchpad-snap.packages.\${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension";
        description = ''
          The GNOME Shell extension package. Its `extensionUuid` passthru
          attribute, when present, is used as the extension id.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      environment.systemPackages = [ cfg.package ]
        ++ lib.optional cfg.gnome.enable cfg.gnome.extensionPackage;

      environment.etc."touchpad-snap/config.json".source = configFile;
    }

    (lib.mkIf cfg.systemd.enable {
      systemd.user.services.touchpad-snap = {
        description = "touchpad-snap daemon";
        wantedBy = [ "graphical-session.target" ];
        partOf = [ "graphical-session.target" ];
        after = [ "graphical-session.target" ];
        serviceConfig = {
          ExecStart = "${cfg.package}/bin/touchpad-snap daemon --config /etc/touchpad-snap/config.json";
          Restart = "on-failure";
          RestartSec = 2;
        };
      };
    })

    # The system dconf database supplies defaults for users who have not
    # written org.gnome.shell.enabled-extensions themselves. It is a file-db
    # behind user-db:user, so a user who toggles extensions in GNOME keeps
    # their own list (and can lose ours that way).
    (lib.mkIf cfg.gnome.enable {
      programs.dconf.profiles.user.databases = [
        {
          settings."org/gnome/shell".enabled-extensions = [ extensionUuid ];
        }
      ];
    })
  ]);
}
