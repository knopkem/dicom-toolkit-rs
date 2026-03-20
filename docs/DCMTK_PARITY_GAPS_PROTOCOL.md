# DCMTK parity gaps protocol

This document captures the **remaining larger parity gaps** identified while comparing
`dcmtk-rs` against the local `dcmtk-3.7.0` checkout.

It is intended as a **later-pass execution protocol**, not as a claim that every item
below is already fully proven or ready to patch. Each item is labeled by confidence:

- **Confirmed gap**: manually verified against both Rust and DCMTK code
- **Candidate gap**: reported by the audit and plausible, but still needs targeted
  repro/verification before code changes
- **Rejected/outdated**: audit noise or items already fixed upstream in Rust

This protocol also records what was already fixed during the current audit pass so
those issues do not get rediscovered.

---

## Audit method used

The parity pass compared the four core crates against the corresponding DCMTK areas:

- `dicom-toolkit-core` ↔ `ofstd` + `dcmdata` character/UID foundations
- `dicom-toolkit-data` ↔ `dcmdata`
- `dicom-toolkit-image` ↔ `dcmimgle` / `dcmimage`
- `dicom-toolkit-net` ↔ `dcmnet`

Important note: the agent-generated audit reports were treated as **candidate
findings only**. Several claims turned out to be stale or incorrect once manually
checked, so future work should keep the same standard: **verify before patching**.

---

## Already fixed during this audit pass

These items are no longer open:

### 1. ISO 2022 Japanese segment decoding

- **Status:** fixed
- **Rust:** `crates/dicom-toolkit-core/src/charset.rs`
- **DCMTK:** `dcmdata/libsrc/dcspchrs.cc`

Issue:

- Rust previously stripped ISO 2022 escape sequences and then decoded the remaining
  raw JIS bytes with `ISO_2022_JP`, which turns escaped Kanji segments into ASCII
  punctuation instead of Japanese text.

What changed:

- The decoder now tracks the active defined term
- Mirrors DCMTK-style multi-byte scan state
- Wraps `ISO 2022 IR 87` / `ISO 2022 IR 159` segments before decoding
- Adds regression coverage for escaped Japanese text

### 2. `Uid::from_static()` placeholder bug

- **Status:** fixed
- **Rust:** `crates/dicom-toolkit-core/src/uid.rs`

Issue:

- `Uid::from_static()` returned an empty string placeholder instead of the provided UID.

What changed:

- The function now preserves the supplied static UID string
- Regression test added

### 3. Invalid window width acceptance

- **Status:** fixed
- **Rust:** `crates/dicom-toolkit-image/src/dicom_image.rs`
- **DCMTK:** `dcmimgle/libsrc/dimoimg.cc`

Issue:

- Rust accepted `window_width < 1.0`
- DCMTK `DiMonoImage::setWindow()` treats that as invalid

What changed:

- `DicomImage::set_window()` now rejects widths below `1.0`
- `render_frame_u8()` now propagates the error
- Regression coverage added

---

## Confirmed unresolved parity gaps

These are the best next-pass candidates because they were either manually confirmed
or directly observed while fixing the confirmed issues above.

### A. `dicom-toolkit-core`: ISO 2022 parity is still incomplete

- **Status:** confirmed gap
- **Priority:** high

#### A.1 Escape-aware encoding for multi-valued charsets is still missing

- **Rust:** `crates/dicom-toolkit-core/src/charset.rs:270-281`
- **Related decode path:** `crates/dicom-toolkit-core/src/charset.rs:297-390`
- **DCMTK reference:** `dcmdata/libsrc/dcspchrs.cc`

Current state:

- `DicomCharsetDecoder::encode()` still uses `self.default_encoding` only
- It does not emit ISO 2022 escape sequences or switch encodings within a string
- This means decode parity improved, but full **encode parity** for multi-valued
  `(0008,0005)` strings is still not there

Why it matters:

- Any future writer path that wants to preserve or emit real ISO 2022 extended
  strings will still diverge from DCMTK
- Today this is partly masked because many workflows stay in UTF-8 or single-byte
  charsets

Recommended later pass:

1. Decide whether the Rust API should support full ISO 2022 emission or document
   decode-only support for multi-valued legacy charsets
2. Add fixture-backed roundtrip tests for Japanese/Korean/Chinese text with
   real escape sequences

#### A.2 Delimiter/reset behavior is still simplified compared to DCMTK

- **Rust:** `crates/dicom-toolkit-core/src/charset.rs:340-358`
- **DCMTK:** `dcmdata/libsrc/dcspchrs.cc:668-701`, `840-857`

Current state:

- Rust resets on `HT`, `LF`, `FF`, `CR`
- DCMTK additionally allows caller-controlled delimiters and special handling for
  `PN` component groups

Why it matters:

- Person Name handling with mixed charsets is more nuanced in DCMTK
- Rust currently lacks the VR-aware delimiter model used by `DcmSpecificCharacterSet`

Recommended later pass:

1. Thread VR or delimiter context into the charset decode path
2. Add PN-specific compatibility tests with `=` separated component groups

#### A.3 Charset mappings still rely on approximations/subsets

- **Rust:** `crates/dicom-toolkit-core/src/charset.rs:49-59`
- **DCMTK:** `dcmdata/libsrc/dcspchrs.cc:427-445`

Current state:

- `ISO 2022 IR 159` still maps through `ISO_2022_JP`
- `ISO 2022 IR 58` maps to `GB18030`
- `ISO 2022 IR 149` maps to `EUC_KR`

Why it matters:

- Some mappings are pragmatic approximations because `encoding_rs` does not expose
  DCMTK/iconv-equivalent converters for every legacy DICOM defined term
- The decoder is now materially better for Japanese `IR 87`, but the broader
  compatibility surface still needs fixture-driven verification

Recommended later pass:

1. Add real DICOM samples for `IR 149`, `IR 58`, and (if available) `IR 159`
2. Decide per-term whether to:
   - keep the approximation,
   - wrap/adapt bytes,
   - or explicitly report unsupported conversion

---

### B. `dicom-toolkit-image`: overlays are extracted but not part of `DicomImage`

- **Status:** confirmed gap
- **Priority:** high

- **Rust image state:** `crates/dicom-toolkit-image/src/dicom_image.rs:37-69`
- **Rust render behavior:** `crates/dicom-toolkit-image/src/render.rs:35-39`
- **Rust overlay helper:** `crates/dicom-toolkit-image/src/overlay.rs`
- **DCMTK reference:** `dcmimgle/libsrc/diovlimg.cc`

Current state:

- Overlay extraction exists as a separate helper
- `DicomImage` does **not** retain overlay planes
- `render_frame_u8(... burn_in_overlays = true)` explicitly returns an error

Why it matters:

- This is a meaningful functional gap vs DCMTK image output behavior
- It blocks standards-friendly rendered output for later DICOMweb work
- It pushes overlay compositing into callers instead of keeping it in the image layer

Recommended later pass:

1. Decide whether overlays belong in `DicomImage` itself or in a separate render
   composition pipeline
2. If kept in `DicomImage`, store extracted overlay planes at construction time
3. Add grayscale and RGB burn-in tests

---

### C. `dicom-toolkit-image`: `YBR_FULL_422` handling is only pixel-interleaved

- **Status:** confirmed gap
- **Priority:** medium

- **Rust dispatch:** `crates/dicom-toolkit-image/src/dicom_image.rs:333-335`
- **Rust converter:** `crates/dicom-toolkit-image/src/color/ycbcr.rs:48-79`
- **DCMTK reference:** `dcmimage/include/dcmtk/dcmimage/diyf2pxt.h`,
  `dcmimage/include/dcmtk/dcmimage/diyp2pxt.h`

Current state:

- The Rust converter assumes `[Cb, Y0, Cr, Y1, ...]`
- `planar_config` is ignored in the `YBR_FULL_422` branch

Why it matters:

- Plane-interleaved 4:2:2 input will be mishandled
- This is lower-frequency than MONOCHROME2/RGB, but it is a real color pipeline gap

Recommended later pass:

1. Build or import a plane-interleaved `YBR_FULL_422` fixture
2. Add dispatch based on `planar_config`
3. Verify output against DCMTK-rendered reference pixels

---

### D. `dicom-toolkit-image`: modality LUT support is linear-rescale only

- **Status:** confirmed gap
- **Priority:** medium

- **Rust:** `crates/dicom-toolkit-image/src/lut.rs:7-58`
- **DCMTK reference:** `dcmimgle/include/dcmtk/dcmimgle/dimomod.h`,
  `dcmimgle/libsrc/dimomod.cc`

Current state:

- Rust supports slope/intercept rescale only
- It does not support explicit modality LUT tables

Why it matters:

- Many common studies are fine with linear rescale only
- Some modalities and special workflows rely on explicit LUT tables

Recommended later pass:

1. Decide whether explicit modality LUT tables are in scope for the crate
2. If yes, add dataset extraction + LUT application + regression fixtures

---

### E. `dicom-toolkit-image`: auto-window semantics still differ from DCMTK

- **Status:** confirmed gap
- **Priority:** medium/low

- **Rust:** `crates/dicom-toolkit-image/src/dicom_image.rs:239-252`,
  `363-390`
- **DCMTK reference:** `dcmimgle/libsrc/dimoimg.cc:1046-1076`

Current state:

- Rust computes one window across **all frames**
- DCMTK exposes per-image/per-operation window behaviors, including ROI and
  histogram helpers

Why it matters:

- Not always a bug; sometimes the Rust behavior is acceptable or even preferable
- Still a parity difference worth documenting because it changes multi-frame UX

Recommended later pass:

1. Decide whether to preserve Rust’s global default and add opt-in per-frame APIs,
   or move closer to DCMTK behavior
2. Add explicit tests for multi-frame window selection semantics

---

## Candidate gaps that still need targeted verification

These items came out of the audit and are plausible, but they should **not** be
patched blindly. Each needs a focused repro or fixture first.

### F. `dicom-toolkit-data`: JSON parity needs a fresh re-audit

- **Status:** candidate gap
- **Priority:** medium/high

Why this section is cautious:

- The older data-layer audit artifacts include at least one confirmed false positive
  (`AT` VR missing), and some notes predate recent JSON/BulkDataURI work

Most plausible remaining areas:

1. **JSON numeric VR completeness**
   - Re-verify `from_json()` handling for numeric VRs against current code
   - Especially `IS`, `DS`, and binary/numeric VRs that may appear as numbers vs strings

2. **Undefined-length `UN` (CP 246)**
   - Re-verify current behavior against DCMTK with a real fixture

3. **DCMTK JSON interoperability matrix**
   - Generate current JSON from DCMTK 3.7.0 for representative datasets
   - Roundtrip through Rust
   - Capture any real mismatches instead of relying on stale report text

Why it matters:

- Data-layer parity bugs are high-impact if they are real
- But the false-positive rate was high enough that this area should start with
  fixtures, not code edits

Recommended later pass:

1. Build a fixture corpus from current DCMTK JSON output
2. Re-run a narrow, evidence-backed audit
3. Only then patch deserialization gaps

---

### G. `dicom-toolkit-net`: DIMSE send/validation semantics need deeper review

- **Status:** candidate gap
- **Priority:** medium

Why this section is cautious:

- The net audit also produced at least one confirmed false positive
  (sub-operation counters were already decoded)

Most plausible remaining areas:

1. **Transfer-syntax validation before sending dataset bytes**
   - Verify whether `send_dimse_data()` can be used in ways DCMTK would reject
   - Determine whether validation belongs in the association layer or at service call sites

2. **C-GET / C-MOVE semantics**
   - Re-verify progress counting, final status construction, and cancellation support
   - Compare against current `dcmnet` provider semantics with a targeted matrix

3. **Negotiation policy differences**
   - Re-check whether the Rust allow-list/preference behavior is a desired policy
     improvement or an accidental divergence from DCMTK defaults

Recommended later pass:

1. Build service-level interoperability tests against DCMTK or Orthanc peers
2. Reproduce concrete mismatches first
3. Patch only the protocol differences that are both real and user-visible

---

## Rejected or outdated findings

These should **not** be reopened unless new evidence appears.

### 1. "`AT` VR is missing" in `dicom-toolkit-data`

- **Status:** rejected
- Manual check showed:
  - `Value::Tags` exists
  - `Vr::AT` handling exists in reader/json paths

### 2. "DIMSE sub-operation counters are not decoded" in `dicom-toolkit-net`

- **Status:** rejected
- Manual check showed:
  - `0000,1020` through `0000,1023` are already decoded in
    `crates/dicom-toolkit-net/src/dimse.rs`

### 3. "BulkDataURI callback never executed"

- **Status:** outdated
- Recent toolkit work already added and validated BulkDataURI support

### 4. C-FIND response dataset encoding hardcoded to Explicit VR LE

- **Status:** already fixed
- Included here only to avoid rediscovery

---

## Recommended execution order for the next parity pass

### Phase 1 — high-confidence, high-value

1. Finish `dicom-toolkit-core` ISO 2022 parity
   - escape-aware encoding
   - delimiter/PN behavior
   - fixture-backed verification for `IR 149`, `IR 58`, `IR 159`

2. Finish `dicom-toolkit-image` rendered parity
   - overlay burn-in
   - `YBR_FULL_422` planar handling

### Phase 2 — medium confidence, needs fresh verification

3. Re-audit `dicom-toolkit-data` JSON parity with current code
4. Re-audit `dicom-toolkit-net` DIMSE send/validation semantics with real peers

### Phase 3 — broader behavior parity

5. Decide whether to add:
   - explicit modality LUT tables
   - ROI/histogram/per-frame auto-window APIs
   - further DCMTK-like imaging convenience surfaces

---

## Exit criteria for the later pass

Do not call the later parity pass complete until these are true:

1. Every item marked **confirmed gap** above is either:
   - fixed and tested, or
   - explicitly documented as an intentional divergence

2. Every item marked **candidate gap** is either:
   - reproduced with a fixture / integration test and then fixed, or
   - rejected with evidence and removed from the open list

3. The final validation step includes:
   - focused crate tests
   - any new regression fixtures introduced by the parity work
   - `bash scripts/simulate-ci.sh`

---

## Practical note

The biggest lesson from this audit is that parity work should be done with
**fixtures and citations**, not just code reading. The reports were useful for
finding suspicious areas, but they were not reliable enough to patch from
directly. The next pass should preserve the same rule:

> identify → reproduce → compare against DCMTK → patch → add regression
