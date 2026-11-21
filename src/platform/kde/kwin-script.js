// touchpad-snap's KWin bridge.
//
// Loaded on demand through org.kde.KWin.Scripting; it is not a package and
// survives only until the touchpad-snap process that loaded it unloads it (or
// until KWin restarts). There is nothing to enable in System Settings.
//
// The script periodically sends a snapshot of the window list to the
// touchpad-snap D-Bus service and applies the single command the reply carries:
// an empty string means "do nothing", `activate <id>` focuses that window.
// The service name is substituted by the Rust side before the file is loaded.

var SERVICE = "__TOUCHPAD_SNAP_SERVICE__";
var OBJECT_PATH = "/io/github/hjosugi/TouchpadSnap/KWin";
var INTERFACE = "io.github.hjosugi.TouchpadSnap.KWin1";
var POLL_MS = 100;

var inFlight = false;

function desktopNumber(desktop) {
    try {
        return Number(desktop.x11DesktopNumber);
    } catch (e) {
        return 0;
    }
}

function desktopList(window) {
    var numbers = [];
    try {
        var desktops = window.desktops;
        for (var i = 0; i < desktops.length; i++) {
            numbers.push(desktopNumber(desktops[i]));
        }
    } catch (e) {
        // A window without a desktop list is on all desktops.
    }
    return numbers;
}

function activityList(window) {
    var activities = [];
    try {
        var list = window.activities;
        for (var i = 0; i < list.length; i++) {
            activities.push(String(list[i]));
        }
    } catch (e) {
        // Activities are informational; ignore an API that lacks them.
    }
    return activities;
}

// A stable identifier for the window. KWin 6 exposes internalId (a QUuid) on
// both X11 and Wayland; fall back to the X11 window id for older versions.
function windowToken(window) {
    try {
        if (window.internalId) {
            var internal = String(window.internalId);
            if (internal) {
                return internal;
            }
        }
    } catch (e) {
        // Fall through to windowId.
    }
    try {
        if (window.windowId !== undefined && window.windowId !== null) {
            return String(window.windowId);
        }
    } catch (e) {
        // No identifier at all; the caller skips this window.
    }
    return "";
}

function isNormalWindow(window) {
    try {
        if (typeof window.normalWindow === "boolean") {
            return window.normalWindow;
        }
    } catch (e) {
        // Older KWin: derive it from specialWindow below.
    }
    try {
        return !window.specialWindow;
    } catch (e) {
        return true;
    }
}

function bool(value) {
    return value === true || value === 1;
}

function number(value) {
    var n = Number(value);
    return isNaN(n) ? 0 : n;
}

function snapshot() {
    var result = {
        version: 1,
        active_workspace: 0,
        windows: []
    };

    try {
        var current = workspace.currentDesktop;
        if (current) {
            result.active_workspace = desktopNumber(current);
        }
    } catch (e) {
        // No virtual desktops; 0 is as good a value as any.
    }

    var windows = [];
    try {
        windows = workspace.windowList();
    } catch (e) {
        windows = [];
    }

    for (var i = 0; i < windows.length; i++) {
        var window = windows[i];
        var token = windowToken(window);
        if (!token) {
            continue;
        }

        var geometry = { x: 0, y: 0, width: 0, height: 0 };
        try {
            var frame = window.frameGeometry;
            geometry.x = Math.round(number(frame.x));
            geometry.y = Math.round(number(frame.y));
            geometry.width = Math.round(number(frame.width));
            geometry.height = Math.round(number(frame.height));
        } catch (e) {
            // Keep zeroes.
        }

        var deleted = false;
        try {
            deleted = bool(window.deleted);
        } catch (e) {
            // Not a KWin 6 window.
        }

        result.windows.push({
            id: token,
            title: String(window.caption || ""),
            resource_class: String(window.resourceClass || ""),
            resource_name: String(window.resourceName || ""),
            desktop_file: String(window.desktopFileName || ""),
            pid: number(window.pid),
            desktops: desktopList(window),
            on_all_desktops: bool(window.onAllDesktops),
            activities: activityList(window),
            minimized: bool(window.minimized),
            active: bool(window.active),
            skip_taskbar: bool(window.skipTaskbar),
            normal_window: isNormalWindow(window),
            deleted: deleted,
            stacking_order: number(window.stackingOrder),
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height
        });
    }

    return JSON.stringify(result);
}

function applyCommand(command) {
    if (!command) {
        return;
    }
    var parts = String(command).split(" ");
    if (parts.length !== 2 || parts[0] !== "activate") {
        return;
    }
    var wanted = parts[1];
    var windows = [];
    try {
        windows = workspace.windowList();
    } catch (e) {
        return;
    }
    for (var i = 0; i < windows.length; i++) {
        if (windowToken(windows[i]) === wanted) {
            try {
                workspace.activeWindow = windows[i];
            } catch (e) {
                // Focus stealing prevention or a window that just vanished.
            }
            return;
        }
    }
}

function poll() {
    if (inFlight) {
        return;
    }
    inFlight = true;
    callDBus(SERVICE, OBJECT_PATH, INTERFACE, "Poll", snapshot(), function (reply) {
        inFlight = false;
        if (reply) {
            applyCommand(reply);
            // Report the new focus state without waiting for the next tick.
            poll();
        }
    });
}

var timer = new QTimer();
timer.interval = POLL_MS;
timer.timeout.connect(poll);
timer.start();
poll();
