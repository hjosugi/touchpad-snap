# A real GNOME session in a VM, driven by a virtual touchpad.
#
# The headless shell of test/e2e/gnome.sh reads no input devices, so the
# extension's touchpad path (stage capture → SwipeClaimer → stop + forward)
# is proven here instead: a uinput touchpad, present from boot like a
# built-in one (swipe.py), plays three-finger swipes through libinput, mutter
# and GNOME Shell, exactly as hardware would.
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
    systemd.services.virtual-touchpad = {
      description = "Virtual touchpad and keyboard for the test (swipe.py)";
      wantedBy = [ "multi-user.target" ];
      after = [ "systemd-udevd.service" ];
      serviceConfig.ExecStart = "${swipe}/bin/swipe serve";
    };

    environment.systemPackages = [
      window-jump
      extension
      swipe
      pkgs.gnome-text-editor
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

    # Lets the test read the shell's state (Main.actionMode, grabs) through
    # org.gnome.Shell.Eval, as nixpkgs' own GNOME tests do. Without the PATH
    # line NixOS would give the shell its minimal default PATH, and the shell
    # treats every .desktop file whose program it cannot find as invalid.
    systemd.user.services."org.gnome.Shell@" = {
      serviceConfig.ExecStart = [
        ""
        "${pkgs.gnome-shell}/bin/gnome-shell --mode=%i --unsafe-mode"
      ];
      environment.PATH = pkgs.lib.mkForce null;
    };

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
        print("--- windows ---")
        print(machine.execute(as_alice("window-jump list --json"))[1])
        print(machine.execute("tail -n 20 /tmp/editor*.log")[1])
        print("--- shell state ---")
        print(shell_state())
        print(machine.execute("journalctl -b --no-pager | grep -E 'window-jump|gnome-shell|JS ' | tail -n 80")[1])
        print("--- signals from the extension ---")
        print(machine.execute("grep -E 'member=|uint32|double' /tmp/signals.log | head -n 60")[1])
        raise Exception(f"timed out waiting for: {what}")


    def shell_eval(js):
        return machine.execute(as_alice(f'gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval "{js}"'))[1].strip()


    def shell_state():
        return shell_eval("JSON.stringify({mode: Main.actionMode, overview: Main.overview.visible, modals: Main.modalCount, grab: String(global.stage.get_grab_actor()), key_focus: String(global.stage.get_key_focus())})")


    def focus_on(wid):
        # A window opened moments ago can still take focus after an
        # activation went through; insist until focus stays put.
        for _ in range(5):
            machine.succeed(as_alice(f"window-jump activate {wid}"))
            wait_for(f"window {wid} focused", lambda: focused() == wid)
            time.sleep(2)
            if focused() == wid:
                return
        raise Exception(f"window {wid} does not keep focus")


    def daemon_log():
        return machine.succeed("journalctl -b --no-pager _SYSTEMD_USER_UNIT=window-jump.service")


    start_all()

    with subtest("GNOME session with the extension and the daemon"):
        machine.wait_for_unit("display-manager.service")
        machine.wait_for_file("/run/user/${toString uid}/wayland-0")
        machine.wait_for_unit("default.target", "alice")
        machine.wait_for_file("/run/swipe/ready")
        wait_for("GNOME Shell start-up", lambda: shell_eval("Main.layoutManager._startingUp") == "(true, 'false')", 120)
        wait_for("daemon registration", lambda: "registered 2 hotkey(s) and 1 swipe claim(s)" in daemon_log(), 180)
        # What the extension sends the daemon, shown if something times out.
        machine.succeed(as_alice('dbus-monitor --session "type=signal,interface=org.hjosugi.WindowJump1" >/tmp/signals.log 2>&1 &'))

    with subtest("two windows of one app"):
        # The session starts in the overview, where swipes are not claimed.
        machine.send_key("esc")
        for n in (1, 2):
            machine.succeed(as_alice(f"gnome-text-editor --standalone >/tmp/editor{n}.log 2>&1 &"))
            wait_for(f"text editor window {n}", lambda: len(editors()) == n)
        first, second = editors()
        focus_on(first)
        assert snapshot()["active_workspace"] == 0

    with subtest("Ctrl + three fingers: claimed, jumps, workspace stays"):
        print("shell state before the swipe:", shell_state())
        # Long enough to clear the 120-unit threshold even after libinput has
        # spent the first millimetres deciding that this is a swipe.
        machine.succeed("swipe send --ctrl --fingers 3 --dx -40")
        wait_for("Ctrl + swipe left -> second editor", lambda: focused() == second, 20)
        assert "swipe Left with 3 fingers" in daemon_log()
        time.sleep(1)
        assert snapshot()["active_workspace"] == 0, "the claimed swipe also switched workspace"
        machine.screenshot("after-ctrl-swipe")

    with subtest("plain three fingers: left to GNOME, switches workspace"):
        jumps = daemon_log().count("swipe Left")
        machine.succeed("swipe send --fingers 3 --dx -40")
        wait_for("plain swipe left -> next workspace", lambda: snapshot()["active_workspace"] == 1, 20)
        assert daemon_log().count("swipe Left") == jumps, "the daemon saw an unclaimed swipe"
        machine.screenshot("after-plain-swipe")
  '';
}
