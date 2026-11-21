"""Plays a touchpad swipe through uinput, the way a real touchpad would.

    swipe [--ctrl] [--fingers 3] [--dx -20] [--dy 0]

Creates a multitouch touchpad (and, with --ctrl, a keyboard holding Left
Ctrl for the duration), moves the fingers by --dx/--dy millimetres, lifts
them and removes the devices. libinput turns the contacts into a swipe
gesture, so everything above it (mutter, GNOME Shell, the window-jump
extension) sees what it would see from hardware. Needs root (/dev/uinput).
"""

import argparse
import time

from evdev import AbsInfo, UInput
from evdev import ecodes as e

RES = 40  # units per millimetre
WIDTH, HEIGHT = 100 * RES, 60 * RES
TOOLS = {
    1: e.BTN_TOOL_FINGER,
    2: e.BTN_TOOL_DOUBLETAP,
    3: e.BTN_TOOL_TRIPLETAP,
    4: e.BTN_TOOL_QUADTAP,
}
# Time for the compositor to pick up a new device.
SETTLE_S = 2.0


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
        name="window-jump test touchpad",
        input_props=[e.INPUT_PROP_POINTER, e.INPUT_PROP_BUTTONPAD],
    )


def keyboard():
    keys = list(range(e.KEY_ESC, e.KEY_KPDOT + 1))
    return UInput({e.EV_KEY: keys}, name="window-jump test keyboard")


def swipe(pad, fingers, dx_mm, dy_mm, steps=40, step_s=0.012):
    # Fingers side by side, 15 mm apart, placed so the whole move stays on
    # the pad.
    x0 = WIDTH / 2 - dx_mm * RES / 2
    y0 = HEIGHT / 2 - dy_mm * RES / 2
    start = [
        (x0 + (i - (fingers - 1) / 2) * 15 * RES, y0 + (i % 2) * 3 * RES)
        for i in range(fingers)
    ]

    def frame(k):
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

    for k in range(steps + 1):
        frame(k)
    for i in range(fingers):
        pad.write(e.EV_ABS, e.ABS_MT_SLOT, i)
        pad.write(e.EV_ABS, e.ABS_MT_TRACKING_ID, -1)
    pad.write(e.EV_KEY, e.BTN_TOUCH, 0)
    pad.write(e.EV_KEY, TOOLS[fingers], 0)
    pad.syn()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--ctrl", action="store_true", help="hold Left Ctrl")
    p.add_argument("--fingers", type=int, default=3, choices=sorted(TOOLS))
    p.add_argument("--dx", type=float, default=-40.0, help="millimetres")
    p.add_argument("--dy", type=float, default=0.0, help="millimetres")
    args = p.parse_args()

    pad = touchpad()
    kbd = keyboard() if args.ctrl else None
    try:
        time.sleep(SETTLE_S)
        if kbd:
            kbd.write(e.EV_KEY, e.KEY_LEFTCTRL, 1)
            kbd.syn()
            time.sleep(0.2)
        swipe(pad, args.fingers, args.dx, args.dy)
        time.sleep(0.3)
        if kbd:
            kbd.write(e.EV_KEY, e.KEY_LEFTCTRL, 0)
            kbd.syn()
        time.sleep(0.5)
    finally:
        pad.close()
        if kbd:
            kbd.close()


if __name__ == "__main__":
    main()
