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
# Asserts that Alt + three fingers left types Ctrl+Shift+N (a new editor
# window) once Alt is up, and that the default swipes jump between the app's windows and leave
# GNOME alone (Ctrl + three fingers left: no workspace switch; Alt + three
# fingers up/down: no overview), that plain three fingers still open the
# overview and switch workspace in apps without a binding, and that a plain
# swipe bound only for the editor jumps there without leaving the workspace.
{
  pkgs,
  touchpad-snap,
  extension,
}:

let
  uid = 1000;
  swipe = pkgs.writers.writePython3Bin "swipe" {
    libraries = [ pkgs.python3Packages.evdev ];
    flakeIgnore = [ "E501" ];
  } (builtins.readFile ./swipe.py);
  # One jump per swipe, whatever distance libinput reports. The last binding
  # is app-filtered: plain three fingers left is ours only in the editor, and
  # stays GNOME's everywhere else.
  config = pkgs.writeText "touchpad-snap.json" ''
    {
      "version": 2,
      "gesture": { "repeat_step": 0.0 },
      "bindings": [
        { "trigger": { "type": "hotkey", "keys": "Super+Alt+N" }, "actions": [ { "type": "window.next" } ] },
        { "trigger": { "type": "hotkey", "keys": "Super+Alt+P" }, "actions": [ { "type": "window.prev" } ] },
        { "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left" }, "actions": [ { "type": "window.next" } ] },
        { "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "right" }, "actions": [ { "type": "window.prev" } ] },
        { "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "up" }, "actions": [ { "type": "window.next" } ] },
        { "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "down" }, "actions": [ { "type": "window.prev" } ] },
        { "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "left" }, "actions": [ { "type": "keys", "keys": "Ctrl+Shift+N" } ] },
        {
          "trigger": { "type": "swipe", "fingers": 3, "direction": "left" },
          "when": { "app": ["*TextEditor*"] },
          "actions": [ { "type": "window.next" } ]
        }
      ]
    }
  '';
in
pkgs.testers.runNixOSTest {
  name = "touchpad-snap-touchpad";

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
      touchpad-snap
      extension
      swipe
      pkgs.gnome-text-editor
      pkgs.gnome-calculator
      pkgs.libinput
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

    systemd.user.services.touchpad-snap = {
      description = "touchpad-snap daemon";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      serviceConfig.ExecStart = "${touchpad-snap}/bin/touchpad-snap --config ${config} daemon --verbose";
    };
  };

  testScript = ''
    import json
    import time

    ENV = "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${toString uid}/bus XDG_RUNTIME_DIR=/run/user/${toString uid} WAYLAND_DISPLAY=wayland-0"


    def as_alice(cmd):
        return f"su - alice -c '{ENV} {cmd}'"


    def snapshot():
        return json.loads(machine.succeed(as_alice("touchpad-snap list --json")))


    def editors():
        return sorted(w["id"] for w in snapshot()["windows"] if "TextEditor" in w["app_id"])


    def calculator():
        return next((w["id"] for w in snapshot()["windows"]
                     if "Calculator" in w["app_id"]), 0)


    def focused():
        return next((w["id"] for w in snapshot()["windows"] if w["focused"]), 0)


    def diagnose():
        machine.screenshot("timeout")
        print("--- windows ---")
        print(machine.execute(as_alice("touchpad-snap list --json"))[1])
        print(machine.execute("tail -n 20 /tmp/editor*.log")[1])
        print("--- shell state ---")
        print(shell_state())
        print(machine.execute("journalctl -b --no-pager | grep -E 'touchpad-snap|gnome-shell|JS ' | tail -n 80")[1])
        print("--- the last signals from the extension (phase, fingers, mods, dx, dy) ---")
        print(machine.execute("grep -E 'uint32|double' /tmp/signals.log | awk '{print $2}' | paste -d' ' - - - - - | tail -n 25")[1])
        print("--- the last gestures libinput reported ---")
        print(machine.execute("grep -E 'GESTURE|KEYBOARD_KEY' /tmp/libinput.log | tail -n 40")[1])


    def wait_for(what, pred, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pred():
                return
            time.sleep(0.5)
        diagnose()
        raise Exception(f"timed out waiting for: {what}")


    def shell_eval(js):
        return machine.execute(as_alice(f'gdbus call --session -d org.gnome.Shell -o /org/gnome/Shell -m org.gnome.Shell.Eval "{js}"'))[1].strip()


    def shell_state():
        return shell_eval("JSON.stringify({mode: Main.actionMode, overview: Main.overview.visible, modals: Main.modalCount, grab: String(global.stage.get_grab_actor()), key_focus: String(global.stage.get_key_focus())})")


    def overview():
        return shell_eval("Main.overview.visible") == "(true, 'true')"


    def focus_on(wid):
        # A window opened moments ago takes focus when it is mapped, which
        # can be before or after an activation went through; insist until
        # focus is there and stays.
        for _ in range(10):
            machine.succeed(as_alice(f"touchpad-snap activate {wid}"))
            time.sleep(2)
            if focused() != wid:
                continue
            time.sleep(1)
            if focused() == wid:
                return
        diagnose()
        raise Exception(f"window {wid} does not keep focus")


    def daemon_log():
        return machine.succeed("journalctl -b --no-pager _SYSTEMD_USER_UNIT=touchpad-snap.service")


    start_all()

    with subtest("GNOME session with the extension and the daemon"):
        machine.wait_for_unit("display-manager.service")
        machine.wait_for_file("/run/user/${toString uid}/wayland-0")
        machine.wait_for_unit("default.target", "alice")
        machine.wait_for_file("/run/swipe/ready")
        wait_for("GNOME Shell start-up", lambda: shell_eval("Main.layoutManager._startingUp") == "(true, 'false')", 120)
        wait_for("daemon registration", lambda: "registered 2 hotkey(s) and 3 swipe claim(s)" in daemon_log(), 180)
        # What the extension sends the daemon and what libinput makes of the
        # virtual devices, shown if something times out.
        machine.succeed(as_alice('dbus-monitor --session "type=signal,interface=io.github.hjosugi.TouchpadSnap1" >/tmp/signals.log 2>&1 &'))
        machine.succeed("libinput debug-events --show-keycodes >/tmp/libinput.log 2>&1 &")

    with subtest("two windows of one app"):
        # The session starts in the overview, where swipes are not claimed.
        machine.send_key("esc")
        for n in (1, 2):
            machine.succeed(as_alice(f"gnome-text-editor --standalone >/tmp/editor{n}.log 2>&1 &"))
            wait_for(f"text editor window {n}", lambda: len(editors()) == n)
        first, second = editors()
        # A second app, where the editor-only plain swipe must stay GNOME's.
        machine.succeed(as_alice("gnome-calculator >/tmp/calc.log 2>&1 &"))
        wait_for("calculator window", lambda: calculator() != 0)
        focus_on(first)
        assert snapshot()["active_workspace"] == 0

    with subtest("Ctrl + three fingers: claimed, jumps, workspace stays"):
        print("shell state before the swipe:", shell_state())
        # Long enough to clear the 120-unit threshold even after libinput has
        # spent the first millimetres deciding that this is a swipe.
        machine.succeed("swipe send --mod ctrl --fingers 3 --dx -40")
        wait_for("Ctrl + swipe left -> second editor", lambda: focused() == second, 20)
        assert "swipe Left with 3 fingers" in daemon_log()
        time.sleep(1)
        assert snapshot()["active_workspace"] == 0, "the claimed swipe also switched workspace"
        machine.screenshot("after-ctrl-swipe")

    with subtest("Alt + three fingers up/down: next/prev, no overview"):
        # A third window tells next from prev: [first, second, third].
        machine.succeed(as_alice("gnome-text-editor --standalone >/tmp/editor3.log 2>&1 &"))
        wait_for("text editor window 3", lambda: len(editors()) == 3)
        third = editors()[2]
        assert editors()[:2] == [first, second]
        focus_on(first)
        machine.succeed("swipe send --mod alt --fingers 3 --dy -40")
        wait_for("Alt + swipe up -> second editor", lambda: focused() == second, 20)
        machine.succeed("swipe send --mod alt --fingers 3 --dy -40")
        wait_for("Alt + swipe up -> third editor", lambda: focused() == third, 20)
        machine.succeed("swipe send --mod alt --fingers 3 --dy 40")
        wait_for("Alt + swipe down -> second editor", lambda: focused() == second, 20)
        log = daemon_log()
        assert log.count("swipe Up with 3 fingers") == 2 and log.count("swipe Down with 3 fingers") == 1, log
        time.sleep(1)
        assert not overview(), "the claimed swipe also opened the overview"
        assert snapshot()["active_workspace"] == 0
        machine.screenshot("after-alt-swipes")

    with subtest("plain three fingers in the editor: claimed by the app filter and jumps"):
        jumps = daemon_log().count("swipe Left")
        focus_on(first)
        machine.succeed("swipe send --fingers 3 --dx -40")
        wait_for("plain swipe left in the editor -> second editor", lambda: focused() == second, 20)
        wait_for("daemon saw the editor's plain swipe",
                 lambda: daemon_log().count("swipe Left") == jumps + 1, 20)
        time.sleep(1)
        assert snapshot()["active_workspace"] == 0, "the app-filtered swipe also switched workspace"
        machine.screenshot("after-plain-editor-swipe")

    with subtest("plain three fingers up in another app: opens the overview"):
        # A GNOME claim is per (fingers, modifiers, app), not per direction:
        # plain swipes stay claimed in the editor in every direction. In an
        # app without a claim the gesture is the shell's.
        focus_on(calculator())
        jumps = daemon_log().count("swipe Up")
        machine.succeed("swipe send --fingers 3 --dy -40")
        wait_for("plain swipe up -> overview", overview, 20)
        assert daemon_log().count("swipe Up") == jumps, "the daemon saw an unclaimed swipe"
        machine.screenshot("after-plain-swipe-up")
        machine.send_key("esc")
        wait_for("overview closed again", lambda: not overview(), 20)

    with subtest("plain three fingers in another app: left to GNOME, switches workspace"):
        # The editor-only claim must not steal the calculator's swipe.
        focus_on(calculator())
        jumps = daemon_log().count("swipe Left")
        machine.succeed("swipe send --fingers 3 --dx -40")
        wait_for("plain swipe left -> next workspace", lambda: snapshot()["active_workspace"] == 1, 20)
        assert daemon_log().count("swipe Left") == jumps, "the daemon saw an unclaimed swipe"
        machine.screenshot("after-plain-swipe")

    with subtest("Alt + three fingers left: types Ctrl+Shift+N once Alt is up"):
        # The keys action: the virtual keyboard cannot release the physical
        # Alt, so the shortcut waits until the (virtual) hand lets go of it.
        # With Alt still down it would arrive as Alt+Ctrl+Shift+N and open
        # nothing.
        before = len(editors())
        focus_on(second)
        machine.succeed("swipe send --mod alt --fingers 3 --dx -40")
        wait_for("a new editor window from Ctrl+Shift+N", lambda: len(editors()) == before + 1, 30)
        assert "keys Ctrl+Shift+N" in daemon_log()
        machine.screenshot("after-keys-swipe")
  '';
}
