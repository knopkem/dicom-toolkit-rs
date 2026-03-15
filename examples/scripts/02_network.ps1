# 02_network.ps1 — storescu / storescp / echoscu showcase
#
# Starts a Storage SCP on localhost:11112, sends all 5 ABDOM slices,
# then verifies the received files.
#
# Usage: pwsh -File 02_network.ps1
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }
$Dcmdump   = Join-Path $Root "target\debug\dcmdump$Ext"
$Echoscu   = Join-Path $Root "target\debug\echoscu$Ext"
$Storescu  = Join-Path $Root "target\debug\storescu$Ext"
$Storescp  = Join-Path $Root "target\debug\storescp$Ext"
$Files     = Join-Path $ScriptDir '..\testfiles'

foreach ($b in @($Dcmdump, $Echoscu, $Storescu, $Storescp)) {
    if (-not (Test-Path $b)) { Write-Error "Binary not found: $b`nRun: cargo build --bins" }
}

$Port    = 11112
$RecvDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $RecvDir | Out-Null

$ScpProc = $null

function Banner($text) {
    Write-Host ""
    Write-Host ("=" * 44) -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host ("=" * 44) -ForegroundColor Cyan
}

try {
    # ── Step 1: start Storage SCP ───────────────────────────────────
    Banner "Step 1 — Start Storage SCP on :$Port"

    # Redirect SCP output to log files so it doesn't interleave with our output.
    $LogOut = Join-Path $RecvDir 'scp.out'
    $LogErr = Join-Path $RecvDir 'scp.err'
    $ScpProc = Start-Process -FilePath $Storescp `
        -ArgumentList @('-v', '-a', 'STORESCP', '-d', $RecvDir, "$Port") `
        -NoNewWindow -PassThru `
        -RedirectStandardOutput $LogOut `
        -RedirectStandardError  $LogErr
    Write-Host "storescp PID $($ScpProc.Id)  →  saving to $RecvDir"
    Start-Sleep -Milliseconds 500

    if ($ScpProc.HasExited) {
        throw "storescp failed to start (port $Port in use?)"
    }

    # ── Step 2: C-ECHO verification ──────────────────────────────────
    Banner "Step 2 — C-ECHO verification"
    & $Echoscu -v -a DEMO_SCU -c STORESCP localhost $Port

    # ── Step 3: send files ───────────────────────────────────────────
    Banner "Step 3 — Send 5 CT slices with storescu"
    $DicomFiles = (Get-ChildItem (Join-Path $Files 'ABDOM_*.dcm') | Sort-Object Name).FullName
    & $Storescu -v -a DEMO_SCU -c STORESCP localhost $Port @DicomFiles

    # ── Step 4: verify received files ───────────────────────────────
    Banner "Step 4 — Verify received files"
    $Received = Get-ChildItem (Join-Path $RecvDir '*.dcm') | Sort-Object Name
    Write-Host "Received $($Received.Count) file(s):"
    foreach ($f in $Received) {
        Write-Host ""
        Write-Host "  File: $($f.Name)" -ForegroundColor Yellow
        & $Dcmdump $f.FullName | Where-Object {
            $_ -match '^\(0010,0010\)|^\(0008,0016\)|^\(0020,0013\)|^\(0028,0010\)|^\(0028,0011\)'
        } | ForEach-Object { Write-Host "    $_" }
    }

    $Total = (Get-ChildItem (Join-Path $Files 'ABDOM_*.dcm')).Count
    Banner "Done — $($Received.Count)/$Total files transferred successfully"

} finally {
    if ($null -ne $ScpProc -and -not $ScpProc.HasExited) {
        Stop-Process -Id $ScpProc.Id -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $RecvDir -ErrorAction SilentlyContinue
}
