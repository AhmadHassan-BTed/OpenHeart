# Security Policy

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

## Supported Versions

No release of **OpenHeart** has been tagged or published yet; the crate version is `0.1.0` (`Cargo.toml`). Security fixes are made on the `main` branch.

| Version | Supported          |
| ------- | ------------------ |
| `main` (unreleased, `0.1.x`) | :white_check_mark: |

Once releases are tagged, only the latest minor release line will receive security fixes, and this table will list it.

## Scope and Known Limitations

OpenHeart is experimental software. Before reporting, note these documented limitations (details in [docs/security-architecture.md](docs/security-architecture.md)):

- The HTTP server (`openheart server`) is a development server for local use and must not be exposed to untrusted networks.
- Binary artifact readers (`.tca` … `.scpg`) are not hardened against hostile files; do not open artifacts from untrusted sources.
- Artifact checksums (CRC) detect accidental corruption only; they provide no tamper evidence.

Reports that show these limitations being exceeded in a way the documentation does not describe — for example memory unsafety, path traversal, or code execution from a source file under analysis — are in scope.

## Reporting a Vulnerability

We take the security of **OpenHeart** seriously. If you discover a security vulnerability, please do **NOT** open a public GitHub issue.

Instead, report it through GitHub's [Private Vulnerability Reporting](https://github.com/AhmadHassan-BTed/OpenHeart/security/advisories/new) for this repository, or by email to **security@openheart.dev**.

### Information to Include

To help us triage and resolve the issue quickly, please include:
- A description of the vulnerability and potential impact.
- Step-by-step instructions to reproduce the issue or a proof-of-concept payload.
- Affected components (e.g. Phase 1 `.tca` reader, FNV-1a interner, tree-sitter adapter, HTTP server).
- Any proposed remediation or patch.

### Response Timeline

- **Acknowledgement**: Within 24 hours.
- **Triage & Assessment**: Within 72 hours.
- **Fix**: Target within 14 days depending on severity.

Thank you for helping keep OpenHeart secure!
