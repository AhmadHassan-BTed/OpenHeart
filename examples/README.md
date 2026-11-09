# OpenHeart Example Codebases

This directory contains small Java fixture codebases used to exercise the **OpenHeart SCPG pipeline**, the GoF design-pattern detectors and the interactive Web Studio.

They are **fixtures, not benchmarks**: they are far too small to measure memory, speed or accuracy, and no published OpenHeart number comes from them. The benchmark requirements are in [docs/status.md §7](../docs/status.md#7-measurement-protocol). The fixtures carry no ground-truth labels (for example pattern role assignments), so they cannot score detector accuracy either.

---

## Directory Overview

| Directory | Contents | Size | Language |
|---|---|---|---|
| **[`test_patterns_codebase/`](./test_patterns_codebase/)** | Implementations of 9 Gang of Four patterns: Factory Method, Builder, Singleton, Adapter, Decorator, Facade, Observer, Strategy and Template Method | 35 files, 451 lines | Java |
| **[`big_project_src/`](./big_project_src/)** | A small multi-package application (`app`, `config`, `core`, `model`, `service`) for package-hierarchy and cross-package reference checks | 10 files, 225 lines | Java |
| **[`huge_enterprise_src/`](./huge_enterprise_src/)** | A banking-style multi-package application (`app`, `config`, `core`, `model`, `service`) with an entity hierarchy, enums and a transfer service | 12 files, 255 lines | Java |

---

## Quickstart: Analyzing Examples with the OpenHeart CLI

### 1. Analyze the GoF Design Patterns Codebase
```bash
# Compile the OpenHeart engine
cargo build --release

# Run the 10-phase analysis on the pattern fixtures
target/release/openheart analyze ./examples/test_patterns_codebase ./output_patterns

# Inspect the generated artifacts and diagrams
ls -la ./output_patterns/ ./output_patterns/diagrams/
```

### 2. Analyze the Multi-Package Fixtures
```bash
target/release/openheart analyze ./examples/big_project_src ./output_big
target/release/openheart analyze ./examples/huge_enterprise_src ./output_enterprise
```

### 3. Launch the Local Web Studio to Explore the Examples
```bash
target/release/openheart server 8080
```
Open `http://localhost:8080` in your web browser to browse the diagrams and jump from diagram elements to the example source files. Check the [diagram provenance matrix](../docs/status.md#5-diagram-provenance-matrix) to see which views are derived from the analysed code and which are still templates.
