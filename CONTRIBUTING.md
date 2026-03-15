# Contributing to dicom-toolkit-rs

Thank you for your interest in contributing! This project aims to be a
high-quality, pure-Rust DICOM toolkit, and contributions are welcome.

## How to Contribute

### Reporting Bugs

- Use the GitHub issue tracker
- Include: Rust version, OS, minimal reproduction steps, expected vs actual behavior
- For DICOM file parsing issues, include a (de-identified) sample file if possible

### Suggesting Features

- Open a GitHub issue with the `enhancement` label
- Describe the use case, not just the solution
- Reference relevant DICOM standard sections (PS3.x) where applicable

### Submitting Code

1. **Fork** the repository and create a feature branch from `main`
2. **Write tests** for any new functionality
3. **Run the full test suite**: `cargo test --workspace`
4. **Run lints**: `cargo clippy --workspace -- -D warnings`
5. **Format code**: `cargo fmt --all`
6. **Open a pull request** with a clear description of your changes

### Code Style

- Follow standard Rust idioms and naming conventions
- Use `rustfmt` defaults (no custom configuration)
- Add doc comments (`///`) to all public APIs
- Keep `unsafe` usage to an absolute minimum (prefer zero)
- Prefer returning `Result<T, E>` over panicking

### DICOM Compliance

- Reference the DICOM standard section when implementing protocol features
- Tag constants should match PS3.6 names (converted to UPPER_SNAKE_CASE)
- Transfer syntax UIDs must match PS3.5/PS3.6 exactly

## Medical Software Disclaimer

⚠️ **This software is NOT intended for clinical use.** It has not been
validated or certified as a medical device under any regulatory framework
(FDA 510(k), CE marking, etc.). Contributors should be aware that changes
to DICOM parsing, networking, or image processing could have patient safety
implications if the software is misused in clinical settings.

## License

By contributing, you agree that your contributions will be licensed under the
same terms as the project: MIT OR Apache-2.0, at the user's option.
