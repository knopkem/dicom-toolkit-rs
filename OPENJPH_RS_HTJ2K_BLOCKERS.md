# openjph-rs HTJ2K migration blockers

## Current blocker

The current remaining blocker for switching `dicom-toolkit-rs` HTJ2K over to
`openjph-rs` is the broad irreversible 9/7 parity gap (`irv97`).

## Current evidence

Running:

```text
cargo test -p openjph-core
```

currently fails in:

```text
tests/integration_encode_decode.rs
```

with:

```text
38 passed; 50 failed
```

Running:

```text
cargo test -p openjph-core irv97
```

reproduces the same blocker bucket directly.

Representative failing tests:

- `dec_irv97_64x64_rgb`
- `dec_irv97_gray_tiles`
- `enc_irv97_decomp_0`
- `enc_irv97_16bit_gray`
- `enc_irv97_tiles_33x33_d5`

Representative current MSE values:

- many 8-bit RGB failures: about `5464.9844`
- 16-bit failures: about `1_039_004_000` to `1_053_217_000`

## Recommendation for `dicom-toolkit-rs`

Do **not** switch the active HTJ2K backend to `openjph-rs` yet.

The next required step is a dedicated `irv97` parity pass against local C++
OpenJPH. Only after that bucket is green should the backend swap be retried and
the downstream HTJ2K integration tests be rerun.
