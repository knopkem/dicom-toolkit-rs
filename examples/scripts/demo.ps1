# demo.ps1  -  full dicom-toolkit-rs CLI demonstration
#
# Runs all showcase scripts in order against the ABDOM CT test series.
# Builds the workspace first if the binaries are not present.
#
# Usage:
#   pwsh -File demo.ps1             # run all demos non-interactively
#   pwsh -File demo.ps1 -Pause      # pause between demos (interactive)
#
# Windows execution policy note:
#   If scripts are blocked, run once: Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
[CmdletBinding()]
param(
    [switch]$Pause
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if (($env:OS -eq 'Windows_NT') -or ((Test-Path variable:IsWindows) -and $IsWindows)) { '.exe' } else { '' }
$Dcmdump   = Join-Path $Root "target\debug\dcmdump$Ext"

function Banner($text) {
    Write-Host ""
    Write-Host ("*" * 64) -ForegroundColor Magenta
    Write-Host ("  " + $text) -ForegroundColor Magenta
    Write-Host ("*" * 64) -ForegroundColor Magenta
}

function MaybePause {
    if ($Pause) {
        Write-Host ""
        Read-Host "  Press Enter to continue"
    }
}

# ── Build if needed ──────────────────────────────────────────────────────────
if (-not (Test-Path $Dcmdump)) {
    Write-Host "Building workspace..." -ForegroundColor Yellow
    Push-Location $Root
    try { cargo build --bins }
    finally { Pop-Location }
}

# ── 01: dcmdump ──────────────────────────────────────────────────────────────
Banner "01 · dcmdump  -  print DICOM file contents"
& "$ScriptDir\01_dump.ps1"
MaybePause

# ── 02: network ──────────────────────────────────────────────────────────────
Banner "02 · echoscu + storescu + storescp  -  network transfer"
& "$ScriptDir\02_network.ps1"
MaybePause

# ── 03: query ────────────────────────────────────────────────────────────────
Banner "03 · findscu  -  C-FIND query examples"
& "$ScriptDir\03_query.ps1"
MaybePause

# ── 04: img2dcm ──────────────────────────────────────────────────────────────
Banner "04 · img2dcm  -  PNG to DICOM Secondary Capture"
& "$ScriptDir\04_img2dcm.ps1"
MaybePause

# ── 05: jpegls ───────────────────────────────────────────────────────────────
Banner "05 · dcmcjpls + dcmdjpls  -  JPEG-LS compress / decompress"
& "$ScriptDir\05_jpegls.ps1"

Write-Host ""
Write-Host ("*" * 64) -ForegroundColor Green
Write-Host "  All demos complete." -ForegroundColor Green
Write-Host ("*" * 64) -ForegroundColor Green
