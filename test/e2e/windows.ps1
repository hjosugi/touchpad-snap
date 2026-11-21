# End-to-end test on a real Windows desktop (the GitHub windows-2022 runner,
# or your own machine): opens two Notepad windows and a Character Map, then
# checks that window-jump groups the Notepads and cycles between them only.
#
#   pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe
param(
    [Parameter(Mandatory = $true)][string]$Bin
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
    [DllImport("user32.dll", SetLastError = true)] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
    [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
    [DllImport("user32.dll")] static extern bool PeekMessage(out MSG m, IntPtr h, uint min, uint max, uint remove);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool PostThreadMessage(uint thread, uint msg, IntPtr w, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam; public uint time; public int x; public int y; }
    // DIAGNOSTIC: does a hotkey registered by this very thread fire on SendInput?
    public static string SelfHotkey(params byte[] keys) {
        if (!RegisterHotKey(IntPtr.Zero, 77, 0x4000 | 0x1 | 0x2, 0x4D)) return "register failed " + Marshal.GetLastWin32Error();
        Chord(keys);
        var until = DateTime.Now.AddSeconds(3);
        MSG m;
        while (DateTime.Now < until) {
            if (PeekMessage(out m, IntPtr.Zero, 0, 0, 1) && m.message == 0x0312) { UnregisterHotKey(IntPtr.Zero, 77); return "fired"; }
            System.Threading.Thread.Sleep(20);
        }
        UnregisterHotKey(IntPtr.Zero, 77);
        return "not fired";
    }
    public static bool HudVisible() {
        var h = FindWindow("window-jump-hud", null);
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
$work = Join-Path ([IO.Path]::GetTempPath()) "window-jump-e2e-$PID"
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
    Start-Sleep -Seconds 3
    $daemon.Refresh()
    Write-Host ("DIAGNOSTIC after 3 s, daemon exited: {0} code: {1}" -f $daemon.HasExited, $(if ($daemon.HasExited) { '0x{0:X8}' -f $daemon.ExitCode } else { '-' }))

    & $Bin activate $n[0].id
    WaitFor 'first Notepad focused' { (Focused) -eq $n[0].id }
    # Synthetic input has to reach the desktop at all for this part to mean
    # anything: type into the focused Notepad and read it back.
    $sent = [Win]::Chord(0x51)  # Q
    WaitFor "typed key to reach Notepad (SendInput accepted $sent of 2)" { [Win]::NotepadText([IntPtr]$n[0].id) -match 'q' } 5
    Write-Host "DIAGNOSTIC self hotkey Ctrl+Alt+M: $([Win]::SelfHotkey(0x11, 0x12, 0x4D))"
    $sent = [Win]::Chord(0x11, 0x12, 0x4E)  # Ctrl, Alt, N
    Write-Host "SendInput accepted $sent of 6 events for Ctrl+Alt+N"
    Start-Sleep -Seconds 2
    $daemon.Refresh()
    Write-Host ("DIAGNOSTIC daemon exited: {0} code: {1}" -f $daemon.HasExited, $(if ($daemon.HasExited) { '0x{0:X8}' -f $daemon.ExitCode } else { '-' }))
    WaitFor 'Ctrl+Alt+N -> second Notepad' { (Focused) -eq $n[1].id }
    WaitFor 'overlay shown' { [Win]::HudVisible() } 5
    if (Snapshot | Where-Object { $_.wm_class -eq 'window-jump-hud' }) { Fail 'the overlay shows up in the window list' }
    WaitFor 'overlay hidden again' { -not [Win]::HudVisible() } 5
    Ok 'daemon: Ctrl+Alt+N jumps, the overlay shows and hides, and is not listed'

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
