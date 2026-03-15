# 01_dump.ps1  -  dcmdump showcase
# Demonstrates the various output modes of dcmdump using the ABDOM CT series.
#
# Usage: pwsh -File 01_dump.ps1
#        (also works with: powershell -File 01_dump.ps1 on older Windows)
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if (($env:OS -eq 'Windows_NT') -or ((Test-Path variable:IsWindows) -and $IsWindows)) { '.exe' } else { '' }
$Bin       = Join-Path $Root "target\debug\dcmdump$Ext"
$Files     = Join-Path $ScriptDir '..\testfiles'

if (-not (Test-Path $Bin)) {
    Write-Error "Binary not found: $Bin`nRun: cargo build --bins"
}

$DCM = Join-Path $Files 'ABDOM_1.dcm'

function Banner($text) {
    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan
}

Banner "1. Default dump (dataset only, truncated values)"
& $Bin $DCM

Banner "2. Include File Meta Information header  (--meta)"
& $Bin --meta $DCM

Banner "3. No string length limit  (--no-limit)"
& $Bin --no-limit $DCM | Select-Object -First 30

Banner "4. DICOM JSON output  (--json)"
& $Bin --json $DCM | Select-Object -First 40

Banner "5. DICOM XML output  (--xml)"
& $Bin --xml $DCM | Select-Object -First 40

Banner "6. Dump all 5 slices  -  key tags only"
Get-ChildItem (Join-Path $Files 'ABDOM_*.dcm') | Sort-Object Name | ForEach-Object {
    Write-Host ""
    Write-Host "--- $($_.Name) ---" -ForegroundColor Yellow
    & $Bin $_.FullName | Where-Object {
        $_ -match '^\(0010,0010\)|^\(0008,0060\)|^\(0020,0013\)|^\(0028,0010\)|^\(0028,0011\)'
    }
}
