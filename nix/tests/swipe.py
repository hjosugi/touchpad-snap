"""A virtual touchpad (and keyboard) for the VM test, driven like hardware.

    swipe serve              create the devices and play what is sent
    swipe send [--mod ctrl|alt|shift|super]... [--fingers 3] [--dx 0] [--dy 0]

`serve` runs as a system service from boot, so the devices are there when
the GNOME session starts, as a built-in touchpad would be (a touchpad
plugged in moments before a swipe is not what is being tested). It creates a
uinput multitouch touchpad and a keyboard, waits until the compositor has
opened them, writes DIR/ready, then plays every command written to the FIFO
DIR/cmd and appends its number to DIR/done (DIR: --dir, /run/swipe).

`send` writes one command and waits until it has been played: fingers down,
moved by --dx/--dy millimetres (negative: left, up), lifted; the --mod keys
are held around the swipe. libinput turns the contacts into a swipe gesture,
so mutter, GNOME Shell and the touchpad-snap extension see what they would
see from hardware. Both need root (/dev/uinput, DIR).
"""

import argparse
import glob
import os
import time

from evdev import AbsInfo, UInput
from evdev import ecodes as e

RES = 40  # units per millimetre
# Tall enough for a 40 mm vertical swipe that starts above the bottom 15 %,
# where libinput takes resting touches for thumbs.
WIDTH, HEIGHT = 110 * RES, 80 * RES
TOOLS = {
    1: e.BTN_TOOL_FINGER,
    2: e.BTN_TOOL_DOUBLETAP,
    3: e.BTN_TOOL_TRIPLETAP,
    4: e.BTN_TOOL_QUADTAP,
}
MODIFIERS = {
    "ctrl": e.KEY_LEFTCTRL,
    "alt": e.KEY_LEFTALT,
    "shift": e.KEY_LEFTSHIFT,
    "super": e.KEY_LEFTMETA,
}
# Distinct ids: libinput pairs an external touchpad with a keyboard of the
# same vendor/product for disable-while-typing.
VENDOR = 0x1D6B
OPEN_TIMEOUT_S = 300.0


def touchpad():
    axis = AbsInfo(value=0, min=0, max=WIDTH, fuzz=0, flat=0, resolution=RES)
    yaxis = AbsInfo(value=0, min=0, max=HEIGHT, fuzz=0, flat=0, resolution=RES)
    caps = {
        e.EV_KEY: [e.BTN_LEFT, e.BTN_TOUCH, *TOOLS.values()],
        e.EV_ABS: [
            (e.ABS_X, axis),
            (e.ABS_Y, yaxis),
            (e.ABS_MT_SLOT, AbsInfo(0, 0, 4, 0, 0, 0)),
            (e.ABS_MT_TRACKING_ID, AbsInfo(0, 0, 65535, 0, 0, 0)),
            (e.ABS_MT_POSITION_X, axis),
            (e.ABS_MT_POSITION_Y, yaxis),
        ],
    }
    return UInput(
        caps,
        name="touchpad-snap test touchpad",
        vendor=VENDOR,
        product=0x0001,
        input_props=[e.INPUT_PROP_POINTER, e.INPUT_PROP_BUTTONPAD],
    )


def keyboard():
    keys = list(range(e.KEY_ESC, e.KEY_KPDOT + 1))
    return UInput(
        {e.EV_KEY: keys},
        name="touchpad-snap test keyboard",
        vendor=VENDOR,
        product=0x0002,
    )


def opened_by_shell(path):
    for fd in glob.glob("/proc/[0-9]*/fd/*"):
        pid = fd.split("/")[2]
        try:
            if os.readlink(fd) != path:
                continue
            with open(f"/proc/{pid}/comm") as comm:
                if "gnome-shell" in comm.read():
                    return True
        except OSError:
            pass
    return False


def wait_until_opened(dev):
    """Waits until GNOME Shell (through logind) holds the device's event
    node open: events written before that are lost. (logind itself opens
    keyboards to watch for power keys; that does not count.)"""
    path = dev.device.path
    deadline = time.time() + OPEN_TIMEOUT_S
    while time.time() < deadline:
        if opened_by_shell(path):
            return
        time.sleep(0.5)
    raise SystemExit(f"nobody opened {path} within {OPEN_TIMEOUT_S} s")


def swipe(pad, fingers, dx_mm, dy_mm, steps=40, step_s=0.012):
    # Fingers side by side, 15 mm apart, placed so the whole move stays on
    # the pad.
    x0 = WIDTH / 2 - dx_mm * RES / 2
    y0 = HEIGHT / 2 - dy_mm * RES / 2
    start = [
        (x0 + (i - (fingers - 1) / 2) * 15 * RES, y0 + (i % 2) * 3 * RES)
        for i in range(fingers)
    ]

    for k in range(steps + 1):
        first = None
        for i, (x, y) in enumerate(start):
            px = int(x + dx_mm * RES * k / steps)
            py = int(y + dy_mm * RES * k / steps)
            pad.write(e.EV_ABS, e.ABS_MT_SLOT, i)
            if k == 0:
                pad.write(e.EV_ABS, e.ABS_MT_TRACKING_ID, 100 + i)
            pad.write(e.EV_ABS, e.ABS_MT_POSITION_X, px)
            pad.write(e.EV_ABS, e.ABS_MT_POSITION_Y, py)
            first = first or (px, py)
        if k == 0:
            pad.write(e.EV_KEY, e.BTN_TOUCH, 1)
            pad.write(e.EV_KEY, TOOLS[fingers], 1)
        pad.write(e.EV_ABS, e.ABS_X, first[0])
        pad.write(e.EV_ABS, e.ABS_Y, first[1])
        pad.syn()
        time.sleep(step_s)

    for i in range(fingers):
        pad.write(e.EV_ABS, e.ABS_MT_SLOT, i)
        pad.write(e.EV_ABS, e.ABS_MT_TRACKING_ID, -1)
    pad.write(e.EV_KEY, e.BTN_TOUCH, 0)
    pad.write(e.EV_KEY, TOOLS[fingers], 0)
    pad.syn()


def play(pad, kbd, line):
    mods, fingers, dx, dy = line.split()
    keys = [MODIFIERS[m] for m in mods.split("+") if m != "none"]
    for key in keys:
        kbd.write(e.EV_KEY, key, 1)
    if keys:
        kbd.syn()
        time.sleep(0.2)
    swipe(pad, int(fingers), float(dx), float(dy))
    time.sleep(0.3)
    for key in reversed(keys):
        kbd.write(e.EV_KEY, key, 0)
    if keys:
        kbd.syn()


def serve(path):
    os.makedirs(path, exist_ok=True)
    fifo = os.path.join(path, "cmd")
    if not os.path.exists(fifo):
        os.mkfifo(fifo)
    pad, kbd = touchpad(), keyboard()
    wait_until_opened(pad)
    wait_until_opened(kbd)
    # Opened is not quite configured yet.
    time.sleep(1.0)
    open(os.path.join(path, "ready"), "w").close()
    played = 0
    while True:
        with open(fifo) as commands:
            for line in commands:
                play(pad, kbd, line)
                played += 1
                with open(os.path.join(path, "done"), "a") as done:
                    done.write(f"{played}\n")


def count_done(path):
    try:
        with open(os.path.join(path, "done")) as done:
            return len(done.readlines())
    except FileNotFoundError:
        return 0


def send(path, args):
    deadline = time.time() + OPEN_TIMEOUT_S
    while not os.path.exists(os.path.join(path, "ready")):
        if time.time() > deadline:
            raise SystemExit("the touchpad service never got ready")
        time.sleep(0.2)
    before = count_done(path)
    with open(os.path.join(path, "cmd"), "w") as cmd:
        mods = "+".join(args.mod) or "none"
        cmd.write(f"{mods} {args.fingers} {args.dx} {args.dy}\n")
    while count_done(path) <= before:
        time.sleep(0.1)
    # Let the compositor and the daemon finish with it.
    time.sleep(0.5)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--dir", default="/run/swipe")
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("serve")
    s = sub.add_parser("send")
    s.add_argument("--mod", action="append", default=[], choices=sorted(MODIFIERS), help="hold this modifier (repeatable)")
    s.add_argument("--fingers", type=int, default=3, choices=sorted(TOOLS))
    s.add_argument("--dx", type=float, default=0.0, help="millimetres")
    s.add_argument("--dy", type=float, default=0.0, help="millimetres")
    args = p.parse_args()
    if args.command == "serve":
        serve(args.dir)
    else:
        send(args.dir, args)


if __name__ == "__main__":
    main()
