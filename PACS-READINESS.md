# PACS Readiness Analysis: dicom-toolkit-rs → Pure-Rust Orthanc Alternative

> **Date:** 2026-03-15
> **Goal:** Use dicom-toolkit-rs as the foundation for a pure-Rust DICOM PACS system with Orthanc feature parity and superior performance.

---

## 1. What dicom-toolkit-rs Already Provides ✅

| Area | Status | Detail |
|------|--------|--------|
| DICOM data model | ✅ Complete | Dataset, Element, 21 Value types, Sequences, PersonName, Date/Time |
| Part 10 file I/O | ✅ Complete | Read/write with all 4 uncompressed transfer syntaxes + deflate |
| DICOM JSON (PS3.18) | ✅ Complete | Encode & decode — ready for DICOMweb |
| DICOM XML (PS3.19) | ✅ Complete | Native DICOM Model |
| Character sets | ✅ Complete | 15+ encodings, ISO 2022, round-trip fidelity |
| C-ECHO | ✅ SCU+SCP | Full verification |
| C-STORE | ✅ SCU+SCP | Send and receive files |
| C-FIND | ✅ SCU | Query remote PACS |
| C-GET | ✅ SCU | Retrieve to self |
| C-MOVE | ✅ SCU | Retrieve to destination |
| TLS/SSL | ✅ Client+Server | rustls-based, cert auth possible |
| JPEG baseline codec | ✅ Encode+Decode | Process 1 (lossy) |
| JPEG-LS codec | ✅ Encode+Decode | Pure Rust, lossless + near-lossless |
| RLE codec | ✅ Encode+Decode | PackBits lossless |
| Image pipeline | ✅ Complete | Window/Level, Modality LUT, VOI, overlays, transforms |
| PNG export | ✅ Complete | 8-bit grayscale + RGB |
| Codec registry | ✅ Extensible | Trait-based, pluggable |
| Async networking | ✅ tokio-based | Non-blocking, high concurrency |
| CLI tools (8) | ✅ Complete | dcmdump, echoscu, storescu, storescp, findscu, img2dcm, dcmcjpls, dcmdjpls |
| 410 tests | ✅ All passing | Unit + integration + E2E |

---

## 2. Gap Analysis: What's Missing for an Orthanc-Class PACS

### 2.1 — DICOM SCP Server Framework (CRITICAL)

**Orthanc has:** A full SCP daemon handling C-STORE, C-FIND, C-GET, C-MOVE concurrently.

**dicom-toolkit-rs has:** Protocol-level implementations (PDU, DIMSE, service messages) but only a basic `storescp` CLI. No generic SCP framework with request routing, concurrent connection handling, or callback hooks for C-FIND/C-GET/C-MOVE on the server side.

**What to build:**
- [ ] Generic `DicomServer` accepting multiple concurrent associations
- [ ] Request router dispatching to service handlers (C-STORE, C-FIND, C-GET, C-MOVE)
- [ ] C-FIND SCP handler (query database, return matching results)
- [ ] C-GET SCP handler (retrieve from storage, send sub-operations)
- [ ] C-MOVE SCP handler (open new association to destination, forward instances)
- [ ] Configurable accepted SOP classes, transfer syntaxes, AE titles
- [ ] Connection pool / max-connections limit
- [ ] Graceful shutdown

### 2.2 — Storage Layer (CRITICAL)

**Orthanc has:** Filesystem storage + SQLite index (default), with PostgreSQL/MySQL plugins.

**dicom-toolkit-rs has:** File read/write only. No storage manager, no indexing, no deduplication.

**What to build:**
- [ ] Storage backend trait (`StorageBackend`: store, retrieve, delete, exists)
- [ ] Filesystem backend (configurable directory structure)
- [ ] Object storage backend (S3-compatible — optional)
- [ ] Instance deduplication (by SOP Instance UID)
- [ ] Storage commitment support

### 2.3 — Database / Index Layer (CRITICAL)

**Orthanc has:** SQLite default + PostgreSQL/MySQL plugins. Indexes Patient/Study/Series/Instance hierarchy. Supports tag-based queries.

**dicom-toolkit-rs has:** Nothing.

**What to build:**
- [ ] Database schema for DICOM hierarchy (Patient → Study → Series → Instance)
- [ ] Index commonly queried tags (PatientID, PatientName, StudyDate, Modality, AccessionNumber, StudyInstanceUID, SeriesInstanceUID, SOPInstanceUID)
- [ ] SQLite backend (default, zero-config)
- [ ] PostgreSQL backend (production scale)
- [ ] Query engine mapping C-FIND queries to SQL
- [ ] Full-text search on patient names (with charset awareness)
- [ ] Statistics (study count, series count, disk usage)
- [ ] Maintenance (orphan cleanup, re-indexing, compaction)

### 2.4 — REST API (CRITICAL)

**Orthanc has:** ~140 REST endpoints covering the full DICOM hierarchy, modalities, peers, system info, etc.

**dicom-toolkit-rs has:** Nothing (library only).

**What to build:**
- [ ] HTTP server (axum recommended — async, tower middleware, great Rust ecosystem fit)
- [ ] Patient/Study/Series/Instance CRUD endpoints
- [ ] Upload (POST multipart DICOM files)
- [ ] Download (GET instance as DICOM/PNG/JPEG)
- [ ] Query/search endpoints with filters
- [ ] Modality management (remote DICOM nodes)
- [ ] Send to modality (C-STORE), query modality (C-FIND), retrieve (C-MOVE/C-GET)
- [ ] System info, statistics, logs
- [ ] Bulk operations (delete study, anonymize study)
- [ ] Job queue for long-running operations (transfers, anonymization)

### 2.5 — DICOMweb (HIGH)

**Orthanc has:** Full DICOMweb plugin (WADO-RS, WADO-URI, QIDO-RS, STOW-RS).

**dicom-toolkit-rs has:** JSON/XML serialization ready but no HTTP layer.

**What to build:**
- [ ] WADO-RS — Retrieve instances/metadata/frames as multipart
- [ ] WADO-URI — Legacy single-instance retrieval
- [ ] QIDO-RS — Query for studies/series/instances via HTTP
- [ ] STOW-RS — Store instances via HTTP POST
- [ ] Multipart MIME encoding/decoding
- [ ] Thumbnail generation for WADO

### 2.6 — Authentication & Authorization (HIGH)

**Orthanc has:** HTTP basic auth, plugin-based authorization, Lua-scripted access control.

**dicom-toolkit-rs has:** TLS client cert (via rustls) only. No user management.

**What to build:**
- [ ] User management (create, update, delete users)
- [ ] Role-based access control (admin, read-only, upload-only)
- [ ] HTTP authentication (Basic, Bearer/JWT, OAuth2)
- [ ] DICOM association-level auth (AE title allowlists, TLS cert verification)
- [ ] Audit logging (who accessed what, when)

### 2.7 — Configuration System (MEDIUM)

**Orthanc has:** JSON config file, environment variable overrides, runtime reconfiguration.

**dicom-toolkit-rs has:** In-memory structs only.

**What to build:**
- [ ] TOML/JSON config file (server port, storage path, DB connection, AE title, limits)
- [ ] Environment variable overrides (12-factor app)
- [ ] Config validation on startup
- [ ] Hot-reload for non-critical settings

### 2.8 — Anonymization / Modification (MEDIUM)

**Orthanc has:** Built-in anonymization (Basic Profile from PS3.15), tag modification.

**dicom-toolkit-rs has:** Dataset manipulation (can set/remove tags) but no anonymization profiles.

**What to build:**
- [ ] DICOM Basic Application Level Confidentiality Profile (PS3.15 Annex E)
- [ ] Configurable anonymization rules (keep/remove/replace/hash)
- [ ] UID remapping (new UIDs for anonymized instances)
- [ ] Bulk anonymization (entire study)
- [ ] REST API endpoints for anonymization

### 2.9 — Plugin / Extension System (MEDIUM)

**Orthanc has:** C/C++ plugin SDK, Lua scripting, Python plugin.

**dicom-toolkit-rs has:** Trait-based codec registry only.

**What to build:**
- [ ] Event hook system (on-receive, on-store, on-query, on-stable-study)
- [ ] Trait-based plugin API for custom storage backends, auth, routing
- [ ] WASM plugin support (optional — unique Rust advantage)
- [ ] Lua or Rhai scripting for lightweight automation

### 2.10 — Worklist & MPPS (LOW for initial release)

**Orthanc has:** Worklist plugin (C-FIND SCP for worklist queries).

**dicom-toolkit-rs has:** Not implemented.

**What to build:**
- [ ] Modality Worklist SCP (return scheduled procedures)
- [ ] MPPS (N-CREATE, N-SET) for procedure status tracking

### 2.11 — JPEG 2000 Codec (LOW-MEDIUM)

**Orthanc has:** Via GDCM/OpenJPEG.

**dicom-toolkit-rs has:** Transfer syntax defined but no codec.

**Options:**
- [ ] Pure-Rust: use `jpeg2000` crate (if mature enough) or write one
- [ ] FFI bridge to OpenJPEG as optional feature (pragmatic)

### 2.12 — Web UI (LOW — can use existing viewers)

**Orthanc has:** Orthanc Explorer (built-in web UI), Orthanc Explorer 2 (enhanced).

**What to build:**
- [ ] Minimal built-in web UI (study list, series browser, image viewer)
- [ ] Or: integrate with existing open-source viewers (OHIF, Stone, Cornerstone.js)
- [ ] API-first approach — the REST API is the primary interface

---

## 3. Recommended Architecture for the PACS

```
┌─────────────────────────────────────────────────────────┐
│                     pacs-rs (binary)                     │
│                                                         │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │  DICOM SCP  │  │   REST API   │  │  DICOMweb API │  │
│  │  (dicom-toolkit-net)│  │   (axum)     │  │  (axum)       │  │
│  └──────┬──────┘  └──────┬───────┘  └───────┬───────┘  │
│         │                │                   │          │
│  ┌──────┴────────────────┴───────────────────┴───────┐  │
│  │              Service Layer (pacs-core)              │  │
│  │  ┌──────────┐ ┌────────────┐ ┌──────────────────┐ │  │
│  │  │ Ingest   │ │   Query    │ │  Retrieve/Send   │ │  │
│  │  │ Pipeline │ │   Engine   │ │  Orchestrator    │ │  │
│  │  └────┬─────┘ └─────┬──────┘ └───────┬──────────┘ │  │
│  └───────┼─────────────┼────────────────┼────────────┘  │
│          │             │                │               │
│  ┌───────┴─────────────┴────────────────┴────────────┐  │
│  │              Storage & Index Layer                  │  │
│  │  ┌──────────────┐          ┌────────────────────┐ │  │
│  │  │  File Store   │          │  Database Index    │ │  │
│  │  │  (fs / S3)    │          │  (SQLite / PG)     │ │  │
│  │  └──────────────┘          └────────────────────┘ │  │
│  └────────────────────────────────────────────────────┘  │
│                                                         │
│  ┌────────────────────────────────────────────────────┐  │
│  │                dicom-toolkit-rs (library)                   │  │
│  │  dicom-toolkit-core │ dicom-toolkit-dict │ dicom-toolkit-data │ dicom-toolkit-net │  │
│  │  dicom-toolkit-image │ dicom-toolkit-codec                         │  │
│  └────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

**Key Rust crate choices:**
- **HTTP:** `axum` (async, tower ecosystem, excellent for APIs)
- **Database:** `sqlx` (async, compile-time checked queries) with SQLite + PostgreSQL
- **Config:** `config` crate + `serde` for TOML/JSON/env
- **Auth:** `jsonwebtoken` for JWT, `argon2` for password hashing
- **Logging:** `tracing` (already used)
- **Object storage:** `opendal` or `rust-s3` for S3 backends

---

## 4. Pre-Publication Checklist for dicom-toolkit-rs

Before the PACS project, dicom-toolkit-rs itself should be published:

### 4.1 — License Compliance ⚠️

- [x] dicom-toolkit-rs is MIT OR Apache-2.0 — ✅ permissive, crates.io compatible
- [ ] **DCMTK license audit:** DCMTK itself is BSD 3-clause. Since this is a *clean-room port* (ported algorithms, not copied code), the port is independently licensed. However:
  - [ ] Add a disclaimer that this is a Rust *port inspired by* DCMTK, not affiliated with OFFIS
  - [ ] Ensure no DCMTK source code was copied verbatim (review JPEG-LS codec which was ported from CharLS — CharLS is BSD-3, note attribution)
  - [ ] Add NOTICE file crediting DCMTK and CharLS as algorithmic references
- [ ] **Dependency license audit:** Run `cargo deny check licenses` to verify all deps are compatible
  - `encoding_rs` → MIT/Apache-2.0 ✅
  - `jpeg-decoder` → MIT/Apache-2.0 ✅
  - `jpeg-encoder` → MIT/Apache-2.0 ✅
  - `png` → MIT/Apache-2.0 ✅
  - `flate2` → MIT/Apache-2.0 ✅
  - `rustls` → MIT/Apache-2.0/ISC ✅
  - `tokio` → MIT ✅
  - `serde` / `serde_json` → MIT/Apache-2.0 ✅
  - `clap` → MIT/Apache-2.0 ✅

### 4.2 — Disclaimers (required for medical software)

- [ ] Add **"NOT FOR CLINICAL USE"** disclaimer prominently in README
- [ ] Add disclaimer: "This software is not a certified medical device. It has not been validated for diagnostic or therapeutic use."
- [ ] Add disclaimer in library docs (lib.rs of each crate)
- [ ] Consider: FDA 510(k) / CE marking pathways if commercial clinical use is planned (far future)

### 4.3 — crates.io Publishing

- [ ] Choose crate names (check availability): `dicom-toolkit-core`, `dicom-toolkit-dict`, `dicom-toolkit-data`, `dicom-toolkit-net`, `dicom-toolkit-image`, `dicom-toolkit-codec`
  - ⚠️ "dcmtk" is OFFIS's trademark — consider renaming to `dicom-core`, `dicom-data`, etc. or `dcm-rs-*`
  - Better: pick a unique project name (e.g., `ferrite-dicom`, `oxide-dcm`, `dcm-toolkit`, etc.)
- [ ] Write crate-level documentation (lib.rs doc comments with examples)
- [ ] Add `categories` and `keywords` to each Cargo.toml (`["science::medical", "parser"]`)
- [ ] Add `repository`, `homepage`, `documentation` URLs
- [ ] Set version 0.1.0 (or 0.1.0-alpha.1 for initial publication)
- [ ] Publish in dependency order: core → dict → data → codec → image → net → tools

### 4.4 — GitHub Repository Setup

- [ ] Create public GitHub repository
- [ ] Add CI (GitHub Actions): build + test on Linux/macOS/Windows
- [ ] Add `cargo deny` for license/advisory checking
- [ ] Add `cargo clippy` and `cargo fmt --check` to CI
- [ ] Add security policy (SECURITY.md)
- [ ] Add CONTRIBUTING.md
- [ ] Add issue templates (bug report, feature request)
- [ ] Set up branch protection on `main`
- [ ] Add badges to README (CI, crates.io, docs.rs)

### 4.5 — Documentation

- [ ] Each crate: module-level doc comments
- [ ] Each public API: doc comments with examples
- [ ] Architecture overview doc (the crate diagram from README)
- [ ] DICOM conformance statement (what SOP classes/TS are supported)
- [ ] Migration guide from DCMTK C++ to dicom-toolkit-rs

---

## 5. Phased Roadmap for the PACS

### Phase 0 — Publish dicom-toolkit-rs (the library)
- Finalize naming, licensing, disclaimers
- Publish to crates.io
- Set up GitHub CI

### Phase 1 — Minimum Viable PACS
- DICOM SCP server (C-STORE receive, C-ECHO)
- Filesystem storage backend
- SQLite index (Patient/Study/Series/Instance)
- REST API: list patients/studies/series, upload, download
- TOML configuration file
- Single binary deployment

### Phase 2 — Query/Retrieve
- C-FIND SCP (query-by-patient, study, series)
- C-MOVE SCP (send to remote AE)
- C-GET SCP (return to requester)
- REST API: search with filters, send-to-modality

### Phase 3 — DICOMweb
- WADO-RS, QIDO-RS, STOW-RS
- Thumbnail/preview generation
- Multipart MIME handling

### Phase 4 — Production Readiness
- PostgreSQL backend
- Authentication (JWT + Basic Auth)
- Role-based access control
- Audit logging
- Connection pooling, rate limiting
- Health checks, metrics (Prometheus)
- Docker image

### Phase 5 — Advanced Features
- Anonymization (PS3.15 Basic Profile)
- Job queue (async transfers, batch operations)
- Plugin/hook system
- Lua/Rhai scripting
- S3 storage backend
- Web UI (OHIF integration or minimal built-in)

### Phase 6 — Orthanc Feature Parity
- Worklist / MPPS
- Peer-to-peer DICOM federation
- JPEG 2000 support
- DICOM-SR (Structured Reports) enhanced support
- WORM (Write-Once-Read-Many) storage compliance

---

## 6. Performance Advantages (Rust vs C++/Orthanc)

The PACS can leverage Rust's strengths to outperform Orthanc:

| Area | Orthanc (C++) | pacs-rs (Rust) |
|------|---------------|----------------|
| Memory safety | Manual (use-after-free risks) | Guaranteed (ownership model) |
| Concurrency | Thread pool, mutexes | async/await + tokio (M:N scheduling) |
| Connection handling | Thread-per-connection | Async (10K+ concurrent connections) |
| Binary size | ~20MB + deps | Single static binary ~5-10MB |
| Deployment | Needs C++ runtime, plugins | Single binary, zero deps |
| WASM target | Not possible | Possible for codec/data crates |
| Memory usage | Higher (C++ allocator) | Lower (zero-copy, arena allocation) |
| Startup time | ~1s | ~10ms |

---

## 7. Naming Considerations

The name "dcmtk" is associated with OFFIS. For the published library + PACS:

**Option A — Keep neutral:** `dicom-toolkit-rs` / `dicom-rs`
**Option B — Distinctive brand:** `ferrite-dicom` (Fe = iron → Rust)
**Option C — Clinical feel:** `oxipacs` / `rustpacs`
**Option D — Memorable:** `radium` (medical + Rust vibe)

Recommendation: Choose a distinctive name for the PACS product, keep dicom-toolkit-rs as the internal/development name for the library (or rename before publishing).
