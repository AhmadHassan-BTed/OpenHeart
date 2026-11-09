# Getting Started with OpenHeart (SCPG Engine)

Welcome to **OpenHeart**, an experimental static program-analysis engine and UML diagram platform designed, created, and maintained by **Ahmad Hassan (B-Ted)**.

> **Before you start.** Only **Java** is supported; several diagram views are still templates. See the [support matrices in status.md](./status.md) for what each phase, language and view does today.

---

## 🚀 Live Web Studio Portal (GitHub Pages)

Browse the bundled 19-view sample diagram set in your browser with zero installation:

👉 **[Launch OpenHeart Web Studio Portal](https://ahmadhassan-bted.github.io/OpenHeart/)**

You can also paste a public GitHub repository URL (`https://github.com/owner/repo`). On GitHub Pages this does **not** run the Rust engine: a JavaScript heuristic extractor analyses a sample of the repository's files, fetched through the unauthenticated GitHub REST API (60 requests per hour). Treat the result as a preview; the input-selection rules are recorded in [status.md §3](./status.md#3-entry-points-and-input-selection). For engine output, run the local server (step 4 below).

---

## 🛠️ Prerequisites

- **Rust Toolchain**: Rust **1.88** or newer (edition 2021) — the `rust-version` declared in `Cargo.toml`
- **Cargo**: Included with the standard Rust toolchain
- **Git**: v2.20 or newer
- **Python**: 3.10+ (optional, for serving the static studio without the engine)

---

## ⚡ Quick Run (One-Click / One-Command)

- **Windows**: Double-click `run.bat` or run:
  ```cmd
  run.bat
  ```
- **Linux / macOS**: Run:
  ```bash
  ./run.sh
  ```
*Detects Rust/Cargo or Python, starts a server on port 8080, and opens your default browser on the studio. With Python only, the studio shows the bundled sample diagrams; analysis requires the Rust engine.*

---

## 💻 Local Setup & Quickstart

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/AhmadHassan-BTed/OpenHeart.git
   cd OpenHeart
   ```

2. **Build the Engine Binary**:
   ```bash
   cargo build --release
   ```

3. **Run Pipeline Analysis via CLI**:
   ```bash
   target/release/openheart analyze <PATH_TO_SOURCE_PROJECT> <OUTPUT_DIRECTORY>
   ```
   The CLI collects source files recursively (only Java is supported) and writes the ten phase artifacts plus the exported diagrams. Add `--verbose` or `--trace` for more logging.

4. **Launch the Local HTTP Server** (development use only; do not expose it to untrusted networks):
   ```bash
   target/release/openheart server 8080
   ```
   The subcommand is `server [PORT]`; open `http://localhost:8080` to use the studio backed by the Rust engine.

5. **Serve the Static Studio Without the Engine** (sample diagrams only):
   ```bash
   cd web
   python3 -m http.server 8000
   ```
   Open `http://localhost:8000`.

6. **Inspect an Artifact**:
   ```bash
   target/release/openheart inspect <OUTPUT_DIRECTORY>/callgraph.cga
   ```
   At the audited baseline `inspect` identified the artifact from its magic number without a full integrity validation; see [security-architecture.md §3.2](./security-architecture.md#32-what-is-verified).

7. **Run the Tests and Quality Gates**:
   ```bash
   cargo test --all-targets   # unit and integration tests (skips doctests)
   make ci                    # fmt, check, clippy -D warnings, tests, doctests, rustdoc
   ```

---

## 📚 Documentation Index

- **[Live Web Studio](https://ahmadhassan-bted.github.io/OpenHeart/)**: Deployed interactive portal on GitHub Pages.
- **[Implementation Status](status.md)**: What the code does today, per phase, language and diagram.
- **[System Overview](overview.md)**: The SCPG formal model and design goals.
- **[System Architecture](architecture.md)**: 10-phase pipeline design and the 19 view mappings.
- **[Codebase Guide](codebase-guide.md)**: Repository layout, module structure and developer reference.
- **[Succinct Structures](succinct_structures.md)**: BP trees, CSR graphs and ROBDDs, design versus implementation.
- **[UML Derivation](uml_derivation.md)**: Derivation rules for the 19 view types, with per-view status.
- **[Architectural Roadmap](../ROADMAP.md)**: Phase status and planned work.
