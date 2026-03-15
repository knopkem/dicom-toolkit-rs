# dicom-toolkit-rs

> ⚠️ **NOT FOR CLINICAL USE** — This software is not a certified medical device.
> It has not been validated for diagnostic or therapeutic use under any regulatory
> framework (FDA 510(k), CE marking, MDR, etc.). Use at your own risk.

A pure-Rust port of [DCMTK](https://dicom.offis.de/dcmtk.php.en) 3.7.0 — a comprehensive DICOM medical imaging toolkit. The port targets feature parity across the four core DCMTK tiers and applies idiomatic Rust patterns throughout.

This is an independent project, not affiliated with or endorsed by OFFIS e.V. See [NOTICE](NOTICE) for attribution details.

[![Tests](https://img.shields.io/badge/tests-428%20passing-brightgreen)](#status)

---

## Status

| Tier | Scope | Tests |
|------|-------|-------|
| 1 — Foundation | `dicom-toolkit-core`, `dicom-toolkit-dict` | 49 + 59 |
| 2 — Data model & I/O | `dicom-toolkit-data` | 129 + 31 integration |
| 3 — Networking | `dicom-toolkit-net` | 48 unit + 4 protocol E2E + 7 server E2E |
| 4 — Imaging & codecs | `dicom-toolkit-image`, `dicom-toolkit-codec` | 44 + 77 |
| **Total** | | **428 passing, 0 failed** |

---

## Crates

| Crate | Ports from DCMTK | Description |
|-------|-----------------|-------------|
| [`dicom-toolkit-core`](crates/dicom-toolkit-core) | `ofstd`, `oficonv`, `oflog` | Error types, UIDs, **full character set support** (ISO 2022 + single-byte + UTF-8), logging |
| [`dicom-toolkit-dict`](crates/dicom-toolkit-dict) | `dcmdata` (dict) | 90+ tag constants, all 34 VRs, 13 transfer syntaxes, SOP class UID registry |
| [`dicom-toolkit-data`](crates/dicom-toolkit-data) | `dcmdata` | DICOM data model, Part 10 file reader/writer, DICOM JSON (PS3.18), XML, deflate |
| [`dicom-toolkit-net`](crates/dicom-toolkit-net) | `dcmnet`, `dcmtls` | Async DICOM networking: PDU layer, association, C-ECHO/STORE/FIND/GET/MOVE, TLS |
| [`dicom-toolkit-image`](crates/dicom-toolkit-image) | `dcmimgle`, `dcmimage` | Pixel pipeline, Modality/VOI LUT, window/level, overlays, color models, PNG export |
| [`dicom-toolkit-codec`](crates/dicom-toolkit-codec) | `dcmjpeg`, `dcmjpls`, `dcmrle` | JPEG baseline, **pure-Rust JPEG-LS** (lossless & near-lossless, 2–16 bit), RLE PackBits, codec registry |
| [`dicom-toolkit-tools`](crates/dicom-toolkit-tools) | `dcmdump`, `echoscu`, etc. | CLI utilities: dump, network SCU/SCP, img2dcm, JPEG-LS compress/decompress (see below) |

---

## Requirements

- Rust **1.75** or later
- `cargo`

No C/C++ compiler or external native libraries are required — all dependencies are pure Rust or bundled.

---

## Build

```bash
# Build the whole workspace
cargo build --workspace

# Build CLI tools only
cargo build --bins

# Run all tests
cargo test --workspace
```

---

## Library Usage

### Reading a DICOM file

```rust
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;

let file = FileFormat::open("image.dcm")?;
let ds = file.dataset();

if let Some(name) = ds.get_string(tags::PATIENT_NAME) {
    println!("Patient: {name}");
}
let rows    = ds.get_u16(tags::ROWS).unwrap_or(0);
let columns = ds.get_u16(tags::COLUMNS).unwrap_or(0);
println!("Size: {columns}×{rows}");
```

### Creating and writing a DICOM file

```rust
use dicom_toolkit_data::{DataSet, FileFormat};
use dicom_toolkit_dict::{tags, transfer_syntaxes as ts};
use dicom_toolkit_core::uid::Uid;

let mut ds = DataSet::new();
ds.set_string(tags::PATIENT_NAME, "Doe^John")?;
ds.set_string(tags::PATIENT_ID,   "12345")?;
ds.set_u16(tags::ROWS,    512)?;
ds.set_u16(tags::COLUMNS, 512)?;

let sop_uid = Uid::generate("2.25")?.to_string();
let file = FileFormat::new(ds, &ts::EXPLICIT_VR_LITTLE_ENDIAN);
file.save("output.dcm")?;
```

### DICOM JSON (PS3.18)

```rust
use dicom_toolkit_data::{FileFormat, json::DicomJson};

let file  = FileFormat::open("image.dcm")?;
let json  = DicomJson::encode(file.dataset())?;
println!("{json}");

let ds2 = DicomJson::decode(&json)?;
```

### Image rendering

```rust
use dicom_toolkit_image::DicomImage;

let image = DicomImage::from_file("ct.dcm")?;
let frame = image.render_frame(0, None)?;  // applies Modality + VOI LUT
frame.save_png("ct_frame0.png")?;
```

### Codec registry

```rust
use dicom_toolkit_codec::registry::GLOBAL_REGISTRY;
use dicom_toolkit_dict::transfer_syntaxes as ts;

let codec = GLOBAL_REGISTRY.get(ts::RLE_LOSSLESS.uid)?;
let raw   = codec.decode(&compressed_bytes, width, height, samples)?;
```

### Async networking (C-ECHO)

```rust
use dicom_toolkit_net::{association::Association, config::AssociationConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AssociationConfig::default();
    let mut assoc = Association::request("pacs.example.com:11112", cfg).await?;
    assoc.c_echo().await?;
    assoc.release().await?;
    Ok(())
}
```

---

## Character Set Support

DICOM strings use **Specific Character Set** `(0008,0005)` to indicate encoding. dicom-toolkit-rs handles this transparently — the reader decodes to UTF-8 on input, the writer re-encodes on output.

| DICOM Term | Encoding | Notes |
|------------|----------|-------|
| *(empty)* / `ISO_IR 6` | ASCII | Default |
| `ISO_IR 100` | Latin-1 (Windows-1252 superset) | Western European |
| `ISO_IR 101` | Latin-2 (ISO 8859-2) | Central European |
| `ISO_IR 109` | Latin-3 (ISO 8859-3) | South European |
| `ISO_IR 110` | Latin-4 (ISO 8859-4) | North European |
| `ISO_IR 144` | Cyrillic (ISO 8859-5) | Russian, etc. |
| `ISO_IR 127` | Arabic (ISO 8859-6) | |
| `ISO_IR 126` | Greek (ISO 8859-7) | |
| `ISO_IR 138` | Hebrew (ISO 8859-8) | |
| `ISO_IR 148` | Latin-5 (ISO 8859-9) | Turkish |
| `ISO_IR 166` | Thai (TIS 620) | |
| `ISO_IR 203` | Latin-9 (ISO 8859-15) | Adds €, Œ, Ÿ |
| `ISO_IR 192` | UTF-8 | Recommended for new data |
| `GB18030` | Chinese (GB 18030) | |
| `GBK` | Chinese (GBK) | |

**ISO 2022 extensions** (multi-charset via escape sequences) are fully supported for Japanese (JIS X 0201/0208/0212), Korean (KS X 1001), and Simplified Chinese (GB 2312).

All string data is stored internally as Rust `String` (UTF-8), with encoding/decoding occurring at I/O time. Round-trip fidelity is tested for all supported charsets.

---

## CLI Tools

All tools are built as part of `cargo build --bins` and placed in `target/debug/` (or `target/release/` with `--release`).

### `dcmdump` — print DICOM file contents

```
dcmdump [OPTIONS] <FILE>...

Options:
  -M, --meta      Also print File Meta Information header
  -n, --no-limit  Do not limit string value output length
      --json      Output as DICOM JSON
      --xml       Output as DICOM XML
  -v, --verbose   Verbose output
```

**Examples**

```bash
# Human-readable dump
dcmdump image.dcm

# Include file meta group (0002,xxxx)
dcmdump --meta image.dcm

# Export as DICOM JSON
dcmdump --json image.dcm > image.json

# Dump multiple files
dcmdump *.dcm
```

---

### `echoscu` — send C-ECHO verification

```
echoscu [OPTIONS] <HOST> <PORT>

Options:
  -a, --aetitle <AE>    Calling AE title [default: ECHOSCU]
  -c, --called-ae <AE>  Called AE title [default: ANY-SCP]
  -r, --repeat <N>      Number of C-ECHO requests [default: 1]
  -v, --verbose
```

**Examples**

```bash
# Verify connectivity to a PACS
echoscu pacs.example.com 11112

# Custom AE titles, send 3 pings
echoscu -a MY_SCU -c ORTHANC -r 3 localhost 4242
```

---

### `storescu` — send DICOM files (C-STORE)

```
storescu [OPTIONS] <HOST> <PORT> <FILE>...

Options:
  -a, --aetitle <AE>    Calling AE title [default: STORESCU]
  -c, --called-ae <AE>  Called AE title [default: ANY-SCP]
  -v, --verbose
```

**Examples**

```bash
# Send one file
storescu pacs.example.com 11112 image.dcm

# Send a whole study directory
storescu -a MY_SCU -c ORTHANC localhost 4242 study/*.dcm
```

---

### `storescp` — receive DICOM files (Storage SCP)

```
storescp [OPTIONS] <PORT>

Options:
  -a, --aetitle <AE>      Called AE title [default: STORESCP]
  -d, --output-dir <DIR>  Directory to save received files [default: .]
  -v, --verbose
```

**Examples**

```bash
# Listen on port 11112, save to /tmp/incoming
storescp -d /tmp/incoming 11112

# Run in background (receives from storescu above)
storescp -v 4242 &
storescu localhost 4242 *.dcm
```

---

### `findscu` — query with C-FIND

```
findscu [OPTIONS] <HOST> <PORT>

Options:
  -a, --aetitle <AE>     Calling AE title [default: FINDSCU]
  -c, --called-ae <AE>   Called AE title [default: ANY-SCP]
  -k, --key <TAG=VALUE>  Query attribute (repeatable), e.g. "0010,0010=Smith*"
  -L, --level <LEVEL>    PATIENT | STUDY | SERIES | IMAGE [default: STUDY]
  -v, --verbose
```

**Examples**

```bash
# Find all studies for patients whose name starts with "Smith"
findscu -k "0010,0010=Smith*" pacs.example.com 11112

# Find a specific study by date, at patient level
findscu -L PATIENT -k "0010,0010=" -k "0008,0020=20240101" localhost 4242
```

---

### `img2dcm` — convert PNG to DICOM

```
img2dcm [OPTIONS] <INPUT> [OUTPUT]

Arguments:
  <INPUT>   Input PNG file
  [OUTPUT]  Output DICOM file [default: <input>.dcm]

Options:
  -p, --patient-name <NAME>          [default: Anonymous]
  -P, --patient-id <ID>
  -s, --study-description <TEXT>
  -S, --series-description <TEXT>
      --sop-class <UID>              Override SOP Class UID
      --sop-instance <UID>           Override SOP Instance UID
  -v, --verbose
```

**Examples**

```bash
# Wrap a PNG as a Secondary Capture DICOM file
img2dcm photo.png

# With patient metadata
img2dcm -p "Doe^John" -P "12345" -s "Chest X-Ray" chest.png chest.dcm
```

### `dcmcjpls` — compress DICOM to JPEG-LS

```
dcmcjpls [OPTIONS] <INPUT> <OUTPUT>

Arguments:
  <INPUT>   Input DICOM file (uncompressed)
  <OUTPUT>  Output DICOM file (JPEG-LS compressed)

Options:
  -n, --max-deviation <NEAR>   Max pixel error; 0 = lossless [default: 0]
  -l, --encode-lossless         Force lossless mode (NEAR=0)
      --encode-nearlossless     Near-lossless mode (default NEAR=2)
  -v, --verbose
```

**Examples**

```bash
# Lossless JPEG-LS compression (default)
dcmcjpls image.dcm image_jls.dcm

# Near-lossless with max deviation of 3
dcmcjpls -n 3 -v image.dcm image_lossy.dcm

# Batch compress all files in a directory
for f in study/*.dcm; do dcmcjpls "$f" "compressed/$(basename $f)"; done
```

### `dcmdjpls` — decompress JPEG-LS DICOM

```
dcmdjpls [OPTIONS] <INPUT> <OUTPUT>

Arguments:
  <INPUT>   Input DICOM file (JPEG-LS compressed)
  <OUTPUT>  Output DICOM file (Explicit VR Little Endian)

Options:
  -v, --verbose
```

**Examples**

```bash
# Decompress a JPEG-LS file
dcmdjpls image_jls.dcm image_raw.dcm

# Verbose — show image parameters and compression ratio
dcmdjpls -v image_jls.dcm image_raw.dcm

# Round-trip: compress then decompress
dcmcjpls -v image.dcm /tmp/compressed.dcm
dcmdjpls -v /tmp/compressed.dcm /tmp/roundtrip.dcm
```

---

## Example Scripts

Ready-to-run scripts live in [`examples/scripts/`](examples/scripts/) and use the five ABDOM CT slices in [`examples/testfiles/`](examples/testfiles/).

| Script | What it demonstrates |
|--------|---------------------|
| `01_dump` | All `dcmdump` output modes: plain, `--meta`, `--no-limit`, `--json`, `--xml`, multi-file batch |
| `02_network` | Start `storescp` → C-ECHO verify with `echoscu` → send all 5 slices with `storescu` → inspect received files |
| `03_query` | `findscu` command patterns; set `RUN_LIVE=1` / `$env:RUN_LIVE='1'` to query a real PACS |
| `04_img2dcm` | Generate a PNG with Python stdlib → wrap as Secondary Capture → dump + JSON export |
| `05_jpegls` | JPEG-LS lossless & near-lossless round-trip, batch compress/decompress, metadata verification |
| `demo` | Master script — runs all five above in order |

Two equivalent versions are provided for each script:

### Linux / macOS (bash)

```bash
# Run the full demo
bash examples/scripts/demo.sh

# Or run individual scripts
bash examples/scripts/01_dump.sh
bash examples/scripts/02_network.sh
bash examples/scripts/04_img2dcm.sh
bash examples/scripts/05_jpegls.sh
```

### Windows (PowerShell)

Requires PowerShell 7+ (`pwsh`) or Windows PowerShell 5.1.
On first run you may need to allow local scripts:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

```powershell
# Run the full demo (non-interactive)
pwsh -File examples/scripts/demo.ps1

# Run with a pause between sections
pwsh -File examples/scripts/demo.ps1 -Pause

# Or run individual scripts
pwsh -File examples/scripts/01_dump.ps1
pwsh -File examples/scripts/02_network.ps1
pwsh -File examples/scripts/04_img2dcm.ps1
```

The PowerShell scripts also work on macOS and Linux with [PowerShell Core](https://github.com/PowerShell/PowerShell).

> **Note:** `03_query` shows command-line patterns but does not execute live queries by default. The `storescp` binary now uses the `DicomServer` framework. C-FIND, C-GET, and C-MOVE SCP handling is available in-process via the library; see [DicomServer](#dicomserver) below. Set `RUN_LIVE=1` to use an external Orthanc instance with the query scripts.

---

## Architecture

The port maps DCMTK's deep C++ class hierarchy to idiomatic Rust:

| DCMTK C++ | Rust equivalent |
|-----------|----------------|
| `OFCondition` / exception | `thiserror` `DcmError` enum + `DcmResult<T>` |
| `DcmObject` → `DcmElement` → `DcmByteString` … | `Value` enum (21 variants) + `Element` struct |
| `DcmDataset` (`std::map`) | `DataSet` backed by `IndexMap<Tag, Element>` |
| `DcmFileFormat` | `FileFormat` struct |
| `OFString` / `OFList` | `String` / `Vec<T>` |
| `oflog` / log4cplus | `tracing` + `tracing-subscriber` |
| `oficonv` | `encoding_rs` |
| `DcmTransportLayer` (OpenSSL) | `rustls` + `tokio-rustls` |
| Blocking socket I/O | `tokio` async I/O |
| CharLS (C++ JPEG-LS) | Pure Rust JPEG-LS codec (ISO 14495-1) |

---

## JPEG-LS Codec

`dicom-toolkit-codec` includes a **pure-Rust JPEG-LS codec** ported from the CharLS algorithm bundled with DCMTK. No C/C++ dependencies — works on any Rust target including WASM.

**Supported features:**
- Lossless mode (DICOM TS `1.2.840.10008.1.2.4.80`)
- Near-lossless/lossy mode (DICOM TS `1.2.840.10008.1.2.4.81`)
- 2–16 bit depths (DICOM commonly uses 8, 12, 16)
- Grayscale and multi-component images (1–4 components)
- Interleave modes: ILV_NONE, ILV_LINE
- HP color transforms (APP8 marker)

**Architecture** (10 modules, ~1,200 LOC):

| Module | Purpose |
|--------|---------|
| `params.rs` | Parameters, threshold computation (ISO §C.2.4.1.1) |
| `sample.rs` | Sample trait for `u8`/`u16` bit-depth dispatch |
| `bitstream.rs` | BitReader/BitWriter with FF-bitstuffing |
| `context.rs` | Context statistics (A, B, C, N) + run-mode context |
| `golomb.rs` | Golomb-Rice coding (encode/decode mapped errors) |
| `prediction.rs` | Median-edge predictor, gradient quantization |
| `marker.rs` | JPEG-LS marker parsing/writing (SOF-55, SOS, LSE) |
| `scan.rs` | Core scan encoder/decoder (line-by-line processing) |
| `decoder.rs` | Top-level decoder: markers → scan decoder → pixels |
| `encoder.rs` | Top-level encoder: pixels → scan encoder → bitstream |

---

## DicomServer

`dicom-toolkit-net` now ships a generic `DicomServer` for building full PACS SCPs.  
It manages concurrent TCP associations, request routing, and graceful shutdown.  
You plug in your own logic via provider traits; the library handles all DICOM protocol mechanics.

### Quick start

```rust
use dicom_toolkit_net::server::{DicomServer, FileStoreProvider};

#[tokio::main]
async fn main() {
    let server = DicomServer::builder()
        .ae_title("MYPACS")
        .port(4242)
        .store_provider(FileStoreProvider::new("/data/dicom"))
        .build()
        .await
        .expect("bind port");

    server.run().await.expect("server error");
}
```

### Provider traits

Implement one or more of these traits to add your own business logic:

| Trait | Service | Callback |
|-------|---------|----------|
| `StoreServiceProvider` | C-STORE | `async fn on_store(&self, StoreEvent) -> StoreResult` |
| `FindServiceProvider` | C-FIND | `async fn on_find(&self, FindEvent) -> Vec<DataSet>` |
| `GetServiceProvider` | C-GET | `async fn on_get(&self, GetEvent) -> Vec<RetrieveItem>` |
| `MoveServiceProvider` | C-MOVE | `async fn on_move(&self, MoveEvent) -> Vec<RetrieveItem>` |

C-ECHO is always handled automatically without a trait.

### DicomServerBuilder options

```rust
DicomServer::builder()
    .ae_title("MYPACS")           // AE title (default: "DICOMRS")
    .port(4242)                    // TCP port (default: 4242)
    .max_associations(100)         // Max concurrent associations
    .store_provider(my_store)      // C-STORE SCP
    .find_provider(my_query)       // C-FIND SCP
    .get_provider(my_get)          // C-GET SCP
    .move_provider(my_move)        // C-MOVE SCP
    .move_destination_lookup(      // AE→host:port for C-MOVE sub-associations
        StaticDestinationLookup::new(vec![
            ("STORESCP".into(), "10.0.0.1:4242".into()),
        ])
    )
    .build()
    .await?
```

### Graceful shutdown

```rust
let token = server.cancellation_token();
// In another task or signal handler:
token.cancel(); // server.run() returns cleanly
```

### Built-in provider: `FileStoreProvider`

Ships ready to use — receives DICOM instances and saves them as `.dcm` files:

```rust
.store_provider(FileStoreProvider::new("/tmp/incoming"))
```

---

## Known Limitations

- **JPEG 2000**: not yet implemented.
- **Worklist / MPPS**: not yet ported.
- **JPEG-LS ILV_SAMPLE**: pixel-interleaved multi-component mode not yet supported (ILV_NONE and ILV_LINE work).

---

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.

See [NOTICE](NOTICE) for attribution of algorithmic references (DCMTK, CharLS).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Security

See [SECURITY.md](SECURITY.md) for vulnerability reporting and DICOM security guidance.
