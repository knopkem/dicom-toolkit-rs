# Security Policy

## ⚠️ Medical Software Notice

This software is **NOT certified for clinical use**. It must not be used for
diagnostic or therapeutic purposes without independent validation and
regulatory clearance. See the project README for the full disclaimer.

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.2.x   | ✅ Current release |
| 0.1.x   | ❌ End of life     |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. **Email:** [TO BE CONFIGURED — use GitHub Security Advisories]
3. **Or:** Use GitHub's [private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability) feature on this repository

### What to include

- Description of the vulnerability
- Steps to reproduce
- Potential impact (data leakage, denial of service, remote code execution, etc.)
- Suggested fix (if you have one)

### Response timeline

- **Acknowledgment:** Within 48 hours
- **Assessment:** Within 7 days
- **Fix/Disclosure:** Coordinated with reporter, typically within 30 days

## Security Considerations for DICOM

DICOM network protocols (DIMSE over TCP) were designed for trusted hospital
networks. When exposing DICOM services to untrusted networks:

- **Always use TLS** — this toolkit supports TLS via `rustls`
- **Restrict AE titles** — configure allowed calling/called AE titles
- **Validate input** — malformed DICOM data could cause unexpected behavior
- **Limit resources** — configure max PDU size and connection limits
- **Network isolation** — place DICOM services behind a firewall/VPN

## Dependency Security

This project uses `cargo deny` to check for known vulnerabilities in
dependencies. Security advisories are monitored via the
[RustSec Advisory Database](https://rustsec.org/).
