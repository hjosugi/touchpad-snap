# Home Manager module: services.touchpad-snap
#
#   services.touchpad-snap = {
#     enable = true;
#     settings = {
#       order = "mru";
#       hotkey = [ { keys = "Super+Alt+N"; action = "next"; } ];
#     };
#   };
#
# settings is rendered to $XDG_CONFIG_HOME/touchpad-snap/config.toml, where
# touchpad-snap looks by default. Home Manager's programs.gnome-shell module
# enables the extension through dconf.
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

  toml = pkgs.formats.toml { };
  configFile = toml.generate "touchpad-snap-config.toml" cfg.settings;

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
      type = toml.type;
      default = { };
      example = lib.literalExpression ''
        {
          order = "mru";
          workspace = "all";
          hotkey = [
            { keys = "Super+Alt+N"; action = "next"; }
            { keys = "Super+Alt+P"; action = "prev"; }
          ];
        }
      '';
      description = ''
        The contents of {file}`$XDG_CONFIG_HOME/touchpad-snap/config.toml`,
        in the same TOML schema as a hand-written config file. An empty
        attribute set writes an empty file, which makes touchpad-snap use
        its built-in defaults.
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
          Install the GNOME Shell extension and enable it with Home Manager's
          `programs.gnome-shell.extensions`.
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
      home.packages = [ cfg.package ]
        ++ lib.optional cfg.gnome.enable cfg.gnome.extensionPackage;

      xdg.configFile."touchpad-snap/config.toml".source = configFile;
    }

    (lib.mkIf cfg.systemd.enable {
      systemd.user.services.touchpad-snap = {
        Unit = {
          Description = "touchpad-snap daemon";
          PartOf = [ "graphical-session.target" ];
          After = [ "graphical-session.target" ];
        };
        Install.WantedBy = [ "graphical-session.target" ];
        Service = {
          ExecStart = "${cfg.package}/bin/touchpad-snap daemon";
          Restart = "on-failure";
          RestartSec = 2;
        };
      };
    })

    # Home Manager's module writes org.gnome.shell.enabled-extensions from
    # this list (and disables the "user extensions" toggle), so it replaces
    # rather than extends a list managed by hand. Other extensions should be
    # added here too.
    (lib.mkIf cfg.gnome.enable {
      programs.gnome-shell = {
        enable = lib.mkDefault true;
        extensions = [
          {
            id = extensionUuid;
            package = cfg.gnome.extensionPackage;
          }
        ];
      };
    })
  ]);
}
