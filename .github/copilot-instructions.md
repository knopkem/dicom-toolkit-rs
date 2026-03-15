# Copilot instructions for `dcmtk-rs`

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
