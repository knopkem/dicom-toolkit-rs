# Copilot instructions for `dicom-tookit-rs`

## Code Quality

- Write **production-ready Rust** — no placeholder logic, no `todo!()` left in non-test code, no `unwrap()` or `expect()` outside of tests or `main` startup validation where a panic is acceptable.
- Prefer `?` for error propagation. Define domain-specific error types with `thiserror`. Never use `anyhow` in library crates; `anyhow` is acceptable only in binary entry points.
- All public items must have doc comments (`///`). Include at least one `# Example` block for non-trivial public APIs.
- No `clippy` warnings — code must pass `cargo clippy -- -D warnings` clean. Apply `#[allow(...)]` only when genuinely necessary and always with a comment explaining why.
- Format all code with `rustfmt` (default settings). Never submit unformatted code.
- Avoid `unsafe` unless interfacing with C FFI (e.g., OpenJPEG). Every `unsafe` block must have a `// SAFETY:` comment explaining the invariants upheld.

---

## Rust Patterns

Apply idiomatic Rust patterns consistently:

- **Newtype pattern** for domain identifiers (e.g., `StudyUid(String)`, `SeriesUid(String)`) — prevents mixing up UIDs at the type level.
- **Builder pattern** for structs with many optional fields (e.g., query builders, config structs). Implement via a dedicated `XxxBuilder` struct with a consuming `build() -> Result<Xxx>`.
- **Typestate pattern** for protocol state machines (e.g., DIMSE association lifecycle: `Association<Unassociated>` → `Association<Established>`).
- **`From`/`Into`/`TryFrom`/`TryInto`** for all conversions between domain types and external types (DICOM elements, database rows, API DTOs).
- **`Display` + `Error`** implementations on all error types.
- **`Default`** on config and option structs where zero-value defaults are meaningful.
- Prefer **`Arc<dyn Trait>`** for shared, injectable dependencies (`MetadataStore`, `BlobStore`) — enables testing with mocks.
- Use **`tokio::sync`** primitives (`RwLock`, `Mutex`, `broadcast`, `mpsc`) over `std::sync` in async code.
- Leverage **`tower` middleware** (tracing, timeout, rate-limit) for Axum routes rather than duplicating cross-cutting logic in handlers.
- Prefer **`bytes::Bytes`** for zero-copy binary data passing between components (DICOM pixel data, multipart bodies).

- Keep changes surgical and aligned with existing workspace patterns.
- When a feature or milestone is complete, always check CI readiness before handing work off.
- Prefer `bash scripts/simulate-ci.sh` for a full local CI simulation before falling back to individual commands.
- Use the local equivalents of `.github/workflows/ci.yml` unless you are explicitly blocked:
  - `cargo check --workspace --all-targets`
  - `cargo test --workspace`
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `RUSTDOCFLAGS="-Dwarnings" cargo doc --workspace --no-deps`
  - `cargo deny check`
- If you cannot run one of those checks locally, say so explicitly and explain why.
- If a change affects packaged binaries or publish/release behavior, also inspect the relevant workflows in `.github/workflows/`.
