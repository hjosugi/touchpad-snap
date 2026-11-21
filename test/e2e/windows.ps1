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

    Write-Host 'ALL PASSED'
}
finally {
    $procs | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
}
