# 04_img2dcm.ps1 — img2dcm showcase
#
# Creates a test PNG with Python (stdlib only, no extra packages),
# then wraps it in a DICOM Secondary Capture file and dumps the result.
#
# Usage: pwsh -File 04_img2dcm.ps1
[CmdletBinding()] param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = Resolve-Path (Join-Path $ScriptDir '..\..')
$Ext       = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }
$Img2dcm   = Join-Path $Root "target\debug\img2dcm$Ext"
$Dcmdump   = Join-Path $Root "target\debug\dcmdump$Ext"

foreach ($b in @($Img2dcm, $Dcmdump)) {
    if (-not (Test-Path $b)) { Write-Error "Binary not found: $b`nRun: cargo build --bins" }
}

# Prefer 'python3', fall back to 'python' (Windows store alias)
$Python = 'python3'
if (-not (Get-Command $Python -ErrorAction SilentlyContinue)) { $Python = 'python' }
if (-not (Get-Command $Python -ErrorAction SilentlyContinue)) {
    Write-Error "Python not found. Install Python 3 from https://www.python.org/"
}

$WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $WorkDir | Out-Null

function Banner($text) {
    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Cyan
    Write-Host " $text" -ForegroundColor Cyan
    Write-Host ("=" * 60) -ForegroundColor Cyan
}

try {
    # ── Step 1: create PNG ──────────────────────────────────────────
    Banner "Step 1 — Create a test PNG with Python"

    $PngPath = Join-Path $WorkDir 'test.png'

    # Inline Python: writes a 128×128 RGB gradient PNG using stdlib only
    $PyScript = @'
import sys, struct, zlib

def write_png(path, width, height):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    rows = []
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            row += bytearray([int(x/width*255), int(y/height*255), 128])
        rows.append(bytes(row))
    raw = zlib.compress(b"".join(rows))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", raw))
        f.write(chunk(b"IEND", b""))
    print(f"  Written {width}x{height} RGB PNG -> {path}")

write_png(sys.argv[1], 128, 128)
'@
    & $Python -c $PyScript $PngPath

    # ── Step 2: convert to DICOM ────────────────────────────────────
    Banner "Step 2 — Wrap PNG in a DICOM Secondary Capture file"

    $OutDcm = Join-Path $WorkDir 'test.dcm'
    & $Img2dcm `
        -p 'Demo^Patient' `
        -P 'DEMO_001' `
        -s 'Test Study' `
        -S 'Secondary Capture' `
        -v `
        $PngPath $OutDcm

    # ── Step 3: dump the result ────────────────────────────────────
    Banner "Step 3 — Dump the resulting DICOM file"
    & $Dcmdump --meta $OutDcm

    # ── Step 4: export as DICOM JSON ───────────────────────────────
    Banner "Step 4 — Export as DICOM JSON"
    & $Dcmdump --json $OutDcm | Select-Object -First 50

    Banner "Done"
    Write-Host "  Secondary Capture DICOM: $OutDcm"
    Write-Host "  (temp dir will be removed on exit)"

} finally {
    Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
}
