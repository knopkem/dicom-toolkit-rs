# openjph-rs HTJ2K migration status

The earlier `irv97` parity blocker has been resolved and `dicom-toolkit-rs` now
uses `openjph-core` as the active HTJ2K backend inside
`crates/dicom-toolkit-jpeg2000`.

## What changed

- HTJ2K encode and decode now route through a dedicated OpenJPH bridge.
- Classic JPEG 2000 remains on the existing in-workspace backend.
- Downstream `dicom-toolkit-codec` and CLI tools continue using the same public
  APIs.
- HTJ2K regression coverage now uses real DICOM slices from
  `examples/testfiles/ABDOM_*.dcm` instead of synthetic fixture codestreams.

## Current notes

- The OpenJPH-backed HT path defaults to `4x1024` block dimensions when callers
  leave the old default block-size exponents unchanged. This preserves exact
  lossless roundtrips on the real 12-bit CT fixtures used by the regression
  suite.
- The OpenJPH HT decode branch is intentionally limited to DICOM-style raw
  codestream use. HTJP2 palette/alpha/channel-definition handling remains
  explicitly unsupported there.

## Recommended validation

```text
cargo test -p dicom-toolkit-jpeg2000 --test htj2k_conformance
cargo test -p dicom-toolkit-codec --test htj2k_registry
cargo test -p dicom-toolkit-tools --test jp2k_cli
```
