# 03_query.ps1 — findscu showcase
#
# Shows findscu command-line patterns for C-FIND queries.
# Set $env:PACS_HOST / $env:PACS_PORT to query a real SCP,
# or set $env:RUN_LIVE = "1" to execute the live query.
#
# Usage: pwsh -File 03_query.ps1
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }
$Findscu   = Join-Path $Root "target\debug\findscu$Ext"

if (-not (Test-Path $Findscu)) {
    Write-Error "Binary not found: $Findscu`nRun: cargo build --bins"
}

$PacsHost = if ($env:PACS_HOST) { $env:PACS_HOST } else { 'localhost' }
$PacsPort = if ($env:PACS_PORT) { $env:PACS_PORT } else { '4242' }

function Banner($text) {
    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan
}
function Example($label, $cmd) {
    Write-Host ""
    Write-Host $label -ForegroundColor Yellow
    Write-Host "Command:" -ForegroundColor Gray
    Write-Host "  $cmd" -ForegroundColor White
}

Banner "findscu — C-FIND query examples"
Write-Host ""
Write-Host " Set `$env:PACS_HOST / `$env:PACS_PORT to target a running QR SCP."

Example "Example 1: Find all studies (wildcard)" `
    "findscu -L STUDY -k '0008,0052=STUDY' -k '0010,0010=' ${PacsHost} ${PacsPort}"

Example "Example 2: Find by patient name prefix" `
    "findscu -L STUDY -k '0010,0010=AVE*' ${PacsHost} ${PacsPort}"

Example "Example 3: Find series within a study" `
    "findscu -L SERIES -k '0020,000D=<StudyInstanceUID>' ${PacsHost} ${PacsPort}"

Example "Example 4: Find CT studies in a date range" `
    "findscu -L STUDY -k '0008,0060=CT' -k '0008,0020=19960101-19971231' ${PacsHost} ${PacsPort}"

if ($env:RUN_LIVE -eq '1') {
    Banner "LIVE query against ${PacsHost}:${PacsPort}"
    & $Findscu -v -a FINDSCU -c ANY-SCP -L STUDY -k '0010,0010=' $PacsHost $PacsPort
} else {
    Write-Host ""
    Write-Host "(Set `$env:RUN_LIVE='1' to execute queries against ${PacsHost}:${PacsPort})" -ForegroundColor DarkGray
}
