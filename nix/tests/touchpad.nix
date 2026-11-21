# A real GNOME session in a VM, driven by a virtual touchpad.
#
# The headless shell of test/e2e/gnome.sh reads no input devices, so the
# extension's touchpad path (stage capture → SwipeClaimer → stop + forward)
# is proven here instead: a uinput touchpad plays three-finger swipes through
# libinput, mutter and GNOME Shell, exactly as hardware would.
#
#   nix build .#checks.x86_64-linux.vm-touchpad -L      (needs KVM)
#
# Asserts that Ctrl + three fingers jumps to the app's other window without
# switching workspace, and that plain three fingers still switch workspace.
{
  pkgs,
  window-jump,
  extension,
}:

let
  uid = 1000;
  swipe = pkgs.writers.writePython3Bin "swipe" {
    libraries = [ pkgs.python3Packages.evdev ];
    flakeIgnore = [ "E501" ];
  } (builtins.readFile ./swipe.py);
  # One jump per swipe, whatever distance libinput reports.
  config = pkgs.writeText "window-jump.toml" ''
    [gesture]
    repeat_step = 0.0
  '';
in
pkgs.testers.runNixOSTest {
  name = "window-jump-touchpad";

  nodes.machine = {
    virtualisation.memorySize = 3072;
    virtualisation.cores = 2;

    users.users.alice = {
      isNormalUser = true;
      inherit uid;
      password = "alice";
    };

    services.displayManager.gdm.enable = true;
    services.displayManager.autoLogin = {
      enable = true;
      user = "alice";
    };
    services.desktopManager.gnome.enable = true;
    services.gnome.gnome-initial-setup.enable = false;

    hardware.uinput.enable = true;

    environment.systemPackages = [
      window-jump
      extension
      swipe
      pkgs.gnome-text-editor
      pkgs.libinput # diagnostics: libinput list-devices
    ];

    programs.dconf.profiles.user.databases = [
      {
        settings = {
          "org/gnome/shell" = {
            enabled-extensions = [ extension.extensionUuid ];
            welcome-dialog-last-shown-version = "999";
          };
          # A locked screen disables the extension; never lock during the test.
          "org/gnome/desktop/session".idle-delay = pkgs.lib.gvariant.mkUint32 0;
          "org/gnome/desktop/screensaver".lock-enabled = false;
        };
      }
    ];

    systemd.user.services.window-jump = {
      description = "window-jump daemon";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      serviceConfig.ExecStart = "${window-jump}/bin/window-jump --config ${config} daemon --verbose";
    };
  };

  testScript = ''
    import json
    import time

    ENV = "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${toString uid}/bus XDG_RUNTIME_DIR=/run/user/${toString uid} WAYLAND_DISPLAY=wayland-0"


    def as_alice(cmd):
        return f"su - alice -c '{ENV} {cmd}'"


    def snapshot():
        return json.loads(machine.succeed(as_alice("window-jump list --json")))


    def editors():
        return sorted(w["id"] for w in snapshot()["windows"] if "TextEditor" in w["app_id"])


    def focused():
        return next((w["id"] for w in snapshot()["windows"] if w["focused"]), 0)


    def wait_for(what, pred, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pred():
                return
            time.sleep(0.5)
        machine.screenshot("timeout")
        print(machine.execute("journalctl -b --no-pager | grep -E 'window-jump|gnome-shell' | tail -n 80")[1])
        raise Exception(f"timed out waiting for: {what}")


    def daemon_log():
        return machine.succeed("journalctl -b --no-pager _SYSTEMD_USER_UNIT=window-jump.service")


    start_all()

    with subtest("GNOME session with the extension and the daemon"):
        machine.wait_for_unit("display-manager.service")
        machine.wait_for_file("/run/user/${toString uid}/wayland-0")
        machine.wait_for_unit("default.target", "alice")
        wait_for("daemon registration", lambda: "registered 2 hotkey(s) and 1 swipe claim(s)" in daemon_log(), 180)

    with subtest("two windows of one app"):
        # The session starts in the overview, where swipes are not claimed.
        machine.send_key("esc")
        for n in (1, 2):
            machine.succeed(as_alice("gnome-text-editor --standalone >/dev/null 2>&1 &"))
            wait_for(f"text editor window {n}", lambda: len(editors()) == n)
        first, second = editors()
        machine.succeed(as_alice(f"window-jump activate {first}"))
        wait_for("first editor focused", lambda: focused() == first)
        assert snapshot()["active_workspace"] == 0

    with subtest("Ctrl + three fingers: claimed, jumps, workspace stays"):
        # Long enough to clear the 120-unit threshold even after libinput has
        # spent the first millimetres deciding that this is a swipe.
        machine.succeed("swipe --ctrl --fingers 3 --dx -40")
        wait_for("Ctrl + swipe left -> second editor", lambda: focused() == second, 20)
        assert "swipe Left with 3 fingers" in daemon_log()
        time.sleep(1)
        assert snapshot()["active_workspace"] == 0, "the claimed swipe also switched workspace"
        machine.screenshot("after-ctrl-swipe")

    with subtest("plain three fingers: left to GNOME, switches workspace"):
        jumps = daemon_log().count("swipe Left")
        machine.succeed("swipe --fingers 3 --dx -40")
        wait_for("plain swipe left -> next workspace", lambda: snapshot()["active_workspace"] == 1, 20)
        assert daemon_log().count("swipe Left") == jumps, "the daemon saw an unclaimed swipe"
        machine.screenshot("after-plain-swipe")
  '';
}
