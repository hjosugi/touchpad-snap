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
public static class Win {
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string name);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    // Presses the keys in order and releases them in reverse, like a person.
    public static void Chord(params byte[] keys) {
        foreach (var k in keys) keybd_event(k, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(50);
        for (int i = keys.Length - 1; i >= 0; i--) keybd_event(keys[i], 0, 2, UIntPtr.Zero);
    }
    public static bool HudVisible() {
        var h = FindWindow("window-jump-hud", null);
        return h != IntPtr.Zero && IsWindowVisible(h);
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

    & $Bin activate $n[0].id
    WaitFor 'first Notepad focused' { (Focused) -eq $n[0].id }
    [Win]::Chord(0x11, 0x12, 0x4E)  # Ctrl, Alt, N
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
