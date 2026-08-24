# Changelog

All notable changes to the **OpenHeart** project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

## [2.0.0] - 2026-08-24

### Added
- **Complete 19-Diagram Universal Suite**: Deterministic generation for all 14 standard OMG UML 2.5 projections + 5 deep compiler pipeline IRs (Control Flow Graph, Data Flow Graph, Control Dependence Graph, Call Graph, and ROBDD Saturation).
- **Declarative Architecture Manifest (`manifest.json`)**: Centralized manifest configuration in `web/diagrams/manifest.json` controlling categories, diagrams, 16 relationship terminologies, and 24 classifier node schemas with zero hardcoding.
- **Dynamic Cytoscape Stylesheet Compiler**: Real-time stylesheet generator in `ThemeManager` compiling Cytoscape edge styles, bezier curves, and arrowheads dynamically from `manifest.json`.
- **100% Free Forever Zero-Backend GitHub Ingestion**: Pure client-side GitHub repository cloning, file tree parsing, and multi-language AST extraction running in-browser on GitHub Pages.
- **Precision Monaco Source Code Synchronizer**: Sticky line numbering, multi-language tokenizers (Rust, Python, Kotlin, TypeScript, PlantUML), 2D bidirectional scrolling, and line jump `@keyframes pulse-line` animations.
- **Deterministic Spatial Layout Engines**: Multi-root 2-column grid layout for package hierarchies, cycle-breaking hierarchical rank algorithms, balanced actor wing layouts, and multi-track timing waveforms.

### Fixed
- **100 Deep Logical Errors Resolved Across Subsystems**:
  - Rust Backend (Issues 1–12, 61–72): Added missing diagram exporters in `JSONFactory`, normalized member visibility, stripped array subscripts (`Task[]`), and resolved wildcard generic bounds (`? extends T`).
  - SVG Vector Card Renderers (Issues 13–24, 69, 71): Added text length truncation with ellipsis, centered composite port pins, resolved ROBDD terminal node detection, and prepended UTF-8 charset data URIs.
  - Cytoscape Layout Engines (Issues 25–34, 73–82): Resolved cycle-breaking rank inversions, balanced actor wing columns, and fixed viewport preservation on graph switch.
  - File Tree & Monaco Code Synchronizer (Issues 35–42, 83–90): Added `data-node-id` matching, smooth auto-scroll, flex gutter line alignment, and multi-language syntax highlighting.
  - PlantUML Parser & GitHub Engine (Issues 43–60): Fixed relationship kinds for sequence messages, state transitions, activity flows, `[*]` state disambiguation, Java 17 records, and 403 API rate-limit graceful recovery.
  - ROBDD & Compiler Pipelines (Issues 91–100): Implemented 128-bit integer `#SAT` overflow protection, Cooper IDOM convergence bounds, and null-byte string safety.

## [1.1.0] - 2026-08-14

### Added
- **Universal Multi-Language Engine Support**: Dynamic generic language adapters for ingesting JavaScript, TypeScript, Python, C++, Rust, Go, Swift, PHP, Ruby, and extensionless scripts across all 10 compilation pipeline phases without hardcoded dependencies.
- **Autonomous PlantUML Diagram Generator CLI (`generate_diagrams.py`)**: Standalone tool that ingests any GitHub repository and outputs all 14 PlantUML `.puml` diagram files directly into `./output_diagrams/<repo_name>/`.
- **Client-Side Vector SVG Fallback Engine**: Pure client-side SVG vector diagram renderer in Web Studio so vector diagrams remain 100% online even during Kroki endpoint outages.
- **Strict Code-File Ingestion Filter**: Excludes non-code Markdown and text documentation files from symbol ingestion.

## [1.0.0] - 2026-08-13

### Added
- **10-Phase Pipeline Engine**: Completed all 10 analysis engine phases (Lexical Ingestion, BP AST, Symbol Table, CFG CSR, SSA Form, Call Graph, Traceability, ROBDD Paths, UML Metadata, SCPG Binary).
- **100% Dynamic 14 UML 2.5 Diagram Exporters**: Native Mermaid exporters for all 14 standard UML diagram types (Class, Object, Component, Deployment, Package, Composite Structure, Profile, Use Case, Activity, State Machine, Sequence, Communication, Interaction Overview, Timing).
- **Zero-Hardcoding Invariant**: 100% generic, AST-driven graph construction and Lowest Common Ancestor (LCA) edge scoping algorithm across any codebase.
- **Web Studio Portal (GitHub Pages)**: Live deployed interactive portal at `https://ahmadhassan-bted.github.io/OpenHeart/`.

## [0.1.0] - 2026-08-08

### Added
- **Phase 1 Engine**: Lexical Ingestion & Token Corpus Construction in Rust.
- **Core Memory Layouts**:
  - `TokenRecord` (16-byte cache-line aligned struct: `sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8`).
  - `TokenEntry` (16-byte backward index struct).
  - `SourceFileRecord` (64-byte fixed binary struct).
  - Bit-packed `sort_key` (`build_sort_key` & `unpack_sort_key`) with 48-bit `file_id`, 24-bit `line`, 8-bit `col`.
- **String Interning**:
  - Deduplicating `StringInterner` using 64-bit FNV-1a hash table with open addressing, load factor limit $\alpha = 0.75$, and length-prefixed byte storage.
- **Binary Serializer**:
  - `.tca` (Token Corpus Artifact) format writer and reader with 64-byte header (`0x544F4B434F525001`), section offset table, and CRC-64/ECMA verification checksum.
- **Corpus Invariants (1–4)**:
  - Automated verification of Monotonicity, Injectivity, Completeness, and Forward-Backward Index Consistency.
- **Adapters & Parsers**:
  - Tree-sitter integration, `LanguageAdapter` trait, `JavaLanguageAdapter`, and `AdapterRegistry`.
- **Automated Testing Suite**:
  - Integration tests covering parsing, interning, sorting, binary round-trip, and invariant assertions.
