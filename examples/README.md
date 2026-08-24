# OpenHeart Example & Benchmark Codebases

This directory contains standardized example codebases and benchmark test fixtures used for evaluating the **OpenHeart SCPG Compilation Pipeline**, verifying GoF design pattern detectors, and demonstrating interactive Web Studio features.

---

## Directory Overview

| Directory | Purpose & Contents | Languages | Key Architectural Characteristics |
|---|---|---|---|
| **[`test_patterns_codebase/`](./test_patterns_codebase/)** | 35 Classic Gang of Four (GoF) Design Pattern Implementations | Java | Factory Method, Builder, Singleton, Adapter, Decorator, Facade, Observer, Strategy, and Template Method |
| **[`sample_project/`](./sample_project/)** | Minimal Multi-Class Application | Java | Entrypoint `App.java` and dependency `Service.java` for quick pipeline sanity checks |
| **[`big_project_src/`](./big_project_src/)** | Multi-Package System Architecture | Java | Multi-tier architecture testing deep package hierarchies and inter-package dependencies |
| **[`huge_enterprise_src/`](./huge_enterprise_src/)** | Enterprise-Scale Benchmark Project | Java | High-volume class and method counts for benchmarking memory compression and #SAT path counts |

---

## Quickstart: Analyzing Examples with OpenHeart CLI

### 1. Analyze the GoF Design Patterns Codebase
```bash
# Compile OpenHeart engine
cargo build --release

# Run 10-phase analysis on test patterns
target/release/openheart analyze ./examples/test_patterns_codebase ./output_patterns

# Inspect generated artifacts
ls -la ./output_patterns/
```

### 2. Analyze the Minimal Sample Project
```bash
target/release/openheart analyze ./examples/sample_project ./output_sample
```

### 3. Launch Local Web Studio to Explore Examples
```bash
target/release/openheart server 8080
```
Open `http://localhost:8080` in your web browser to browse vector diagrams and synchronize code directly with the example source files.
