# 05_jpegls.ps1  -  JPEG-LS compression / decompression showcase
#
# Demonstrates dcmcjpls and dcmdjpls with the ABDOM CT test slices:
#   1. Lossless compress → decompress round-trip
#   2. Near-lossless compress → decompress round-trip
#   3. Batch compress all test files
#   4. Verify round-trip integrity by comparing dcmdump output
#
# Usage: pwsh -File 05_jpegls.ps1
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if (($env:OS -eq 'Windows_NT') -or ((Test-Path variable:IsWindows) -and $IsWindows)) { '.exe' } else { '' }
$Cjpls     = Join-Path $Root "target\debug\dcmcjpls$Ext"
$Djpls     = Join-Path $Root "target\debug\dcmdjpls$Ext"
$Dump      = Join-Path $Root "target\debug\dcmdump$Ext"
$Files     = Join-Path $ScriptDir '..\testfiles'

foreach ($b in @($Cjpls, $Djpls, $Dump)) {
    if (-not (Test-Path $b)) {
        Write-Error "Binary not found: $b`nRun: cargo build --bins"
    }
}

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "dcmtk-jpegls-demo"
if (Test-Path $TmpDir) { Remove-Item -Recurse -Force $TmpDir }
New-Item -ItemType Directory -Path $TmpDir | Out-Null

$DCM = Join-Path $Files 'ABDOM_1.dcm'

function Banner($text) {
    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan
}

function FileSize($path) {
    (Get-Item $path).Length
}

try {

# ── 1. Lossless round-trip ────────────────────────────────────────────────────
Banner "1. Lossless JPEG-LS round-trip"

$compLossless = Join-Path $TmpDir 'compressed_lossless.dcm'
$rtLossless   = Join-Path $TmpDir 'roundtrip_lossless.dcm'

Write-Host "  Compressing:  ABDOM_1.dcm -> compressed_lossless.dcm"
& $Cjpls -v $DCM $compLossless
Write-Host ""
Write-Host "  Decompressing: compressed_lossless.dcm -> roundtrip_lossless.dcm"
& $Djpls -v $compLossless $rtLossless

Write-Host ""
Write-Host "  File sizes:"
$origSize = FileSize $DCM
$compSize = FileSize $compLossless
$rtSize   = FileSize $rtLossless
Write-Host "    Original:     $origSize bytes"
Write-Host "    Compressed:   $compSize bytes"
Write-Host "    Round-trip:   $rtSize bytes"

# Show transfer syntax from dumps
Write-Host ""
Write-Host "  Transfer syntax of each file:"
$tsOrig = (& $Dump --meta $DCM 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
$tsComp = (& $Dump --meta $compLossless 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
$tsRt   = (& $Dump --meta $rtLossless 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
Write-Host "    Original:     $tsOrig"
Write-Host "    Compressed:   $tsComp"
Write-Host "    Round-trip:   $tsRt"

# ── 2. Near-lossless ──────────────────────────────────────────────────────────
Banner "2. Near-lossless JPEG-LS (max deviation = 3)"

$compLossy = Join-Path $TmpDir 'compressed_lossy.dcm'
$rtLossy   = Join-Path $TmpDir 'roundtrip_lossy.dcm'

Write-Host "  Compressing:  ABDOM_1.dcm -> compressed_lossy.dcm"
& $Cjpls -v -n 3 $DCM $compLossy
Write-Host ""
Write-Host "  Decompressing: compressed_lossy.dcm -> roundtrip_lossy.dcm"
& $Djpls -v $compLossy $rtLossy

Write-Host ""
$lossySize = FileSize $compLossy
Write-Host "  File sizes:"
Write-Host "    Original:            $origSize bytes"
Write-Host "    Lossy compressed:    $lossySize bytes"
Write-Host "    Lossless compressed: $compSize bytes"

# ── 3. Batch lossless compress ────────────────────────────────────────────────
Banner "3. Batch lossless compress all 5 ABDOM slices"

$batchDir = Join-Path $TmpDir 'batch'
New-Item -ItemType Directory -Path $batchDir | Out-Null

foreach ($f in Get-ChildItem (Join-Path $Files 'ABDOM_*.dcm')) {
    $outPath = Join-Path $batchDir $f.Name
    & $Cjpls $f.FullName $outPath
    $oSize = $f.Length
    $cSize = (Get-Item $outPath).Length
    if ($cSize -gt 0) {
        $ratio = [math]::Round($oSize / $cSize, 1)
    } else {
        $ratio = '?'
    }
    Write-Host "  $($f.Name):  $oSize -> $cSize bytes  (${ratio}:1)"
}

# ── 4. Batch decompress + verify ─────────────────────────────────────────────
Banner "4. Batch decompress -> verify metadata preserved"

$roundtripDir = Join-Path $TmpDir 'roundtrip'
New-Item -ItemType Directory -Path $roundtripDir | Out-Null

foreach ($f in Get-ChildItem (Join-Path $batchDir 'ABDOM_*.dcm')) {
    $outPath = Join-Path $roundtripDir $f.Name
    & $Djpls $f.FullName $outPath

    $origPatient = (& $Dump (Join-Path $Files $f.Name) 2>$null | Select-String '\(0010,0010\)' | Select-Object -First 1)
    $rtPatient   = (& $Dump $outPath 2>$null | Select-String '\(0010,0010\)' | Select-Object -First 1)
    if ($origPatient -and $rtPatient -and $origPatient.ToString() -eq $rtPatient.ToString()) {
        Write-Host "  $($f.Name):  metadata preserved" -ForegroundColor Green
    } else {
        Write-Host "  $($f.Name):  WARNING - metadata differs" -ForegroundColor Yellow
    }
}

# ── 5. Dump compressed file structure ─────────────────────────────────────────
Banner "5. Dump compressed file structure"

Write-Host "  Showing metadata header of JPEG-LS compressed file:"
& $Dump --meta $compLossless 2>$null | Select-Object -First 30 | ForEach-Object { Write-Host "  $_" }
Write-Host "  ..."

Write-Host ""
Write-Host "Done - all JPEG-LS demos complete" -ForegroundColor Green

} finally {
    # Cleanup temp directory
    if (Test-Path $TmpDir) { Remove-Item -Recurse -Force $TmpDir }
}
