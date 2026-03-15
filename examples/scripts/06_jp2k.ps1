# 06_jp2k.ps1  -  JPEG 2000 compression / decompression showcase
#
# Demonstrates dcmcjp2k and dcmdjp2k with the ABDOM CT test slices:
#   1. Lossless compress → decompress round-trip
#   2. Lossy compress → decompress smoke test
#   3. Batch lossless compress all test files
#   4. Batch decompress → verify metadata preserved
#
# Usage: pwsh -File 06_jp2k.ps1
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if (($env:OS -eq 'Windows_NT') -or ((Test-Path variable:IsWindows) -and $IsWindows)) { '.exe' } else { '' }
$Cjp2k     = Join-Path $Root "target\debug\dcmcjp2k$Ext"
$Djp2k     = Join-Path $Root "target\debug\dcmdjp2k$Ext"
$Dump      = Join-Path $Root "target\debug\dcmdump$Ext"
$Files     = Join-Path $ScriptDir '..\testfiles'

foreach ($b in @($Cjp2k, $Djp2k, $Dump)) {
    if (-not (Test-Path $b)) {
        Write-Error "Binary not found: $b`nRun: cargo build --bins"
    }
}

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "dcmtk-jp2k-demo"
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
Banner "1. Lossless JPEG 2000 round-trip"

$compLossless = Join-Path $TmpDir 'compressed_lossless_j2k.dcm'
$rtLossless   = Join-Path $TmpDir 'roundtrip_lossless_j2k.dcm'

Write-Host "  Compressing:  ABDOM_1.dcm -> compressed_lossless_j2k.dcm"
& $Cjp2k -v $DCM $compLossless
Write-Host ""
Write-Host "  Decompressing: compressed_lossless_j2k.dcm -> roundtrip_lossless_j2k.dcm"
& $Djp2k -v $compLossless $rtLossless

Write-Host ""
Write-Host "  File sizes:"
$origSize = FileSize $DCM
$compSize = FileSize $compLossless
$rtSize   = FileSize $rtLossless
Write-Host "    Original:     $origSize bytes"
Write-Host "    Compressed:   $compSize bytes"
Write-Host "    Round-trip:   $rtSize bytes"

Write-Host ""
Write-Host "  Transfer syntax of each file:"
$tsOrig = (& $Dump --meta $DCM 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
$tsComp = (& $Dump --meta $compLossless 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
$tsRt   = (& $Dump --meta $rtLossless 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
Write-Host "    Original:     $tsOrig"
Write-Host "    Compressed:   $tsComp"
Write-Host "    Round-trip:   $tsRt"

# ── 2. Lossy smoke test ───────────────────────────────────────────────────────
Banner "2. Lossy JPEG 2000 smoke test"

$compLossy = Join-Path $TmpDir 'compressed_lossy_j2k.dcm'
$rtLossy   = Join-Path $TmpDir 'roundtrip_lossy_j2k.dcm'

Write-Host "  Compressing:  ABDOM_1.dcm -> compressed_lossy_j2k.dcm"
& $Cjp2k --encode-lossy -v $DCM $compLossy
Write-Host ""
Write-Host "  Decompressing: compressed_lossy_j2k.dcm -> roundtrip_lossy_j2k.dcm"
& $Djp2k -v $compLossy $rtLossy

Write-Host ""
$lossySize   = FileSize $compLossy
$lossyRtSize = FileSize $rtLossy
Write-Host "  File sizes:"
Write-Host "    Original:            $origSize bytes"
Write-Host "    Lossless compressed: $compSize bytes"
Write-Host "    Lossy compressed:    $lossySize bytes"
Write-Host "    Lossy round-trip:    $lossyRtSize bytes"
Write-Host ""
Write-Host "  Lossy indicators:"
$tsLossy = (& $Dump --meta $compLossy 2>$null | Select-String '\(0002,0010\)' | Select-Object -First 1).ToString().Trim()
$lossyFlag = (& $Dump $compLossy 2>$null | Select-String '\(0028,2110\)' | Select-Object -First 1).ToString().Trim()
Write-Host "    Transfer Syntax:    $tsLossy"
Write-Host "    Lossy flag:         $lossyFlag"

# ── 3. Batch lossless compress ────────────────────────────────────────────────
Banner "3. Batch lossless compress all 5 ABDOM slices"

$batchDir = Join-Path $TmpDir 'batch'
New-Item -ItemType Directory -Path $batchDir | Out-Null

foreach ($f in Get-ChildItem (Join-Path $Files 'ABDOM_*.dcm')) {
    $outPath = Join-Path $batchDir $f.Name
    & $Cjp2k $f.FullName $outPath
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
    & $Djp2k $f.FullName $outPath

    $origPatient = (& $Dump (Join-Path $Files $f.Name) 2>$null | Select-String '\(0010,0010\)' | Select-Object -First 1)
    $rtPatient   = (& $Dump $outPath 2>$null | Select-String '\(0010,0010\)' | Select-Object -First 1)
    if ($origPatient -and $rtPatient -and $origPatient.ToString() -eq $rtPatient.ToString()) {
        Write-Host "  $($f.Name):  metadata preserved" -ForegroundColor Green
    } else {
        Write-Host "  $($f.Name):  WARNING - metadata differs" -ForegroundColor Yellow
    }
}

# ── 5. Dump compressed file structure ────────────────────────────────────────
Banner "5. Dump compressed file structure"

Write-Host "  Showing metadata header of JPEG 2000 compressed file:"
& $Dump --meta $compLossless 2>$null | Select-Object -First 30 | ForEach-Object { Write-Host "  $_" }
Write-Host "  ..."

Write-Host ""
Write-Host "Done - all JPEG 2000 demos complete" -ForegroundColor Green

} finally {
    if (Test-Path $TmpDir) { Remove-Item -Recurse -Force $TmpDir }
}
