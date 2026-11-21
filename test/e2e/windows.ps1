# End-to-end test on a real Windows desktop (the GitHub windows-2022 runner,
# or your own machine): opens two Notepad windows and a Character Map, then
# checks that touchpad-snap groups the Notepads and cycles between them only.
#
#   pwsh test/e2e/windows.ps1 -Bin target/debug/touchpad-snap.exe
#
# Add -VirtualDesktops to also cover a window on a second virtual desktop.
# That part is opt-in because it needs a desktop session where Win+Ctrl+D
# works (and assumes the test starts on a single desktop).
param(
    [Parameter(Mandatory = $true)][string]$Bin,
    [switch]$VirtualDesktops
)

$ErrorActionPreference = 'Stop'
$Bin = (Resolve-Path $Bin).Path

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Win {
    [StructLayout(LayoutKind.Sequential)]
    struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; }
    [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint mapType);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, StringBuilder l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);

    static INPUT Key(byte vk, bool up) {
        var i = new INPUT { type = 1 };
        i.ki.wVk = vk;
        i.ki.wScan = (ushort)MapVirtualKey(vk, 0);
        i.ki.dwFlags = up ? 2u : 0u;
        return i;
    }
    // One key event; returns 1 if Windows accepted it.
    public static uint Send(byte vk, bool up) {
        var sent = SendInput(1, new[] { Key(vk, up) }, Marshal.SizeOf(typeof(INPUT)));
        System.Threading.Thread.Sleep(30);
        return sent;
    }
    // Presses the keys in order and releases them in reverse, like a person.
    // Returns how many events Windows accepted (2 per key when all went in).
    public static uint Chord(params byte[] keys) {
        uint sent = 0;
        foreach (var k in keys) {
            sent += SendInput(1, new[] { Key(k, false) }, Marshal.SizeOf(typeof(INPUT)));
            System.Threading.Thread.Sleep(30);
        }
        for (int i = keys.Length - 1; i >= 0; i--) {
            sent += SendInput(1, new[] { Key(keys[i], true) }, Marshal.SizeOf(typeof(INPUT)));
            System.Threading.Thread.Sleep(30);
        }
        return sent;
    }
    public static bool HudVisible() {
        var h = FindWindow("touchpad-snap-hud", null);
        return h != IntPtr.Zero && IsWindowVisible(h);
    }
    // The text of a classic Notepad's edit control.
    public static string NotepadText(IntPtr notepad) {
        var edit = FindWindowEx(notepad, IntPtr.Zero, "Edit", null);
        var sb = new StringBuilder(4096);
        SendMessage(edit, 0x000D, (IntPtr)sb.Capacity, sb);  // WM_GETTEXT
        return sb.ToString();
    }
}
"@

function Fail([string]$msg) {
    Write-Host "FAIL: $msg"
    & $Bin list
    exit 1
}
function Ok([string]$msg) { Write-Host "ok:   $msg" }

function Snapshot { (& $Bin list --json | Out-String | ConvertFrom-Json).windows }
function Notepads { @(Snapshot | Where-Object { $_.app_id -like '*\notepad.exe' } | Sort-Object id) }
function Focused { (Snapshot | Where-Object { $_.focused } | Select-Object -First 1).id }

function WaitFor([string]$what, [scriptblock]$cond, [int]$seconds = 15) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $cond) { return }
        Start-Sleep -Milliseconds 200
    }
    Fail "timed out waiting for: $what"
}

$procs = @()
$daemon = $null
$work = Join-Path ([IO.Path]::GetTempPath()) "touchpad-snap-e2e-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $procs += Start-Process notepad -PassThru
    WaitFor 'first Notepad window' { (Notepads).Count -ge 1 }
    $procs += Start-Process notepad -PassThru
    WaitFor 'second Notepad window' { (Notepads).Count -ge 2 }
    $procs += Start-Process charmap -PassThru
    WaitFor 'Character Map window' { @(Snapshot | Where-Object { $_.app_id -like '*\charmap.exe' }).Count -ge 1 }

    $n = Notepads
    if ($n.Count -ne 2) { Fail "expected 2 Notepad windows, got $($n.Count)" }
    if ($n[0].app_id -ne $n[1].app_id) { Fail 'the two Notepads are not grouped as one app' }
    $charmap = (Snapshot | Where-Object { $_.app_id -like '*\charmap.exe' } | Select-Object -First 1).id
    Ok "two Notepads grouped ($($n[0].id), $($n[1].id)), Character Map $charmap"

    & $Bin activate $n[0].id
    if ($LASTEXITCODE -ne 0) { Fail 'activate failed' }
    WaitFor 'first Notepad focused' { (Focused) -eq $n[0].id }
    Ok 'activate'

    & $Bin next
    if ($LASTEXITCODE -ne 0) { Fail 'next failed' }
    WaitFor 'next -> second Notepad' { (Focused) -eq $n[1].id }
    Ok "next: $($n[0].id) -> $($n[1].id)"

    Start-Sleep -Milliseconds 500
    & $Bin next
    if ($LASTEXITCODE -ne 0) { Fail 'next failed' }
    WaitFor 'next wraps -> first Notepad' { (Focused) -eq $n[0].id }
    Ok 'next wraps round (Character Map skipped)'

    Start-Sleep -Milliseconds 500
    & $Bin prev
    if ($LASTEXITCODE -ne 0) { Fail 'prev failed' }
    WaitFor 'prev -> second Notepad' { (Focused) -eq $n[1].id }
    Ok 'prev'

    & $Bin doctor
    if ($LASTEXITCODE -ne 0) { Fail 'doctor reported problems' }
    Ok 'doctor'

    # The daemon, with its own config (never the user's): a hotkey typed on
    # the keyboard goes through RegisterHotKey, jumps, and shows the overlay.
    $cfg = Join-Path $work 'config.toml'
    Set-Content -Path $cfg -Encoding ascii -Value "[[hotkey]]`nkeys = `"Ctrl+Alt+N`"`naction = `"next`"`n"
    $log = Join-Path $work 'daemon.log'
    $daemon = Start-Process -FilePath $Bin -ArgumentList @('--config', $cfg, 'daemon', '--verbose') `
        -RedirectStandardError $log -RedirectStandardOutput (Join-Path $work 'daemon.out') -PassThru
    WaitFor 'daemon registration' { (Test-Path $log) -and (Select-String -Path $log -Pattern 'registered 1 hotkey' -Quiet) }
    Ok 'daemon registered Ctrl+Alt+N'
    # It must still be running a moment later (it once died at the first
    # paint of its overlay window).
    Start-Sleep -Seconds 3
    $daemon.Refresh()
    if ($daemon.HasExited) { Fail ('the daemon exited with 0x{0:X8}' -f $daemon.ExitCode) }

    & $Bin activate $n[0].id
    WaitFor 'first Notepad focused' { (Focused) -eq $n[0].id }
    # Synthetic input has to reach the desktop at all for this part to mean
    # anything: type into the focused Notepad and read it back.
    $sent = [Win]::Chord(0x51)  # Q
    WaitFor "typed key to reach Notepad (SendInput accepted $sent of 2)" { [Win]::NotepadText([IntPtr]$n[0].id) -match 'q' } 5
    $sent = [Win]::Chord(0x11, 0x12, 0x4E)  # Ctrl, Alt, N
    if ($sent -ne 6) { Fail "SendInput accepted only $sent of 6 events for Ctrl+Alt+N" }
    WaitFor 'Ctrl+Alt+N -> second Notepad' { (Focused) -eq $n[1].id }
    WaitFor 'overlay shown' { [Win]::HudVisible() } 5
    if (Snapshot | Where-Object { $_.wm_class -eq 'touchpad-snap-hud' }) { Fail 'the overlay shows up in the window list' }
    WaitFor 'overlay hidden again' { -not [Win]::HudVisible() } 5
    Ok 'daemon: Ctrl+Alt+N jumps, the overlay shows and hides, and is not listed'

    # Alt+Tab-style sessions with order = "mru": presses while Ctrl+Alt stay
    # down dig further back however slowly they come; releasing commits, and
    # the windows passed on the way do not count as used.
    $procs += Start-Process notepad -PassThru
    WaitFor 'third Notepad window' { (Notepads).Count -ge 3 }
    $n = Notepads
    Set-Content -Path $cfg -Encoding ascii -Value "order = `"mru`"`n[[hotkey]]`nkeys = `"Ctrl+Alt+N`"`naction = `"next`"`n"
    WaitFor 'daemon to load order = mru' { Select-String -Path $log -Pattern 'reloaded' -Quiet }
    foreach ($i in 2, 1, 0) {
        & $Bin activate $n[$i].id
        WaitFor "Notepad $i focused" { (Focused) -eq $n[$i].id }
    }
    Start-Sleep -Milliseconds 500
    # Hold Ctrl+Alt, press N, wait longer than mru_session_ms (1 s), press N again.
    $sent = [Win]::Send(0x11, $false) + [Win]::Send(0x12, $false) + [Win]::Send(0x4E, $false) + [Win]::Send(0x4E, $true)
    Start-Sleep -Milliseconds 1500
    $sent += [Win]::Send(0x4E, $false) + [Win]::Send(0x4E, $true)
    Start-Sleep -Milliseconds 300
    $sent += [Win]::Send(0x12, $true) + [Win]::Send(0x11, $true)
    if ($sent -ne 8) { Fail "SendInput accepted only $sent of 8 events" }
    WaitFor 'held Ctrl+Alt, N, N -> third Notepad' { (Focused) -eq $n[2].id }
    if (-not (Select-String -Path $log -Pattern 'keys still held' -Quiet)) { Fail 'the second press did not continue the session' }
    WaitFor 'the release to be reported' { Select-String -Path $log -Pattern 'hotkey modifiers released' -Quiet }
    # Z order is now 2, 1, 0, but 1 was only passed through: a new press goes
    # back to 0, where the session started.
    $sent = [Win]::Chord(0x11, 0x12, 0x4E)
    WaitFor 'next press -> back to the first Notepad' { (Focused) -eq $n[0].id }
    Ok 'daemon: held modifiers keep digging past the timeout, release commits'
    if ($VirtualDesktops) {
        # A second desktop, with one Notepad on it. Win+Ctrl+D switches to the
        # new desktop, so the Notepad starts there.
        $sent = [Win]::Chord(0x5B, 0x11, 0x44)  # Win, Ctrl, D
        if ($sent -ne 6) { Fail "SendInput accepted only $sent of 6 events for Win+Ctrl+D" }
        Start-Sleep -Seconds 2
        $second = Start-Process notepad -PassThru
        $procs += $second
        WaitFor 'Notepad on the second desktop' {
            @(Snapshot | Where-Object { $_.pid -eq $second.Id }).Count -ge 1
        }
        $far = Snapshot | Where-Object { $_.pid -eq $second.Id } | Select-Object -First 1
        if (-not $far) { Fail 'the Notepad on the second desktop is not listed' }

        # Back on the first desktop its windows are ws 0 and the far one is
        # reported on another workspace.
        $sent = [Win]::Chord(0x5B, 0x11, 0x25)  # Win, Ctrl, Left
        if ($sent -ne 6) { Fail "SendInput accepted only $sent of 6 events for Win+Ctrl+Left" }
        Start-Sleep -Seconds 2
        foreach ($w in $n) {
            $now = Snapshot | Where-Object { $_.id -eq $w.id } | Select-Object -First 1
            if ($now.workspace -ne 0) { Fail "a first-desktop Notepad is at ws $($now.workspace)" }
        }
        $far = Snapshot | Where-Object { $_.id -eq $far.id } | Select-Object -First 1
        if ($far.workspace -le 0) {
            Fail "the Notepad on the second desktop is at ws $($far.workspace), not another desktop"
        }
        Ok "second desktop: Notepad $($far.id) listed at ws $($far.workspace)"

        # Activating it must switch desktops and focus it.
        & $Bin activate $far.id
        if ($LASTEXITCODE -ne 0) { Fail 'activate on another desktop failed' }
        WaitFor 'activation switched to the second desktop' { (Focused) -eq $far.id }
        Ok 'activate switched to the window on the second desktop'

        # Leave the machine on the first desktop again.
        [Win]::Chord(0x5B, 0x11, 0x25) | Out-Null
        Start-Sleep -Seconds 2
    }

    Write-Host 'ALL PASSED'
}
finally {
    if ($daemon) {
        Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue
        Write-Host '--- daemon log ---'
        Get-Content (Join-Path $work 'daemon.log') -ErrorAction SilentlyContinue
    }
    $procs | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
