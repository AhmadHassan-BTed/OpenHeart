# Changelog

All notable changes to the **OpenHeart** project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

👉 **[Launch OpenHeart Web Studio Portal (GitHub Pages)](https://ahmadhassan-bted.github.io/OpenHeart/)**

> **Versioning note.** The crate version is `0.1.0` (`Cargo.toml`) and no release has been tagged or published. The sections below that were previously headed `2.0.0`, `1.1.0` and `1.0.0` were development milestones, not releases; they are now headed by date. The first tagged release will be `0.1.0`, and from then on the version in `Cargo.toml`, the git tag and the heading here must agree.

## [Unreleased]

### Added
- [`docs/status.md`](docs/status.md): implementation status per phase, language support matrix, diagram provenance matrix, claims ledger, measurement protocol and the list of environment variables the engine reads.
- `docs/openheart.bib`: the bibliography that `docs/openheart_research_paper.tex` cites (`\bibliography{openheart}`), which was missing.
- Architecture Decision Records for the hand-rolled HTTP server, the logger, file hand-off between phases, typed errors, the pinned tree-sitter 0.20 stack and the documentation status rule (`docs/technical-decisions.md`).

### Changed
- **Documentation now matches the code.** A new page, [`docs/status.md`](docs/status.md), records the implementation status of every phase, a per-language support matrix, a per-diagram provenance matrix, a ledger of withdrawn claims and the measurement protocol that any future accuracy, space or speed number must follow. The README, roadmap, architecture and overview pages, the phase plans and the research paper were corrected against it.
- **Minimum supported Rust version** is 1.88, declared as `rust-version` in `Cargo.toml` (previously documented as 1.75).
- **Roadmap status rule**: an item is marked implemented only when it links to the module and to a test that asserts an expected result.
- **Research paper**: the path function is now a flow-conservation encoding whose model count equals the number of acyclic paths (the previous encoding left the edges of unreached blocks free); the ROBDD size theorem is restated as a proven $m(d+1)$ bound with the linear bound as a conjecture; the IFDS bound is corrected to $O(|E||D|^3)$; the dominance-frontier and SSA sparsity lemmas are corrected; unmeasured size tables and LZ4 figures are removed; and a "Reference Implementation Status" section lists every divergence between the model and the code.
- The HTML design pages under `docs/` and the README overview graphic now say that their figures describe the design, not measurements.

### Removed
- Withdrawn headline claims: "128× memory compression", "4.65 MB per 1M AST nodes", "zero-copy memory mapping of all phase artifacts", "F1 = 1.0000 across Java, Kotlin, Python, Rust, TypeScript and C++", "19 projections derived from binary graph layers", "all 14 UML 2.5 diagrams", IFDS solvers, Andersen points-to, 1-CFA, Rudell sifting, exact #SAT path counting, CFL-reachability, O(1) incremental delta updates, LZ4-compressed `.scpg` sizes, and the unsourced competitor figures. The reasons are recorded in the [claims ledger](docs/status.md#6-claims-ledger).
- The static "CI passing" and "security enforced" badges and the hard-coded test counts in the README.
- `docs/RESEARCH_PAPER_NEXT_PROMPT.txt` (an assistant prompt, not documentation) and the empty `docs/plans/phase4.md` (the Phase 4 design is in `docs/plans/phase4_cfg_dominator_analysis.html`).
- Undocumented configuration knobs that no code reads (`OPENHEART_MAX_MEMORY_MB`, `OPENHEART_ARTIFACT_DIR`, `OPENHEART_STRICT_INVARIANTS`) from the security documentation.

### Fixed
- Broken links in `SUPPORT.md` and `examples/README.md` (the latter pointed to a `sample_project/` directory that does not exist).

## Milestone 2026-08-24 (untagged; previously headed 2.0.0)

### Added
- **19-diagram suite**: exporters for 14 UML view types and 5 compiler-IR view types (Control Flow Graph, Data Flow Graph, Control Dependence Graph, Call Graph, ROBDD). *Correction:* several of these views are templates whose content does not depend on the analysed program, and the output was not deterministic across runs; see the [diagram provenance matrix](docs/status.md#5-diagram-provenance-matrix).
- **Declarative architecture manifest (`manifest.json`)**: `web/diagrams/manifest.json` defines categories, diagrams, 16 relationship kinds and 24 node kinds for the web studio.
- **Dynamic Cytoscape stylesheet compiler**: `ThemeManager` compiles Cytoscape edge styles, bezier curves and arrowheads from `manifest.json`.
- **Zero-backend GitHub ingestion in the browser**: client-side repository fetching and file-tree parsing on GitHub Pages. *Correction:* class extraction is a regular-expression heuristic over a sample of files, not AST parsing, and is independent of the Rust engine.
- **Monaco source synchronizer**: sticky line numbering, syntax highlighting for several languages, 2D scrolling and line-jump animations.
- **Spatial layout engines**: multi-root grid layout for package hierarchies, cycle-breaking hierarchical ranks, balanced actor wing layouts and multi-track timing waveforms.

### Fixed
- A batch of 100 issues across the subsystems:
  - Rust backend (issues 1–12, 61–72): missing diagram exporters in `JSONFactory`, member-visibility normalisation, stripping of array subscripts (`Task[]`), wildcard generic bounds (`? extends T`).
  - SVG card renderers (issues 13–24, 69, 71): text truncation with ellipsis, centred composite port pins, ROBDD terminal-node detection, UTF-8 charset data URIs.
  - Cytoscape layout engines (issues 25–34, 73–82): cycle-breaking rank inversions, actor wing columns, viewport preservation on graph switch.
  - File tree and Monaco synchronizer (issues 35–42, 83–90): `data-node-id` matching, auto-scroll, gutter alignment, syntax highlighting.
  - PlantUML parser and GitHub engine (issues 43–60): relationship kinds for sequence messages, state transitions and activity flows, `[*]` state disambiguation, Java 17 records, graceful handling of GitHub API rate limits (403).
  - ROBDD and compiler pipelines (issues 91–100): 128-bit integer `#SAT` overflow protection, Cooper IDOM convergence bounds, null-byte string safety.

## Milestone 2026-08-14 (untagged; previously headed 1.1.0)

### Added
- **Generic language adapters**: files with JavaScript, TypeScript, Python, C++, Rust, Go, Swift, PHP, Ruby and extensionless-script extensions are accepted by the server's collector. *Correction:* only Java and JavaScript grammars are linked; every non-Java language is parsed with the JavaScript grammar and receives no semantic analysis, so this is not multi-language support. See the [language support matrix](docs/status.md#2-language-support-matrix).
- **PlantUML diagram generator script (`scripts/generate_diagrams.py`)**: ingests a GitHub repository and writes 14 PlantUML `.puml` files into `./output_diagrams/<repo_name>/`.
- **Client-side SVG fallback renderer** in the web studio, used when the Kroki endpoint is unavailable.
- **Code-file ingestion filter**: Markdown and text documentation files are excluded from symbol ingestion.

## Milestone 2026-08-13 (untagged; previously headed 1.0.0)

### Added
- **10-phase pipeline engine**: all ten phases (Lexical Ingestion, BP AST, Symbol Table, CFG CSR, SSA Form, Call Graph, Traceability, ROBDD Paths, UML Metadata, SCPG Container) run end to end on Java input. *Correction:* "completed" described the pipeline wiring, not feature completeness; see the [phase capability ledger](docs/status.md#4-phase-capability-ledger).
- **Mermaid exporters for the 14 UML view types** (Class, Object, Component, Deployment, Package, Composite Structure, Profile, Use Case, Activity, State Machine, Sequence, Communication, Interaction Overview, Timing).
- **Lowest Common Ancestor (LCA) edge-scoping** in diagram construction. *Correction:* the claimed "zero-hardcoding invariant" did not hold; several views were templates.
- **Web studio portal (GitHub Pages)** deployed at `https://ahmadhassan-bted.github.io/OpenHeart/`.

## Milestone 2026-08-08 (untagged; crate version 0.1.0)

### Added
- **Phase 1 engine**: lexical ingestion and token corpus construction in Rust.
- **Core memory layouts**:
  - `TokenRecord` (16-byte struct: `sort_key: u64`, `text_id: u32`, `len: u16`, `token_type: u8`, `_padding: u8`).
  - `TokenEntry` (16-byte backward index struct).
  - `SourceFileRecord` (64-byte fixed binary struct).
  - Bit-packed `sort_key` (`build_sort_key` and `unpack_sort_key`) with a 16-bit `file_id` (bits 63–48), a 24-bit `line` (bits 47–24), a 16-bit `col` (bits 23–8) and an 8-bit reserved field. *Correction:* this entry previously described a 48-bit `file_id` and an 8-bit `col`, which does not match the code.
- **String interning**: deduplicating `StringInterner` with a 64-bit FNV-1a hash table, open addressing, a load-factor limit of 0.75 and length-prefixed byte storage.
- **Binary serializer**: `.tca` (Token Corpus Artifact) writer and reader with a 64-byte header (`0x544F4B434F525001`), a section offset table and a CRC-64/ECMA checksum.
- **Corpus invariants (1–4)**: verification of monotonicity, injectivity, completeness and forward-backward index consistency.
- **Adapters and parsers**: tree-sitter integration, the `LanguageAdapter` trait, `JavaLanguageAdapter` and `AdapterRegistry`.
- **Tests**: integration tests covering parsing, interning, sorting, binary round-trip and invariant assertions.
