# Findings index

Generated from `findings.json`. Status is per finding across the owners it was routed to.

| Status | Count |
|---|---|
| deferred-testing-phase | 86 |
| fixed | 53 |
| in-queue | 530 |
| not-yet-dispatched | 1 |
| partial | 172 |
| skipped | 16 |

## context:AUTO (22)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| AUTO-01 | critical | flaw | fixed | /api/source lets any client read any file; server listens on all interfaces with CORS * | src/adapters/server.rs |
| AUTO-02 | high | flaw | fixed | Unvalidated repo_url goes straight into git clone (option injection, SSRF) | src/adapters/server.rs, src/adapters/web_repo.rs |
| AUTO-03 | high | flaw | partial | Unsound unsafe aliasing in the ROBDD manager can read freed memory | src/psa/bdd/mod.rs, src/psa/bdd/apply.rs, src/psa/bdd/unique_table.rs |
| AUTO-04 | high | flaw | partial | Showcase diagrams are not reproducible engine output; hash and 'verified' are constants | src/scpg/diagram/export/json.rs, web/diagrams/statemachine.json, web/diagrams/usecase.json |
| AUTO-05 | high | flaw | partial | Only Java and JavaScript grammars exist; Kotlin, TypeScript and Rust use the wrong grammar | Cargo.toml, src/ingestion/adapter/kotlin.rs, src/ast/adapter/registry.rs |
| AUTO-06 | high | flaw | deferred-testing-phase | The F1 metric scores 1.0 on empty sets and the published result cannot be reproduced | ruthless_verify.py, ruthless_config.json, scripts/test_ruthless_popular_datasets.py |
| AUTO-07 | high | flaw | partial | Docker image cannot build or start; minimum Rust version is undeclared and misdocumented | Dockerfile, Cargo.toml, src/main.rs |
| AUTO-08 | medium | flaw | partial | Diagram files: duplicate copies, communication equals sequence, empty Mermaid files, duplicate ids | web/diagrams/manifest.json, web/diagrams/communication.json, web/diagrams/class.json |
| AUTO-09 | medium | flaw | partial | Binary artifact readers panic or allocate huge buffers on malformed input | src/uma/serializer.rs, src/symbol/serializer.rs, src/tra/serializer.rs |
| AUTO-10 | medium | flaw | fixed | Server quietly analyzes a different repository and swallows I/O errors | src/adapters/server.rs, src/main.rs |
| AUTO-11 | medium | flaw | partial | Heavy copy-paste in the diagram exporters (35% exact, 79% after normalization) | src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/plantuml.rs |
| AUTO-12 | medium | enhancement | partial | Very long functions hold most of the risk | src/symbol/passes/pass1_discovery.rs, src/uma/serializer.rs, src/adapters/server.rs |
| AUTO-13 | medium | enhancement | partial | Only 29.6% of public Rust items have doc comments | src/lib.rs, src/ssa, src/cfg |
| AUTO-14 | medium | flaw | fixed | 14% of the web JavaScript is dead legacy UI code | web/app.js, web/js/ui.js, web/js/viewer.js |
| AUTO-15 | medium | flaw | partial | CI has no linting, minimum-version, supply-chain or release-artifact checks | .github/workflows/ci.yml, .github/workflows/release.yml, Cargo.toml |
| AUTO-16 | medium | flaw | partial | Overflow checks are off in dev and test builds, alongside 846 unchecked numeric casts | Cargo.toml, src/uma/serializer.rs, src/symbol/passes/pass1_discovery.rs |
| AUTO-17 | medium | enhancement | deferred-testing-phase | The test suite is thin and no test backs the headline claims | tests, src/psa |
| AUTO-18 | low | flaw | partial | CDN scripts load without Subresource Integrity | web/index.html |
| AUTO-19 | low | flaw | partial | Broken relative links and duplicated HTML docs | SUPPORT.md, examples/README.md, docs/ast/phase2_bp_architecture.html |
| AUTO-20 | low | flaw | partial | Hardcoded developer paths, ports and personal URLs | src/adapters/server.rs, src/adapters/web_repo.rs, ruthless_verify.py |
| AUTO-21 | low | flaw | partial | .env.example documents settings the code never reads | .env.example, src/core/logger.rs, Dockerfile |
| AUTO-22 | low | flaw | fixed | A prompt written for an AI assistant is committed under docs/ | docs/RESEARCH_PAPER_NEXT_PROMPT.txt |

## context:BENCH (19)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| BENCH-01 | critical | flaw | partial | 'F1 = 1.0000 across six languages' compares the engine against a near-copy of itself | README.md, docs/overview.md, ruthless_verify.py |
| BENCH-02 | high | flaw | partial | The six-language claim rests on only two linked grammars | Cargo.toml, src/ingestion/adapter/registry.rs, src/ingestion/adapter/kotlin.rs |
| BENCH-03 | high | flaw | partial | The code's own data structures contradict the 128x and 4.65 MB per 1M AST nodes claims | README.md, docs/overview.md, docs/architecture.md |
| BENCH-04 | high | flaw | deferred-testing-phase | Latency and O(1) traceability claims are unmeasured and contradict each other | Cargo.toml, docs/overview.md, docs/architecture.md |
| BENCH-05 | high | flaw | partial | The paper's tables cannot be reproduced, and its space theorem relies on structures the code lacks | docs/openheart_research_paper.tex, docs/architecture.md, Cargo.toml |
| BENCH-06 | high | flaw | deferred-testing-phase | Tests named 'formal verification' and 'accuracy' are smoke tests with no expected outputs | tests/research_paper_theorems_tests.rs, tests/pipeline_accuracy_tests.rs, tests/adversarial_trick_tests.rs |
| BENCH-07 | high | flaw | deferred-testing-phase | Corpora are unpinned, and the server can silently analyze a different repository | .gitignore, src/adapters/server.rs, ruthless_verify.py |
| BENCH-08 | high | flaw | partial | The advertised IFDS solvers and Andersen points-to analysis are local marking passes | src/ssa/ifds.rs, src/cg/points_to.rs, README.md |
| BENCH-09 | high | enhancement | partial | There is no machine-readable export, so scoring has to regex-parse PlantUML | src/main.rs, src/scpg/api/mod.rs, ruthless_verify.py |
| BENCH-10 | medium | flaw | deferred-testing-phase | Evaluation scripts either cannot fail or never run the engine | scripts/benchmark_all_java_design_patterns.py, scripts/test_ruthless_popular_datasets.py, scripts/verify_all_14_uml_diagrams.py |
| BENCH-11 | medium | flaw | skipped | Machine-generated .urm.puml files are presented as architect ground truth, and the oracle is hand-patched | scripts/benchmark_all_java_design_patterns.py, scripts/test_ruthless_popular_datasets.py |
| BENCH-12 | medium | flaw | deferred-testing-phase | Pattern detection has no labelled evaluation and partly relies on class names | src/uma/patterns/factory.rs, src/uma/patterns/state.rs, src/uma/patterns/builder.rs |
| BENCH-13 | medium | flaw | deferred-testing-phase | Reported timings mix an HTTP round trip, a debug build and file writes into the repo | scripts/benchmark_all_java_design_patterns.py, src/adapters/server.rs, Cargo.toml |
| BENCH-14 | medium | flaw | partial | CI has no accuracy or performance gate, the badge is static, and the test count is stale | .github/workflows/ci.yml, README.md, scripts/ci_check.sh |
| BENCH-15 | medium | flaw | partial | The web studio uses a separate regex extractor capped at 80 files | web/js/github-engine.js, README.md, src/main.rs |
| BENCH-16 | medium | flaw | partial | 'Zero-copy' artifacts are copied and re-indexed on load | README.md, src/ast/serializer.rs |
| BENCH-17 | medium | flaw | partial | UML 2.5 claims have no conformance checks, and timing diagrams use a made-up clock | src/scpg/diagram/export/xmi.rs, src/scpg/diagram/export/plantuml.rs, docs/uml_derivation.md |
| BENCH-18 | low | flaw | fixed | Competitor comparisons use unsourced figures that published data contradicts | docs/overview.md |
| BENCH-19 | low | flaw | skipped | The layouts' 'zero overlap' guarantees are never measured | web/js/uml-layout.js, web/js/graph-canvas.js |

## context:COMP (15)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| COMP-01 | critical | enhancement | partial | Genuine Kotlin and multi-language frontends with a versioned support matrix | src/ingestion/adapter/kotlin.rs, src/ingestion/adapter/registry.rs, src/ast/adapter/registry.rs |
| COMP-02 | critical | enhancement | in-queue | Build-system, classpath and bytecode awareness | src/main.rs, src/symbol/std_library/java_stubs.rs, src/adapters/server.rs |
| COMP-03 | critical | enhancement | partial | Real interprocedural analyses: constraint-based points-to, RTA/VTA, IFDS/IDE and taint specs | src/cg/points_to.rs, src/cg/resolution.rs, src/ssa/ifds.rs |
| COMP-04 | critical | enhancement | deferred-testing-phase | Benchmark-grade accuracy and scale evidence | README.md, ruthless_verify.py, scripts/benchmark_all_java_design_patterns.py |
| COMP-05 | high | enhancement | partial | Declarative query language exposed via CLI, HTTP and library | src/scpg/query/mod.rs, src/scpg/query/cfl.rs, src/scpg/query/slice.rs |
| COMP-06 | high | enhancement | partial | Standard interchange: SARIF, GraphML/DOT/GraphSON, SCIP and full XMI 2.5.1 | src/scpg/diagram/export/xmi.rs, src/scpg/diagram/factory.rs, src/scpg/diagram/export/json.rs |
| COMP-07 | high | enhancement | partial | Real incremental, commit-aware analysis | src/scpg/incremental/mod.rs, src/tra/delta/invalidation.rs, src/ingestion/parser/tree_sitter.rs |
| COMP-08 | high | enhancement | partial | Demonstrated large-codebase scale: indexed, parallel, streaming pipeline | src/cg/resolution.rs, src/scpg/query/cfl.rs, src/adapters/server.rs |
| COMP-09 | high | enhancement | partial | CI distribution: binaries, GitHub Action, SARIF upload and PR diagram diffs | .github/workflows/release.yml, .github/workflows/ci.yml, Dockerfile |
| COMP-10 | high | enhancement | partial | Wire token-anchored traceability into exports and the studio | src/scpg/diagram/export/json.rs, src/scpg/diagram/factory.rs, src/scpg/query/navigation.rs |
| COMP-11 | high | enhancement | partial | Diagram UX parity: search, semantic filters, focus+context, commit diffs, local rendering | web/js/graph-canvas.js, web/js/icons.js, web/js/api.js |
| COMP-12 | medium | enhancement | skipped | IDE and LSP integration | src/main.rs, Cargo.toml, src/adapters/server.rs |
| COMP-13 | medium | enhancement | partial | Open extension model: loadable language adapters, rule/pattern packs, stable schema | src/ingestion/adapter/registry.rs, src/ast/adapter/registry.rs, src/symbol/adapter/mod.rs |
| COMP-14 | medium | enhancement | partial | Semantics-based design-pattern and architecture recovery | src/uma/patterns/singleton.rs, src/uma/patterns/observer.rs, scripts/benchmark_all_java_design_patterns.py |
| COMP-15 | medium | enhancement | partial | Claims ledger, docs-code alignment and contributor community | README.md, ROADMAP.md, docs/overview.md |

## find:BDD (17)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| BDD-01 | critical | flaw | fixed | Use-after-free: BDD apply/not/restrict read through a raw slice of the Vec they push into | src/psa/bdd/mod.rs:77-96, src/psa/bdd/mod.rs:99-110, src/psa/bdd/mod.rs:119-132 |
| BDD-02 | critical | flaw | partial | #SAT(f_paths) is not the path count: Phi_b leaves unreached edges free, so paths are overstated exponentially | src/psa/construction/constraints.rs:41-86, src/psa/construction/mod.rs:84-109, src/psa/bdd/sat_count.rs:1-20 |
| BDD-03 | high | flaw | partial | Static variable orders make ROBDDs exponential on plain sequential code, with no node or time budget | src/psa/ordering/rpo.rs:12-64, src/psa/ordering/force.rs:19-76, src/psa/ordering/force.rs:106-118 |
| BDD-04 | high | flaw | fixed | compute_max_path_length enumerates every walk (exponential) and returns the bound for any loop | src/psa/construction/mod.rs:153-173 |
| BDD-05 | high | flaw | partial | Feasibility filter is unsound: SSA ids are reused as BDD variable indices and collide with edge variables | src/psa/construction/feasibility.rs:63-75, src/psa/construction/feasibility.rs:85-101, src/psa/construction/feasibility.rs:108-124 |
| BDD-06 | high | flaw | partial | 'Rudell sifting' is a no-op: only the ordering table is permuted, never the BDD | src/psa/ordering/sifting.rs:44-82, src/psa/ordering/mod.rs:116-129, src/psa/construction/mod.rs:103 |
| BDD-07 | high | flaw | partial | The 'SCPG hash chain' does not hash the artifacts; staleness detection is global and non-deterministic | src/tra/uml_link/hash_chain.rs:28-56, src/tra/builder.rs:97-109, src/tra/uml_link/mod.rs:60 |
| BDD-08 | high | flaw | partial | Incremental invalidation is dead code and unsound; the documented 5-stage protocol does not exist | src/tra/delta/mod.rs:20-47, src/tra/delta/invalidation.rs:7-20, src/scpg/incremental/mod.rs:25-34 |
| BDD-09 | medium | flaw | partial | Backward index returns fabricated or misaligned source ranges | src/tra/backward/blk_bi.rs:17-21, src/tra/backward/ssa_bi.rs:13-41, src/core/types/ssa.rs:43 |
| BDD-10 | medium | flaw | partial | 'O(1) bidirectional traceability' is not delivered: forward and navigation queries are scans | src/tra/forward/sym_span.rs:50-71, src/tra/forward/cs_span.rs:41-62, src/tra/uml_link/mod.rs:67-68 |
| BDD-11 | medium | flaw | partial | BDD engine lacks complement edges, a persistent computed table and GC; the artifact ships about 8x garbage nodes | src/psa/bdd/mod.rs:32-37, src/psa/bdd/mod.rs:77-78, src/psa/bdd/mod.rs:99-131 |
| BDD-12 | medium | flaw | partial | Recursion, interprocedural composition and BDD queries are not implemented | src/psa/construction/recursive.rs:13-28, src/psa/mod.rs:14-18, src/psa/mod.rs:98-102 |
| BDD-13 | medium | flaw | partial | edge_index_of linear scans make Phase 8 O(E^2) and map parallel edges to one variable | src/psa/ordering/rpo.rs:106-108, src/psa/construction/constraints.rs:42-50, src/psa/ordering/force.rs:92-129 |
| BDD-14 | medium | enhancement | partial | No reference/occurrence index and no stable symbol identity (gap against Kythe/SCIP) | src/tra/types.rs:125-136, src/symbol/serializer.rs:14-34, src/ingestion/mod.rs:69-70 |
| BDD-15 | medium | flaw | deferred-testing-phase | Phase 7/8 tests and the 'theorem' tests are tautological; no ground truth, Miri, fuzzing or benchmarks | tests/research_paper_theorems_tests.rs:45-85, tests/research_paper_theorems_tests.rs:141-152, tests/psa_tests.rs:105-112 |
| BDD-16 | low | flaw | partial | sat_count silently saturates at u64::MAX, and consumers then wrap | src/psa/bdd/sat_count.rs:60, src/psa/bdd/sat_count.rs:79-92, src/uma/visitor.rs:214 |
| BDD-17 | low | flaw | partial | PSA/TRA readers trust header counts; the PSA uses a different and slow CRC-64 variant | src/psa/serializer.rs:192-244, src/psa/serializer.rs:259-279, src/tra/serializer.rs:130-138 |

## find:CORE (17)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| CORE-01 | critical | flaw | partial | Only Java has a real grammar: Kotlin is parsed as Java then JavaScript; Python/Rust/TS/C++ as JavaScript | src/ingestion/adapter/kotlin.rs:29-31, src/ingestion/adapter/registry.rs:34-39, src/ingestion/adapter/registry.rs:52-57 |
| CORE-02 | critical | flaw | partial | Java reducer's default Drop silently discards constructor bodies, switch bodies, try-with-resources and generic types | src/ast/adapter/java.rs:105-131, src/cfg/stmts/mod.rs:41, src/cfg/stmts/try_stmt.rs:62-66 |
| CORE-03 | high | flaw | partial | Java NodeAttr encoding matches modifier substrings (annotations included), maps 9 operators, and its decoder is a stub | src/ast/adapter/java.rs:133-179, src/core/types/ast.rs:224-232, src/symbol/passes/pass1_discovery.rs:895-930 |
| CORE-04 | high | flaw | partial | The 'succinct' AST costs ~209 B/node in memory and ~25 B/node on disk, against the documented 4.65-9.65 MB per 1M nodes and the 128x claim | src/ast/rmq.rs:24-54, src/ast/jump_table.rs:14-31, src/ast/rank_select.rs:23-52 |
| CORE-05 | high | flaw | partial | Silent narrowing casts cap the system at 65,535 files, 65,535 columns, 64 KB tokens, 256 blocks/method and 65,535-member SCCs | src/ingestion/mod.rs:70, src/ingestion/mod.rs:123, src/ingestion/walker.rs:54-63 |
| CORE-06 | high | flaw | partial | lca() is wrong for most queries; select1 returns 0 (root) for absent ranks and for the last node in ~1% of corpora | src/ast/rmq.rs:81-96, src/ast/rank_select.rs:86-109, src/ast/builder.rs:58-121 |
| CORE-07 | medium | flaw | partial | Artifact readers trust file-supplied lengths; Phase 2 never checks the .tca's magic, version or CRC; `inspect` claims 'VERIFIED VALID' without checking | src/ast/mod.rs:40-60, src/ast/serializer.rs:105-142, src/ingestion/serializer.rs:212-300 |
| CORE-08 | medium | flaw | partial | Safe wrapper over unsafe mmap combined with in-place truncating writes to fixed, predictable paths | src/core/io/mmap.rs:13-17, src/ingestion/serializer.rs:168-170, src/ast/serializer.rs:21 |
| CORE-09 | medium | flaw | partial | No syntax-error model, no parse timeout, recursive CST walks on the caller's stack, and a 2023-era grammar | src/ingestion/walker.rs:32-77, src/ast/reducer.rs:8-93, src/ast/adapter/java.rs:129 |
| CORE-10 | medium | flaw | partial | Each file is read three times and parsed twice, single-threaded, with no content check between phases | src/ingestion/mod.rs:50-59, src/ingestion/mod.rs:104-117, src/ast/mod.rs:136-155 |
| CORE-11 | medium | flaw | deferred-testing-phase | Artifacts are not reproducible: wall-clock time and mtimes feed every downstream hash link | src/ingestion/serializer.rs:80-84, src/ingestion/mod.rs:62-67, src/ast/mod.rs:48 |
| CORE-12 | medium | enhancement | partial | The token corpus is lossy and partly mis-typed, so it cannot serve as a faithful lexical layer | src/ingestion/adapter/java.rs:33-77, src/ingestion/adapter/mod.rs:14-26, src/ingestion/adapter/generic.rs:35-60 |
| CORE-13 | medium | flaw | partial | Logger writes every message at every level to two files under a global mutex; phase timers never fire | src/core/logger.rs:124-159, src/core/logger.rs:110-112, src/core/logger.rs:186-207 |
| CORE-14 | medium | enhancement | partial | Pipeline abstraction is decorative: the stage template has no implementors and orchestration is copy-pasted three times | src/core/pipeline.rs:34-78, src/core/pipeline.rs:97-101, src/core/pipeline.rs:143-148 |
| CORE-15 | medium | enhancement | deferred-testing-phase | Phase 1/2 tests only cover toy inputs: no BP over 512 bits, no property, fuzz, golden or performance tests | tests/ast_tests.rs:15-155, tests/ingestion_tests.rs:12-136, Cargo.toml:18-19 |
| CORE-16 | medium | flaw | partial | Source discovery follows symlinks, aborts on the first I/O error, ignores .gitignore and generated code, and neither dedups nor caps file size | src/main.rs:76-99, src/adapters/server.rs:764-830, src/ingestion/mod.rs:38-39 |
| CORE-17 | low | flaw | fixed | Unseeded FNV-1a with linear probing leaves the interner open to hash flooding | src/ingestion/interner.rs:6-13, src/ingestion/interner.rs:47-72, src/ingestion/interner.rs:127-145 |

## find:ENG (17)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| ENG-01 | critical | flaw | partial | Kotlin and 'six-language' support is illusory: Kotlin goes through the Java and JavaScript grammars, everything else through JavaScript | README.md:60, src/ingestion/adapter/kotlin.rs:29-31, src/ast/adapter/registry.rs:19-28 |
| ENG-02 | high | flaw | deferred-testing-phase | The 'F1 = 1.0000' validation is methodologically invalid and cannot be reproduced | ruthless_verify.py:88, ruthless_verify.py:232, ruthless_verify.py:299-300 |
| ENG-03 | high | flaw | partial | Use-after-free from unsafe aliasing in BDDLibrary on the production Phase 8 path; no Miri, and the docs claim forbid(unsafe_code) | src/psa/bdd/mod.rs:78-96, src/psa/bdd/mod.rs:99-110, src/psa/bdd/mod.rs:119-132 |
| ENG-04 | high | flaw | deferred-testing-phase | Integration tests mostly check that the pipeline didn't panic: tautologies, non-emptiness, prefixes and round-trips | tests/scpg_tests.rs:120, tests/scpg_tests.rs:128, tests/scpg_tests.rs:165 |
| ENG-05 | high | flaw | deferred-testing-phase | No property-based, differential, fuzz or snapshot tests; binary loaders panic or over-allocate on crafted artifacts | Cargo.toml:19-20, src/tra/serializer.rs:174-176, src/scpg/mmap/mod.rs:35-58 |
| ENG-06 | high | flaw | in-queue | Artifacts are non-deterministic (wall-clock time, file mtimes, raw paths), so the hash chain changes on every run | src/ingestion/serializer.rs:80-84, src/ingestion/mod.rs:62-67, src/ingestion/serializer.rs:115 |
| ENG-07 | high | flaw | partial | No error model: String errors, an unused thiserror, panics in Phases 7-9, silently dropped files and integer truncation | Cargo.toml:13, src/core/pipeline.rs:51, src/tra/mod.rs:41-43 |
| ENG-08 | high | flaw | partial | Headline performance claims (128x, O(1), zero-copy) are unmeasured or contradicted, and no benchmark infrastructure exists | README.md:48, README.md:54-55, README.md:59 |
| ENG-09 | high | flaw | partial | 'All 10 phases COMPLETED' and the Phase 10 readiness checklist mark stubs and missing features as done | README.md:98-106, ROADMAP.md:71, ROADMAP.md:115-116 |
| ENG-10 | high | flaw | partial | CI runs only check/fmt/test on Linux: no lint, MSRV, coverage, Miri, audit or docs gates, and the CI badge is static | .github/workflows/ci.yml:14-47, README.md:7, README.md:10-11 |
| ENG-11 | medium | flaw | in-queue | Ten-phase orchestration is hand-wired at 18 call sites through positional raw-hash parameters, and has already broken the hash chain | src/main.rs:150-255, src/core/pipeline.rs:34-78, src/core/pipeline.rs:100-168 |
| ENG-12 | medium | flaw | in-queue | The single-threaded pipeline reads every file three times, parses each twice, and logs through a logger that formats and flushes on every event | src/ingestion/mod.rs:53, src/ingestion/mod.rs:110-117, src/ast/mod.rs:142-143 |
| ENG-13 | medium | flaw | partial | Distribution is broken: the release workflow has never run and ships an .rlib; the Docker image can neither build nor start | .github/workflows/release.yml:3-28, Dockerfile:2, Dockerfile:49 |
| ENG-14 | medium | flaw | partial | Cargo.toml has a wrong description, missing metadata, overflow checks off under test, tree-sitter six minor versions behind, and an unused dependency | Cargo.toml:1-23, Cargo.lock:3, Cargo.lock |
| ENG-15 | medium | flaw | partial | Supply-chain posture fails key OpenSSF Scorecard checks (pinning, token permissions, SRI, SAST) | .github/workflows/ci.yml:21-30, .github/workflows/release.yml:13-22, .github/workflows/deploy_pages.yml:3-37 |
| ENG-16 | medium | enhancement | in-queue | Everything is public, mostly undocumented, stringly typed and in one crate; the CLI and JSON handling are hand-rolled | src/lib.rs:1-12, src/main.rs:491-508, src/main.rs:532-534 |
| ENG-17 | medium | flaw | partial | Docs and repo hygiene: a committed LLM prompt, an empty Phase 4 spec, stale paths, and false security and config statements | docs/RESEARCH_PAPER_NEXT_PROMPT.txt:1, docs/README.md:29, docs/plans/phase4.md |

## find:FLOW (21)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| FLOW-01 | critical | flaw | fixed | CFG never wires edges into or out of try and do-while; their bodies and all later code are pruned | src/cfg/builder/state.rs:76-84, src/cfg/stmts/try_stmt.rs:50-81, src/cfg/stmts/do_while.rs:24-36 |
| FLOW-02 | critical | flaw | partial | Java AST reduction (Phase 2 root cause) discards switch bodies, constructor bodies and try-with-resources before Phases 4-6 | src/ast/adapter/java.rs:129, src/ast/adapter/mod.rs:19-20, src/cfg/mod.rs:60-75 |
| FLOW-03 | critical | flaw | fixed | Loop builders choose children by position, dropping enhanced-for bodies and mis-wiring for-loops | src/cfg/stmts/for_stmt.rs:14-36, src/cfg/stmts/for_stmt.rs:88-101, src/cfg/stmts/while_stmt.rs:19-22 |
| FLOW-04 | critical | flaw | fixed | SSA keys variables by AST-node occurrence, so no φ-function or def-use edge is ever produced | src/ssa/liveness.rs:124-171, src/ssa/placement.rs:45, src/ssa/renaming.rs:146-166 |
| FLOW-05 | critical | flaw | partial | Call-target resolution returns the enclosing method, so the call graph is mostly self-loops | src/cg/resolution.rs:179-239, src/cg/call_sites.rs:70-127, src/symbol/passes/pass1_discovery.rs:139 |
| FLOW-06 | critical | flaw | partial | User-facing CFG, DFG, CDG and call-graph diagrams are hard-coded templates, not analysis output | src/scpg/diagram/export/plantuml.rs:1253-1297, src/scpg/diagram/export/plantuml.rs:1339-1398, src/scpg/diagram/export/mermaid.rs:1575-1621 |
| FLOW-07 | high | flaw | partial | Exceptional control flow and finally semantics are not modeled | src/cfg/stmts/throw_stmt.rs:20-37, src/cfg/stmts/return_stmt.rs:20-21, src/cfg/stmts/break_stmt.rs:30-31 |
| FLOW-08 | high | flaw | fixed | Labeled break/continue, synchronized, switch expressions and short-circuit operators are unsupported | src/cfg/stmts/mod.rs:35-63, src/cfg/stmts/break_stmt.rs:17-32, src/cfg/stmts/continue_stmt.rs:17-32 |
| FLOW-09 | high | flaw | fixed | CDG equals the CFG edge set because the post-dominator DFS walks the wrong adjacency | src/ssa/cdg.rs:37-61, src/ssa/cdg.rs:65-86, src/cfg/analysis/dominators.rs:21-23 |
| FLOW-10 | high | flaw | partial | The advertised IFDS solver is a three-predicate syntactic stub | src/ssa/ifds.rs:11-89, src/ssa/mod.rs:66, README.md:100 |
| FLOW-11 | high | flaw | partial | 'Andersen' points-to has no real constraints and merges SSA ids across methods | src/cg/points_to.rs:25-79, src/ssa/renaming.rs:124, src/ssa/renaming.rs:159 |
| FLOW-12 | high | flaw | partial | Dispatch model is weaker than CHA: name-only, downward-only, no RTA, initializers, lambdas or libraries | src/cg/resolution.rs:58-112, src/cg/resolution.rs:115-176, src/cg/call_sites.rs:32-35 |
| FLOW-13 | high | flaw | fixed | Linear scans over symbols and SSA records make Phases 4-6 quadratic | src/cg/call_sites.rs:70-91, src/cg/call_sites.rs:129-140, src/cg/resolution.rs:79-102 |
| FLOW-14 | high | flaw | partial | Unbounded recursion on CFG size can abort the analysis server | src/cfg/analysis/rpo.rs:7-20, src/cfg/analysis/loops.rs:29-48, src/ssa/cdg.rs:37-56 |
| FLOW-15 | high | flaw | deferred-testing-phase | Phase 4-6 tests are smoke tests that pass on these defects, and some encode them | tests/pipeline_accuracy_tests.rs:142-160, tests/pipeline_accuracy_tests.rs:222, tests/ssa_tests.rs:86 |
| FLOW-16 | high | enhancement | partial | No intermediate representation: statement-level AST blocks block precise SSA, ICFG and IFDS | src/core/types/cfg.rs:35, src/cfg/builder/state.rs:52-62, src/cfg/stmts/mod.rs:61-63 |
| FLOW-17 | medium | flaw | fixed | Loop nesting and cyclomatic complexity are miscomputed | src/cfg/analysis/loops.rs:29-90, src/cfg/stmts/continue_stmt.rs:30-31, src/cfg/builder/mod.rs:183-185 |
| FLOW-18 | medium | flaw | partial | .cfa/.ssa/.cga encodings are lossy, partly write-only and mis-indexed | src/cfg/serializer.rs:102, src/cfg/serializer.rs:147-185, src/ssa/serializer.rs:81-95 |
| FLOW-19 | medium | flaw | partial | Call-site records carry the wrong token, arity and block, and the CG spans every symbol | src/cg/call_sites.rs:142-180, src/cg/call_sites.rs:43, src/cg/mod.rs:110-128 |
| FLOW-20 | medium | flaw | partial | Kotlin (and the other advertised languages) get no CFG, SSA or call graph | src/ast/adapter/registry.rs:19-41, src/symbol/passes/pass1_discovery.rs:439-447, src/cfg/mod.rs:55-58 |
| FLOW-21 | low | enhancement | partial | Per-function phases are sequential and never incremental | src/cfg/mod.rs:39-95, src/ssa/mod.rs:46-93, docs/traceability_and_incremental.md:79 |

## find:SCPG (19)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SCPG-01 | critical | flaw | partial | Compiler-IR diagrams (CFG/DFG/CDG/call graph/ROBDD) and several views are hard-coded templates, not analysis output | src/scpg/diagram/export/json.rs:1748-1830, src/scpg/diagram/export/json.rs:1833-2100, src/scpg/diagram/export/json.rs:1511-1745 |
| SCPG-02 | high | flaw | partial | The unified .scpg stores only element counts; the 'memory-mapped' open is a full copy, and no query uses it | src/scpg/serializer/mod.rs:56-108, src/scpg/mmap/mod.rs:13-18, src/scpg/api/mod.rs:22-45 |
| SCPG-03 | high | flaw | partial | State, object, component, deployment, composite, use-case and activity views ignore UMA data and invent content | src/scpg/diagram/export/plantuml.rs:1001-1016, src/scpg/diagram/export/plantuml.rs:524-556, src/scpg/diagram/export/plantuml.rs:572-614 |
| SCPG-04 | high | flaw | partial | Exporters key classes by sanitized simple name, merging same-named types and misattaching edges | src/scpg/diagram/export/plantuml.rs:443-506, src/scpg/diagram/export/plantuml.rs:211-227, src/scpg/diagram/export/json.rs:140-146 |
| SCPG-05 | high | flaw | partial | JSON Graph IR fabricates source provenance and 'verified' metadata | src/scpg/diagram/export/json.rs:267-268, src/scpg/diagram/export/json.rs:580-581, src/scpg/diagram/export/json.rs:733-734 |
| SCPG-06 | high | flaw | partial | Generated PlantUML and Mermaid is invalid for common inputs, and nothing validates it | src/scpg/diagram/export/mermaid.rs:1536, web/diagrams/dfg.mmd, src/scpg/diagram/export/plantuml.rs:963-974 |
| SCPG-07 | high | flaw | partial | .scpg loader never verifies CRC or version and panics or aborts on corrupt or hostile files | src/scpg/mmap/mod.rs:34-35, src/scpg/mmap/mod.rs:52-72, src/scpg/mmap/mod.rs:86-89 |
| SCPG-08 | high | flaw | partial | 'Backward/forward slice' is a caller and parent-symbol closure that ignores SSA/CDG and drops nodes under the depth limit | src/scpg/query/slice.rs:1, src/scpg/query/slice.rs:12-45, src/scpg/query/slice.rs:48-74 |
| SCPG-09 | high | flaw | partial | 'CFL-reachability' is plain call-graph reachability with an O(V·E) inner loop | src/scpg/query/cfl.rs:1-35, src/core/types/cg.rs:150-153, src/core/types/cg.rs:126-138 |
| SCPG-10 | high | flaw | partial | Incremental engine is a stub, and its hash update undoes itself, so cached results can go stale | src/scpg/incremental/mod.rs:10-34, src/scpg/incremental/delta.rs:11-24, src/scpg/query/cache.rs:21-28 |
| SCPG-11 | high | flaw | partial | XMI export is a placeholder: class shells with numbered names and obsolete namespaces, unreachable from CLI and server | src/scpg/diagram/export/xmi.rs:8-19, src/scpg/diagram/factory.rs:140-148, src/main.rs:281-309 |
| SCPG-12 | high | flaw | deferred-testing-phase | Phase 10 tests are tautological, and the published diagram gallery is hand-edited rather than generated | tests/scpg_tests.rs:119-128, tests/scpg_tests.rs:168-170, tests/scpg_tests.rs:287-290 |
| SCPG-13 | medium | flaw | partial | Diagram and .scpg output differ from run to run | src/scpg/diagram/export/json.rs:131-148, src/scpg/diagram/export/json.rs:433-530, src/scpg/diagram/export/mermaid.rs:600-622 |
| SCPG-14 | medium | flaw | partial | Query cache flushes everything when full, uses a weak 32-bit XOR key, and sits behind one global Mutex | src/scpg/query/cache.rs:21-36, src/scpg/serializer/mod.rs:124-134, src/scpg/serializer/integrity.rs:18-39 |
| SCPG-15 | medium | flaw | partial | Impact, navigation and ROBDD queries miss their stated semantics and complexity | src/scpg/query/impact.rs:10-34, src/scpg/query/navigation.rs:8-27, src/scpg/query/robdd.rs:9-20 |
| SCPG-16 | medium | flaw | partial | No large-graph strategy: arbitrary first-N truncation and single giant diagrams that exceed renderer limits | src/scpg/diagram/export/plantuml.rs:925-929, src/scpg/diagram/export/plantuml.rs:958, src/scpg/diagram/export/plantuml.rs:1033-1067 |
| SCPG-17 | medium | flaw | fixed | The three exporters are divergent re-implementations; shared helpers and the edge optimizer are dead or buggy | src/scpg/diagram/common.rs:12-150, src/scpg/diagram/export/plantuml.rs:229-235, src/scpg/diagram/export/mermaid.rs:489-560 |
| SCPG-18 | medium | enhancement | partial | Class-diagram detail falls far short of UML reverse-engineering tools | src/scpg/diagram/export/plantuml.rs:335-363, src/scpg/diagram/export/plantuml.rs:483-489, src/scpg/diagram/export/json.rs:356-366 |
| SCPG-19 | medium | enhancement | partial | No standard interop or query surface (SCIP/LSIF, a query language, LSP) and no stable symbol ids | src/scpg/query/mod.rs:22-84, src/scpg/types.rs:44-59, src/main.rs:508-548 |

## find:SRV (16)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SRV-01 | critical | flaw | fixed | /api/source reads arbitrary files; basename walkdir fallback + CORS * enables cross-origin exfiltration | src/adapters/server.rs:95-240, src/adapters/server.rs:109-116, src/adapters/server.rs:200-224 |
| SRV-02 | critical | flaw | fixed | /api/analyze clones an unvalidated URL: SSRF, git argument injection, arbitrary transports, path traversal; validate_url is dead code | src/adapters/server.rs:366-465, src/adapters/server.rs:393-416, src/adapters/server.rs:429-437 |
| SRV-03 | high | flaw | fixed | Docker default command invokes a 'serve' subcommand the CLI does not implement; container exits immediately | Dockerfile:49, src/main.rs:508-548 |
| SRV-04 | high | flaw | fixed | Single-threaded serial accept loop runs the full 10-phase pipeline (and a network clone) per request with no limits | src/adapters/server.rs:40-44, src/adapters/server.rs:241-253, src/adapters/server.rs:505-699 |
| SRV-05 | high | flaw | fixed | Binds 0.0.0.0 with CORS * and no authentication or CSRF protection | src/adapters/server.rs:31-38, src/adapters/server.rs:71-74, src/adapters/server.rs:320-334 |
| SRV-06 | high | flaw | fixed | Non-compliant HTTP parsing: Content-Length ignored, body is one 8 KiB read gated on a substring, 500 ms timeout truncates, HEAD returns a body | src/adapters/server.rs:46-69, src/adapters/server.rs:241-253, src/adapters/server.rs:292-318 |
| SRV-07 | high | flaw | fixed | Silent fallback analyzes the first directory in ./target_repos, returning a different repo's results as success | src/adapters/server.rs:439-482 |
| SRV-08 | medium | flaw | fixed | validate_url panics on index [1] for URLs without a repo segment, and is never called on the clone path | src/adapters/web_repo.rs:104-118 |
| SRV-09 | medium | flaw | partial | Artifact deserializers slice fixed header offsets after only a minimal length check, panicking on truncated-but-magic-valid files | src/main.rs:347-485, src/uma/serializer.rs:238-270, src/tra/serializer.rs:114-170 |
| SRV-10 | medium | flaw | fixed | Hand-rolled CLI: silent flag dropping, positional-only port, no --version, subcommand name drift | src/main.rs:487-555, src/main.rs:532-539 |
| SRV-11 | medium | enhancement | partial | Docker image is unhardened: root user, no healthcheck, build errors swallowed, no .dockerignore, base-image/version drift | Dockerfile:1-49 |
| SRV-12 | medium | flaw | partial | No security headers on server responses; Pages app loads CDN scripts without SRI or CSP | src/adapters/server.rs:302-334, web/index.html:29-34 |
| SRV-13 | medium | flaw | fixed | Duplicated pipeline orchestration; server omits Phase 10 yet reports 'All 10 phases verified' and writes into the live served web/diagrams | src/adapters/server.rs:505-699, src/adapters/server.rs:636-693, src/main.rs:101-345 |
| SRV-14 | medium | flaw | partial | Security/config docs describe controls the code does not implement (env knobs, forbid(unsafe_code), module paths, supported versions) | .env.example:1-17, docs/security-architecture.md:9-34, src/core/logger.rs:97-108 |
| SRV-16 | medium | enhancement | deferred-testing-phase | No automated tests or lint/audit gates for the server, CLI, or clone adapter; release ships a .rlib, not the binary | .github/workflows/ci.yml:40-47, .github/workflows/release.yml:25-26, scripts/test_full_suite.py:150-165 |
| SRV-15 | low | flaw | fixed | Server leaks internal filesystem paths and raw git stderr to clients and stdout | src/adapters/server.rs:313-314, src/adapters/server.rs:446-451, src/adapters/server.rs:474-502 |

## find:SYM (17)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SYM-01 | critical | flaw | partial | Cross-file type resolution does not work: import declarations never reach Pass 2 and same-package lookup is missing | src/ast/adapter/java.rs:129, src/ast/reducer.rs:17, src/symbol/passes/pass2_imports.rs:15-18 |
| SYM-02 | critical | flaw | partial | Declaration names come from token-window heuristics and are wrong for annotated members, every parameter and long class headers | src/symbol/passes/pass1_discovery.rs:635-893, src/symbol/passes/pass1_discovery.rs:645, src/symbol/passes/pass1_discovery.rs:703 |
| SYM-03 | high | flaw | partial | Scope lookup ignores symbol kind and Java scope rules: self references bind to constructors, parameter types bind to the parameter | src/symbol/scope_graph/resolver.rs:9-32, src/symbol/passes/pass3_types.rs:47-63, src/symbol/passes/pass3_types.rs:93-106 |
| SYM-04 | high | flaw | partial | Secondary token sweep fabricates classes and fields in pure Java and misplaces Kotlin members | src/symbol/passes/pass1_discovery.rs:200-456, src/symbol/passes/pass1_discovery.rs:257, src/symbol/passes/pass1_discovery.rs:396-407 |
| SYM-05 | high | flaw | partial | Containment tree corrupted: append_child re-links already-chained symbols and orphans are parented to the first file's module | src/symbol/builder.rs:83-96, src/symbol/passes/pass1_discovery.rs:579-620, src/symbol/passes/pass1_discovery.rs:622-628 |
| SYM-06 | high | flaw | partial | Type hierarchy comes from token scans: type arguments, bounds and Kotlin property types become supertypes | src/symbol/passes/pass5_hierarchy.rs:40-74, src/symbol/passes/pass5_hierarchy.rs:76-157, src/symbol/passes/pass5_hierarchy.rs:141-146 |
| SYM-07 | high | flaw | partial | Type references are bare identifiers: generic, array, qualified and wildcard types are dropped, and type parameters are unnamed | src/ast/adapter/java.rs:83-91, src/ast/adapter/java.rs:129, src/symbol/passes/pass3_types.rs:26-36 |
| SYM-08 | high | flaw | partial | UML association inference is constant-folded and unused: every resolvable field is aggregation 0..*, and composition is unreachable | src/symbol/uml_meta/associations.rs:40-49, src/symbol/uml_meta/associations.rs:52-111, src/symbol/std_library/java_stubs.rs:57-62 |
| SYM-09 | high | flaw | partial | Quadratic loops (O(R·S), O(E²), O(C·S)) contradict the O(n_a) claim and expose the serial server to CPU exhaustion | src/symbol/passes/pass3_types.rs:108-118, src/symbol/builder.rs:112-130, src/symbol/passes/pass1_discovery.rs:373-382 |
| SYM-10 | high | enhancement | partial | No reference resolution beyond type identifiers: no expression names, calls, field accesses, inherited members or overload resolution | src/symbol/passes/pass3_types.rs:21-24, src/symbol/passes/pass1_discovery.rs:945-955, src/cg/resolution.rs:84-92 |
| SYM-11 | high | enhancement | in-queue | JDK model is 12 hand-written stubs without members; no classpath, and java.util is treated as implicitly imported | src/symbol/std_library/java_stubs.rs:31-106, src/symbol/std_library/java_stubs.rs:119-148, src/symbol/passes/pass3_types.rs:80-89 |
| SYM-12 | high | flaw | partial | Declaration coverage gaps: enum constants, anonymous classes, records, interface constants, and catch/resource/for-each/pattern variables | src/ast/adapter/java.rs:30-129, src/ast/reducer.rs:17, src/core/types/symbol.rs:17 |
| SYM-13 | high | flaw | deferred-testing-phase | No resolution-level tests or oracle; the only accuracy metric compares class-name sets | tests/symbol_tests.rs:14-175, tests/symbol_tests.rs:19-42, tests/pipeline_accuracy_tests.rs:124-126 |
| SYM-14 | medium | flaw | partial | .sta drops resolution results, is byte-nondeterministic, and uses positional symbol ids | src/symbol/serializer.rs:165-185, src/symbol/serializer.rs:138-143, src/symbol/builder.rs:30 |
| SYM-15 | medium | flaw | partial | Visibility and modifiers are probably never extracted for Java, and the defaults make package-private members public | src/ast/adapter/java.rs:139-157, src/symbol/passes/pass1_discovery.rs:895-943, src/symbol/adapter/java.rs:125-133 |
| SYM-16 | medium | flaw | in-queue | Phase 3 invariants are tautological: the data is repaired before it is verified | src/symbol/passes/pass1_discovery.rs:622-628, src/symbol/builder.rs:161-234, src/symbol/builder.rs:256-309 |
| SYM-17 | low | flaw | in-queue | Dead and misleading code in the symbol module | src/symbol/uml_meta/patterns.rs:102-137, src/symbol/qual_name_table.rs:40-47, src/symbol/scope_graph/resolver.rs:57-73 |

## find:UML (14)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| UML-01 | critical | flaw | fixed | Seven of 14 UML diagrams are templated or hard-coded, yet labeled '100% Symbol-Grounded' and verified:true | src/scpg/diagram/export/json.rs:1584-1657, src/scpg/diagram/export/json.rs:758-851, src/scpg/diagram/export/json.rs:1436-1452 |
| UML-02 | critical | flaw | partial | Field-type TH_USES edges are treated as supertypes, so nearly every class with a field is reported as Decorator and Composite | src/symbol/passes/pass5_hierarchy.rs:159-182, src/uma/patterns/decorator.rs:14-33, src/uma/patterns/composite.rs:15-34 |
| UML-03 | high | flaw | partial | Phase 9 has no working extractor for six diagram types: the state machine is a fixed template, four extractors are empty, and the object extractor misreads AST ids | src/uma/behavioral/state_machine.rs:10-69, src/uma/structural/composite_diagram.rs:9-11, src/uma/behavioral/communication_diagram.rs:9-11 |
| UML-04 | high | flaw | partial | Pattern rules are name and keyword heuristics that miss canonical GoF instances and flag unrelated classes | src/uma/patterns/singleton.rs:13-34, src/uma/patterns/observer.rs:25-29, src/uma/patterns/factory.rs:17 |
| UML-05 | high | flaw | partial | Class extractor silently drops legitimate user types through a 188-name blacklist and lexical filters | src/uma/structural/class_diagram.rs:346-538, src/uma/structural/class_diagram.rs:97-104, src/uma/structural/class_diagram.rs:106-114 |
| UML-06 | high | flaw | partial | The 'F1 = 1.0000' validation is circular and measures class names only; tests assert no ground truth for patterns or behavior | README.md:60, ruthless_verify.py:88, ruthless_verify.py:232 |
| UML-07 | high | flaw | partial | Sequence diagrams list messages in the wrong order with no nesting, returns or fragments, and all scenarios merge into one diagram | src/uma/behavioral/sequence_diagram.rs:86-163, src/uma/actor_identification.rs:24-45, src/scpg/diagram/export/plantuml.rs:1033-1066 |
| UML-08 | high | flaw | partial | Class diagrams lose multiplicity, collection element types, relationship kinds and member signatures, and collide on simple names | src/uma/structural/class_diagram.rs:210-295, src/symbol/passes/pass3_types.rs:25-31, src/symbol/passes/pass5_hierarchy.rs:184-213 |
| UML-09 | high | flaw | fixed | Quadratic and cubic scans in sequence, class, component and pattern extraction create performance cliffs on large codebases | src/uma/behavioral/sequence_diagram.rs:86-151, src/uma/structural/class_diagram.rs:283-307, src/uma/structural/class_diagram.rs:56-61 |
| UML-10 | high | flaw | partial | The .uma reader misparses the design-pattern table and desynchronizes on large records; objects and labels are never persisted | src/uma/serializer.rs:213-218, src/uma/serializer.rs:650-664, src/uma/serializer.rs:39-43 |
| UML-11 | high | enhancement | partial | Pattern output has no roles, participants, calibrated confidence or conflict resolution, and covers 11 of 23 GoF patterns | src/uma/types.rs:246-252, src/uma/types.rs:87, src/uma/patterns/mod.rs:48-190 |
| UML-12 | medium | flaw | partial | Activity diagrams discard CFG guards, exception edges, loops and labels; the label extractor reads the wrong token | src/uma/behavioral/activity_diagram.rs:66-134, src/core/types/cfg.rs:7-15, src/uma/label_extraction.rs:22-35 |
| UML-13 | medium | flaw | partial | Phase 9 output is nondeterministic across runs because HashSet/HashMap iteration order leaks into records | src/uma/structural/class_diagram.rs:250-251, src/uma/structural/class_diagram.rs:300-301, src/uma/behavioral/sequence_diagram.rs:40 |
| UML-14 | medium | enhancement | partial | Package, component and deployment views are containment counts with no dependency or build-metadata grounding | src/uma/structural/package_diagram.rs:10-58, src/uma/structural/component_diagram.rs:10-54, src/scpg/diagram/export/mermaid.rs:834-898 |

## find:WEB (16)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| WEB-01 | critical | flaw | fixed | DOM XSS: repo file names, ZIP entries and shared .openheart.json reach innerHTML | web/js/github-engine.js:123, web/js/github-engine.js:218, web/index.html:915 |
| WEB-02 | critical | flaw | fixed | Studio fabricates diagrams, source code, traceability and 'Verified' metrics | web/js/graph-canvas.js:276-300, web/js/graph-canvas.js:366-384, web/js/graph-canvas.js:476-578 |
| WEB-03 | high | flaw | partial | Real Rust engine is effectively unreachable; backend results are discarded | web/index.html:861-862, web/index.html:877-884, web/index.html:888-907 |
| WEB-04 | high | flaw | partial | In-browser analyzer builds wrong class models (member leakage, calls as methods, ID collisions) | web/js/github-engine.js:337-339, web/js/github-engine.js:434, web/js/github-engine.js:404 |
| WEB-05 | high | flaw | fixed | Cubic regex backtracking in extractMethods freezes the tab on realistic files | web/js/github-engine.js:434, web/js/github-engine.js:337-339, web/js/github-engine.js:89-94 |
| WEB-06 | high | flaw | partial | Layout is grid packing, not graph layout; sequence diagrams and large exports break | web/js/uml-layout.js:105-123, web/js/uml-layout.js:463-479, web/js/uml-layout.js:381-426 |
| WEB-07 | high | flaw | fixed | Theme toggle and overlapping async renders discard user edits and race | web/js/graph-canvas.js:721-726, web/js/graph-canvas.js:698-703, web/js/graph-canvas.js:623-646 |
| WEB-08 | high | flaw | deferred-testing-phase | No tests, lint, types, build or CI gate for roughly 10K lines of frontend | .github/workflows/ci.yml:41-47, .github/workflows/deploy_pages.yml:3-6, .github/workflows/deploy_pages.yml:24-36 |
| WEB-09 | medium | flaw | partial | Render-blocking CDN scripts without SRI or CSP; unused libraries shipped | web/index.html:29-34, web/index.html:20-22, web/index.html:6-8 |
| WEB-10 | medium | flaw | fixed | No Graph-IR validation; the bundled landing IR is internally inconsistent | web/js/graph-canvas.js:1383-1393, web/js/graph-loader.js:26-37, web/js/graph-canvas.js:1367-1371 |
| WEB-11 | medium | flaw | fixed | Undo/redo, edge filters and selection state are inconsistent | web/js/graph-canvas.js:2363-2380, web/js/graph-canvas.js:698-703, web/js/graph-canvas.js:1406-1411 |
| WEB-12 | medium | flaw | fixed | Code viewer corrupts its own highlighting and fires request storms for missing sources | web/js/editor.js:575-582, web/js/editor.js:523-558, web/js/editor.js:174-207 |
| WEB-13 | medium | flaw | fixed | GitHub ingestion ignores rate limits, truncation, slashed branches and sub-paths | web/js/github-engine.js:54-80, web/js/github-engine.js:82-84, web/js/github-engine.js:28-33 |
| WEB-14 | medium | flaw | fixed | State is scattered across globals, storage keys and a DOM text sentinel | web/js/state.js:71, web/index.html:611-636, web/index.html:980-988 |
| WEB-15 | medium | flaw | partial | Fails core WCAG 2.2 AA criteria: keyboard, focus, status messages, reflow, shortcuts | web/js/file-tree.js:207-277, web/style.css:191, web/style.css:939 |
| WEB-16 | medium | enhancement | partial | Rendering has no level-of-detail, spatial index or cache invalidation | web/js/graph-loader.js:196-221, web/js/uml-card-renderer.js:150-153, web/js/graph-canvas.js:682-684 |

## find2:BDD (47)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| BDD2-01 | high | flaw | in-queue | Entry constraint via restrict doubles every function's sat_count | src/psa/construction/mod.rs:89-97, src/psa/construction/mod.rs:106-109, src/psa/bdd/restrict.rs:8-12 |
| BDD2-02 | high | flaw | in-queue | Exit constraint is missing; loops are encoded as self-consistent cycles | src/psa/construction/constraints.rs:7, src/psa/construction/constraints.rs:52-54, src/psa/construction/mod.rs:81-97 |
| BDD2-03 | medium | flaw | in-queue | Feasibility filter cannot prune any path; its condition variables only inflate #SAT | src/psa/construction/feasibility.rs:68-75, src/psa/construction/feasibility.rs:108-124, src/psa/construction/mod.rs:99-109 |
| BDD2-04 | medium | flaw | in-queue | Recursive BDD operations can overflow the stack on large functions | src/psa/bdd/apply.rs:103-104, src/psa/bdd/apply.rs:137-138, src/psa/bdd/restrict.rs:58-66 |
| BDD2-05 | medium | flaw | in-queue | Switch exactly-one constraint is built with O(k^2) pairwise applies | src/psa/construction/constraints.rs:115-131 |
| BDD2-06 | medium | flaw | in-queue | Phase 7/8 panic instead of returning errors, which kills the HTTP server | src/psa/mod.rs:153-155, src/tra/mod.rs:42-44, src/tra/builder.rs:60-135 |
| BDD2-07 | medium | flaw | in-queue | Call-site TRA indexes are keyed by array position, not call_site_id | src/tra/backward/cs_bi.rs:12-16, src/tra/forward/cs_span.rs:18-30, src/tra/types.rs:66-71 |
| BDD2-08 | medium | flaw | in-queue | BI_ssa and BI_blk cannot be addressed: ids are function-local and there is no per-function directory | src/tra/backward/ssa_bi.rs:26-51, src/tra/backward/blk_bi.rs:12-28, src/tra/types.rs:49-64 |
| BDD2-09 | medium | enhancement | in-queue | Most TRA content is write-only: no tau/tau^-1 API and no per-layer forward indexes | src/tra/types.rs:5-20, src/tra/builder.rs:33-41, src/tra/forward/sym_span.rs:50-54 |
| BDD2-10 | medium | enhancement | in-queue | BDD library lacks exists, ITE, eval, any_sat and dag_size primitives | src/psa/bdd/mod.rs:39-175, src/psa/bdd/mod.rs:112-116, src/psa/mod.rs:14-18 |
| BDD2-11 | medium | enhancement | in-queue | SipHash maps and per-call allocations on every BDD hot path | src/psa/bdd/unique_table.rs:21, src/psa/bdd/mod.rs:36, src/psa/bdd/mod.rs:77-78 |
| BDD2-12 | medium | enhancement | deferred-testing-phase | PSA construction, ordering and builder code have no unit tests | src/psa/construction/constraints.rs, src/psa/construction/feasibility.rs, src/psa/construction/mod.rs |
| BDD2-13 | low | flaw | in-queue | XOR terminal shortcut rebuilds NOT with a fresh cache on every hit | src/psa/bdd/apply.rs:56-69, src/psa/construction/constraints.rs:81 |
| BDD2-14 | low | flaw | in-queue | apply and apply_not silently return FALSE for out-of-range node ids | src/psa/bdd/apply.rs:86-90, src/psa/bdd/apply.rs:132-134, src/psa/bdd/restrict.rs:50 |
| BDD2-15 | low | flaw | in-queue | sat_count gap factor clamps to 2^63 instead of saturating | src/psa/bdd/sat_count.rs:76-79, src/psa/bdd/sat_count.rs:87-90 |
| BDD2-16 | low | flaw | in-queue | sat_count's documented depth contract contradicts how it is called, and invalid variables pass silently | src/psa/bdd/sat_count.rs:36, src/psa/bdd/sat_count.rs:71-77, src/psa/bdd/mod.rs:135-137 |
| BDD2-17 | low | flaw | in-queue | Canonicity check omits ordering and child validity, and runs only in debug builds as a panic | src/psa/bdd/mod.rs:139-163, src/psa/construction/mod.rs:125-134, tests/psa_tests.rs:129-146 |
| BDD2-18 | low | flaw | in-queue | Three inconsistent definitions of 'terminal', and node arrays are publicly mutable | src/psa/bdd/node.rs:95-111, src/psa/bdd/mod.rs:4, src/psa/bdd/mod.rs:34 |
| BDD2-19 | low | flaw | in-queue | Wrong or misleading doc comments in the BDD primitives | src/psa/bdd/unique_table.rs:17, src/psa/bdd/unique_table.rs:25, src/psa/bdd/unique_table.rs:79 |
| BDD2-20 | low | flaw | in-queue | Public apply() memo key ignores the operation | src/psa/bdd/apply.rs:17-31, src/psa/bdd/apply.rs:80-84, src/psa/bdd/mod.rs:77-78 |
| BDD2-21 | low | flaw | in-queue | Dead or test-only PSA APIs | src/psa/bdd/unique_table.rs:70-87, src/psa/ordering/mod.rs:59-74, src/psa/ordering/mod.rs:102-106 |
| BDD2-22 | low | flaw | in-queue | No-op sifting still costs about 2n^2 swaps per function, and its driver is wrong | src/psa/ordering/sifting.rs:27-40, src/psa/ordering/sifting.rs:51-81 |
| BDD2-23 | low | enhancement | in-queue | FORCE runs a fixed number of iterations and returns the last ordering, not the best | src/psa/ordering/force.rs:24-75 |
| BDD2-24 | low | flaw | in-queue | rpo_edge_ordering is DFS edge-discovery order, not reverse postorder | src/psa/ordering/rpo.rs:1-12, src/psa/ordering/rpo.rs:31-54 |
| BDD2-25 | low | flaw | in-queue | VariableOrdering silently clamps and aliases edges beyond the u16 range | src/psa/ordering/mod.rs:39-57, src/psa/ordering/mod.rs:76-94 |
| BDD2-26 | low | flaw | in-queue | Phase 8 processes SCCs caller-first, the opposite of its comments and §8.5.1 | src/psa/mod.rs:23-24, src/psa/mod.rs:55-60, src/psa/mod.rs:98-104 |
| BDD2-27 | low | flaw | in-queue | Cyclomatic Invariant 2 is never verified; the V(G) doc and unit test are wrong | src/psa/builder.rs:41-45, src/psa/construction/mod.rs:111-113, src/psa/metrics.rs:8-10 |
| BDD2-28 | low | flaw | in-queue | PSA metrics record drops unwind_depth and stores sat_count as a 'mean' | src/psa/metrics.rs:62-98, src/psa/builder.rs:77-82, src/psa/serializer.rs:237-244 |
| BDD2-29 | low | flaw | in-queue | 'Lazy-loaded' node arrays and 'always mmap'd' metrics are not implemented | src/psa/serializer.rs:18-25, src/psa/serializer.rs:158-235, src/psa/types.rs:172-177 |
| BDD2-30 | low | flaw | in-queue | PSA and TRA readers skip version, reserved-field, sortedness and reference checks | src/psa/serializer.rs:191-197, src/psa/serializer.rs:221-235, src/psa/types.rs:113-127 |
| BDD2-31 | low | flaw | in-queue | Artifact writers are non-atomic, fully buffered and unvalidated | src/psa/serializer.rs:73-150, src/tra/serializer.rs:13-107, src/psa/types.rs:156-181 |
| BDD2-32 | low | flaw | in-queue | UMLLink col_end overflows on long lines and is wrong for multi-line tokens | src/tra/uml_link/mod.rs:44-48, src/tra/forward/sym_span.rs:27-31 |
| BDD2-33 | low | flaw | in-queue | CallSiteSpanRecord truncates lines to u16; TRA line widths are inconsistent | src/tra/forward/cs_span.rs:26-33, src/tra/types.rs:73-109, src/tra/serializer.rs:90-95 |
| BDD2-34 | low | flaw | in-queue | UML kind list is duplicated, and Invariant 2 is a count tautology that can panic | src/tra/builder.rs:74-95, src/tra/uml_link/mod.rs:22-37, src/tra/builder.rs:134 |
| BDD2-35 | low | flaw | in-queue | Composite SCPG hash XOR-combines its components and truncates to 32 bits | src/tra/uml_link/hash_chain.rs:43-44, src/tra/uml_link/hash_chain.rs:58-68, src/tra/types.rs:111-121 |
| BDD2-36 | low | flaw | in-queue | Misleading TRA names: filter_stale keeps the stale ids, and the TRA_MAGIC comment has the wrong byte order | src/tra/delta/invalidation.rs:14-21, src/tra/types.rs:3, src/tra/serializer.rs:110 |
| BDD2-37 | low | enhancement | in-queue | Phase 7/8 run independent per-function work sequentially | src/psa/mod.rs:104-147, src/tra/builder.rs:31-41 |
| BDD2-38 | low | enhancement | deferred-testing-phase | BDD primitive and PSA serializer tests cover only trivial cases | src/psa/bdd/apply.rs:145-186, src/psa/bdd/restrict.rs:76-120, src/psa/bdd/sat_count.rs:97-142 |
| BDD2-39 | low | enhancement | deferred-testing-phase | src/tra has no unit tests at all | src/tra, tests/tra_tests.rs:80-99 |
| BDD2-40 | low | flaw | deferred-testing-phase | Sort-key 'Lemma 5.1' test checks two neighbours and no boundaries | tests/research_paper_theorems_tests.rs:23-43, src/core/types/token.rs:121-124 |
| BDD2-41 | low | flaw | in-queue | Empty-function early return hard-codes metrics and ignores recursion | src/psa/construction/mod.rs:59-71 |
| BDD2-42 | low | flaw | in-queue | compute_max_path_length depth counter overflows u16 | src/psa/construction/mod.rs:159-170 |
| BDD2-43 | low | flaw | in-queue | Phase 8 ignores CFG edge types | src/psa/construction/feasibility.rs:29-31, src/psa/construction/feasibility.rs:51-60, src/psa/construction/constraints.rs:67-86 |
| BDD2-44 | low | flaw | in-queue | Feasibility scans the whole taint and SSA lists for every branch block | src/psa/construction/feasibility.rs:88-93, src/psa/construction/feasibility.rs:116-119 |
| BDD2-45 | low | flaw | in-queue | FeasibilityFilter docs describe behavior the code does not have | src/psa/construction/feasibility.rs:7-15, src/psa/construction/feasibility.rs:33-34 |
| BDD2-46 | low | enhancement | in-queue | TRA duplicates BPA token ranges and has no section directory | src/tra/backward/ast_bi.rs:9-22, src/tra/serializer.rs:16-99 |
| BDD2-47 | low | enhancement | in-queue | f_paths conjunction schedule is a left fold in block-id order | src/psa/construction/mod.rs:81-87 |

## find2:CORE (53)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| CORE2-01 | high | flaw | in-queue | Java reducer emits every variable_declarator as a second NN_LOCAL_VAR_DECL, duplicating declarations | src/ast/adapter/java.rs:40-42, src/symbol/adapter/java.rs:43-45, src/symbol/adapter/java.rs:63-65 |
| CORE2-02 | high | flaw | in-queue | Enum constants are dropped from the AST, so SK_ENUM_CONSTANT is never produced | src/ast/adapter/java.rs:35, src/ast/adapter/java.rs:112-113, src/ast/adapter/java.rs:129 |
| CORE2-03 | high | flaw | in-queue | The BP AST stores no child roles, so consumers decode children by position and misread for and enhanced-for loops | src/ast/preorder.rs:3-9, src/ast/reducer.rs:48-91, src/core/types/ast.rs:21 |
| CORE2-04 | high | flaw | in-queue | Java reducer silently drops many constructs beyond those in CORE-02 | src/ast/adapter/java.rs:30-130, src/ast/adapter/java.rs:92-101, src/ast/adapter/java.rs:124-129 |
| CORE2-05 | medium | flaw | in-queue | Anonymous class bodies and instance initializers get no declaration node | src/ast/adapter/java.rs:45, src/ast/adapter/java.rs:73, src/ast/adapter/java.rs:110 |
| CORE2-06 | medium | flaw | in-queue | A reloaded .tca cannot name its files: read() discards the paths and path_str_offset is reinterpreted | src/ingestion/serializer.rs:31-37, src/ingestion/serializer.rs:93-98, src/ingestion/serializer.rs:228 |
| CORE2-07 | medium | flaw | in-queue | The tca_hash link and Phase 2 invariants 2-5 are documented as enforced but never checked | src/ast/builder.rs:179-207, src/ast/serializer.rs:31, src/ast/serializer.rs:114 |
| CORE2-08 | medium | flaw | in-queue | Phase 2 silently skips files it cannot open or parse, and reopens paths relative to the working directory | src/ast/mod.rs:85-99, src/ast/mod.rs:136-155, src/ingestion/serializer.rs:94 |
| CORE2-09 | medium | flaw | in-queue | SCPGPipelineBuilder::execute returns Result, but phases 7 to 10 panic on I/O errors | src/core/pipeline.rs:209-225, src/tra/mod.rs:41-43, src/psa/mod.rs:154 |
| CORE2-10 | medium | flaw | in-queue | CGCSR::edges_of panics on the caller CSR because its edge_types array is always empty | src/core/types/cg.rs:126-137, src/cg/mod.rs:172-176, src/cg/serializer.rs:210-214 |
| CORE2-11 | medium | flaw | in-queue | The generic reducer erases identifiers and mis-types nodes by substring matching, even for JavaScript | src/ast/adapter/generic.rs:30-122 |
| CORE2-13 | medium | flaw | in-queue | type_parameters becomes one NN_TYPE_PARAM node and each type_parameter is dropped | src/ast/adapter/java.rs:83, src/ast/adapter/java.rs:129, src/symbol/adapter/java.rs:66 |
| CORE2-14 | medium | flaw | in-queue | select1 scans bit by bit and is O(n) for absent ranks; every navigation call pays this despite the O(1) claims | src/ast/rank_select.rs:1, src/ast/rank_select.rs:62-109, src/ast/builder.rs:58-121 |
| CORE2-30 | medium | flaw | in-queue | Console logging uses println!/eprintln!, which panic on a closed pipe, and it mixes logs into stdout | src/core/logger.rs:135-142 |
| CORE2-48 | medium | enhancement | in-queue | Phases 1 and 2 have no incremental path, although hashes are recorded and the docs promise one | src/ingestion/parser/tree_sitter.rs:22, src/ingestion/mod.rs:56-67, src/ast/builder.rs:124-141 |
| CORE2-12 | low | flaw | in-queue | NodeAttr has room for only 8 modifiers, so default, strictfp, non-sealed, varargs and Kotlin modifiers never reach symbols | src/core/types/ast.rs:186-214, src/core/types/symbol.rs:55-95, src/symbol/passes/pass1_discovery.rs:914-943 |
| CORE2-15 | low | flaw | in-queue | close_node updates every open ancestor's token range, and that result is always overwritten | src/ast/builder.rs:163-177, src/ast/reducer.rs:52-85 |
| CORE2-16 | low | flaw | in-queue | Phase 1 wraps columns above 65,535 while Phase 2 saturates them, so their sort keys disagree | src/ingestion/walker.rs:59, src/ast/reducer.rs:56-57 |
| CORE2-17 | low | flaw | in-queue | Columns are tree-sitter byte offsets with mixed bases, which is undocumented | src/ingestion/walker.rs:58-59, src/ast/reducer.rs:55-57, src/core/types/token.rs:114-124 |
| CORE2-18 | low | flaw | in-queue | Phase 2 uses positions in the sorted forward index as token ids | src/ast/reducer.rs:95-116, src/ast/mod.rs:107-127, docs/plans/phase2_ast_reduction_spec.md:656 |
| CORE2-19 | low | flaw | in-queue | Zero-width MISSING nodes become phantom tokens with empty text | src/ingestion/walker.rs:39-64 |
| CORE2-20 | low | flaw | in-queue | Language detection ignores the adapter registry: ext_map is never read and detect() is a hardcoded static | src/ingestion/adapter/registry.rs:15, src/ingestion/adapter/registry.rs:44-50, src/ingestion/adapter/registry.rs:59-83 |
| CORE2-21 | low | flaw | in-queue | The .tca string-header section is written but never used | src/ingestion/serializer.rs:144-158, src/ingestion/serializer.rs:288-301 |
| CORE2-22 | low | flaw | in-queue | TCA read() discards the filter flags, source-tree hash and timestamp; the tree hash ignores paths | src/ingestion/serializer.rs:56-62, src/ingestion/serializer.rs:215-217, src/ingestion/mod.rs:92-97 |
| CORE2-23 | low | flaw | in-queue | The TCA magic is written as a little-endian u64, so the file starts with reversed bytes | src/ingestion/serializer.rs:12, src/ingestion/serializer.rs:68, src/ast/serializer.rs:15 |
| CORE2-24 | low | flaw | in-queue | The .bpa header has a hardcoded file_count, an ignored version and no per-file root table | src/ast/serializer.rs:26, src/ast/serializer.rs:29, src/ast/serializer.rs:46 |
| CORE2-25 | low | flaw | in-queue | Phase 2 hand-parses .tca bytes with hardcoded offsets and cannot take the in-memory corpus | src/ast/mod.rs:28-32, src/ast/mod.rs:54, src/ast/mod.rs:58-127 |
| CORE2-26 | low | flaw | in-queue | Phase 2 reserves memory for 2x token_count nodes, taken from the unvalidated header | src/ast/mod.rs:107, src/ast/mod.rs:134, src/ast/builder.rs:133-141 |
| CORE2-27 | low | flaw | in-queue | The interner hashes the full text but stores a truncated copy, and its u32 counters can wrap | src/ingestion/interner.rs:47-72, src/ingestion/interner.rs:95-102 |
| CORE2-28 | low | enhancement | in-queue | Interner and string-pool layouts waste memory | src/ingestion/interner.rs:18, src/ingestion/interner.rs:30, src/ingestion/interner.rs:95-102 |
| CORE2-29 | low | flaw | in-queue | The sort key's 'reserved' flags byte silently carries a duplicate counter that wraps at 256 | src/core/types/token.rs:114-133, src/ingestion/mod.rs:140-154 |
| CORE2-31 | low | flaw | in-queue | Log files grow without bound, session IDs collide within a second, and setup errors are swallowed | src/core/logger.rs:48-80, src/core/logger.rs:97-109 |
| CORE2-32 | low | flaw | in-queue | Infallible From<u8> conversions turn corrupt bytes into plausible values | src/core/types/token.rs:20-36, src/core/types/token.rs:76-105, src/core/types/ast.rs:70-134 |
| CORE2-33 | low | flaw | in-queue | Record layout comments are wrong, and three repr(C) records have no size test | src/core/types/symbol.rs:97, src/core/types/symbol.rs:173, src/core/types/symbol.rs:212-226 |
| CORE2-34 | low | flaw | in-queue | Several TokenType and ASTNodeType variants are never produced | src/core/types/token.rs:61-68, src/core/types/ast.rs:58, src/core/types/ast.rs:60 |
| CORE2-35 | low | flaw | in-queue | Flag constants in core types are never set, so is_polymorphic() always returns false | src/core/types/cg.rs:13-18, src/core/types/cg.rs:64-66, src/core/types/ssa.rs:8-11 |
| CORE2-36 | low | flaw | in-queue | The Artifact trait is implemented only by the token corpus and fixes the file count at u16 | src/core/types/artifact.rs:1-6, src/ingestion/serializer.rs:39-51 |
| CORE2-37 | low | flaw | in-queue | The core module depends on every phase, inverting the layering | src/core/pipeline.rs:10-29, src/core/mod.rs:3-7 |
| CORE2-38 | low | flaw | in-queue | Error messages Debug-format strings and paths, which double-escapes them | src/core/pipeline.rs:173-207, src/ingestion/mod.rs:52, src/ingestion/mod.rs:54 |
| CORE2-39 | low | flaw | in-queue | The library pipeline writes graph.scpg where the CLI writes unified.scpg, and exposes no filter or override settings | src/core/pipeline.rs:168, src/core/pipeline.rs:171, src/main.rs:148 |
| CORE2-40 | low | flaw | in-queue | BinaryReader allocates before reading, and the alignment helpers divide by zero | src/core/io/binary.rs:53-61, src/core/io/binary.rs:109-125, src/ast/serializer.rs:72-196 |
| CORE2-41 | low | flaw | in-queue | Loading a .bpa rebuilds rank/select and the sparse table, and both writers buffer the whole artifact | src/ast/serializer.rs:183-184, src/ast/serializer.rs:20-70, src/ingestion/serializer.rs:63-170 |
| CORE2-42 | low | flaw | in-queue | BPASTArtifact::depth is 1-based while the builder's depth is 0-based, and four accessors are unused | src/ast/builder.rs:115-121, src/ast/builder.rs:143-145, src/ast/reducer.rs:16 |
| CORE2-43 | low | flaw | in-queue | BPASTBuilder exposes mutable internals and panics instead of returning errors | src/ast/builder.rs:124-130, src/ast/builder.rs:163-177, src/ast/builder.rs:179-207 |
| CORE2-44 | low | enhancement | in-queue | Token classification runs the same string match twice per leaf | src/ingestion/walker.rs:22, src/ingestion/walker.rs:27, src/ingestion/walker.rs:42-46 |
| CORE2-45 | low | enhancement | in-queue | tok_table_lookup binary-searches the whole corpus for every kept node | src/ast/reducer.rs:58, src/ast/reducer.rs:95-116 |
| CORE2-46 | low | enhancement | in-queue | Sources are assumed to be UTF-8, with no BOM or encoding handling | src/ingestion/mod.rs:53, src/ingestion/mod.rs:111, src/ingestion/mod.rs:144-145 |
| CORE2-47 | low | flaw | in-queue | Phase 1 records file size from metadata read before the content, and some errors omit the file path | src/ingestion/mod.rs:51-54, src/ingestion/mod.rs:75, src/ingestion/mod.rs:111 |
| CORE2-49 | low | flaw | in-queue | Low-level accessors return silent sentinel values for invalid input | src/ast/bp_encoder.rs:56-59, src/ast/rmq.rs:64-67, src/ingestion/interner.rs:104-117 |
| CORE2-50 | low | enhancement | deferred-testing-phase | The Phase 1 round-trip test compares only lengths, and there is no multi-file test | tests/ingestion_tests.rs:62-136 |
| CORE2-51 | low | enhancement | deferred-testing-phase | There are no negative tests for TokenCorpusSerializer::read or BPASTSerializer::read | src/ingestion/serializer.rs:175-210, src/ast/serializer.rs:72-102, tests/ingestion_tests.rs:1-136 |
| CORE2-52 | low | enhancement | deferred-testing-phase | The Java reduction test passes an empty token table and checks no node types | tests/ast_tests.rs:81-155 |
| CORE2-53 | low | enhancement | deferred-testing-phase | Unit tests are missing for interner growth, language detection, the caller CSR, the binary helpers and logger configuration | src/ingestion/interner.rs:154-171, src/ingestion/adapter/registry.rs:59-83, src/core/types/cg.rs:126-137 |

## find2:ENG (59)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| ENG2-01 | high | flaw | deferred-testing-phase | ruthless_verify.py reports CONVERGENCE ACHIEVED and exits 0 when nothing was verified | ruthless_verify.py, scripts/test_full_suite.py |
| ENG2-02 | medium | flaw | in-queue | generate_diagrams.py looks for its config in scripts/ and silently writes zero diagrams | scripts/generate_diagrams.py, CHANGELOG.md |
| ENG2-03 | medium | flaw | deferred-testing-phase | Ground truth and engine output are compared by simple class name only | ruthless_verify.py |
| ENG2-04 | medium | flaw | deferred-testing-phase | 'Pkg Nesting' accuracy is precision-only and computed from the wrong diagram | ruthless_verify.py |
| ENG2-05 | medium | flaw | deferred-testing-phase | Oracle regexes miss common Java and Kotlin declaration forms | ruthless_config.json, ruthless_verify.py |
| ENG2-06 | medium | flaw | deferred-testing-phase | test_diagrams_outputs_refresh.py passes the diagram section when every analysis fails | scripts/test_diagrams_outputs_refresh.py |
| ENG2-07 | medium | flaw | deferred-testing-phase | test_authored_design_patterns.py relies on vacuous and substring assertions | scripts/test_authored_design_patterns.py, scripts/test_diagrams_outputs_refresh.py |
| ENG2-08 | medium | flaw | skipped | Harness docstrings advertise checks the scripts never perform | scripts/benchmark_all_java_design_patterns.py, scripts/test_authored_design_patterns.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-09 | medium | enhancement | deferred-testing-phase | Diagram 'syntax validation' is header/footer and brace counting, not a parser | scripts/test_diagrams_outputs_refresh.py, scripts/verify_all_14_uml_diagrams.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-10 | medium | flaw | skipped | Six harness scripts copy the same client and server-bootstrap code, and the copies have drifted | scripts/benchmark_all_java_design_patterns.py, scripts/test_ruthless_popular_datasets.py, scripts/test_authored_design_patterns.py |
| ENG2-11 | medium | flaw | skipped | Harness tests whatever target/debug binary exists and waits a fixed 2 s for readiness | scripts/benchmark_all_java_design_patterns.py, scripts/test_ruthless_popular_datasets.py, scripts/test_authored_design_patterns.py |
| ENG2-12 | medium | flaw | partial | Scripts SIGKILL any process listening on the benchmark port | scripts/restart_server.sh, scripts/benchmark_all_java_design_patterns.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-13 | medium | flaw | partial | The Python harness and the web studio have no CI at all | .github/workflows/ci.yml, .github/workflows/deploy_pages.yml, ruthless_verify.py |
| ENG2-14 | medium | flaw | skipped | Pages deploy runs on every push to main regardless of CI status | .github/workflows/deploy_pages.yml |
| ENG2-15 | medium | flaw | skipped | Release workflow lacks contents: write and uses the deprecated action-gh-release@v1 | .github/workflows/release.yml |
| ENG2-16 | medium | flaw | in-queue | Unseeded FNV-1a with low-bit indexing makes string interning hash-floodable | docs/technical-decisions.md, src/ingestion/interner.rs |
| ENG2-17 | medium | flaw | in-queue | security-architecture.md has no threat model for the server, git clone or web studio | docs/security-architecture.md, src/adapters/server.rs, src/adapters/web_repo.rs |
| ENG2-18 | medium | flaw | in-queue | The 12 HTML specs under docs/ are chat-widget fragments that break when opened standalone | docs/plans/phase8_robdd_path_summaries.html, docs/plans/phase3_symbol_table_architecture.html, docs/architecture/scpg_architecture_diagram.html |
| ENG2-19 | medium | flaw | in-queue | The research paper cannot compile because its bibliography file is missing | docs/openheart_research_paper.tex |
| ENG2-20 | medium | flaw | in-queue | The paper's Limitations section cites mitigations the code does not implement | docs/openheart_research_paper.tex, src/psa/construction/recursive.rs, src/core/types/cg.rs |
| ENG2-21 | low | flaw | skipped | ruthless_verify.py ignores ignore_dirs and overrides package_stmt_patterns from its config | ruthless_verify.py, ruthless_config.json |
| ENG2-22 | low | flaw | skipped | ruthless_verify.py re-slices the file prefix for every regex match | ruthless_verify.py |
| ENG2-23 | low | flaw | deferred-testing-phase | Pass thresholds differ by script and contradict the printed 'F1=1.0' message | ruthless_verify.py, scripts/test_ruthless_popular_datasets.py, scripts/benchmark_all_java_design_patterns.py |
| ENG2-24 | low | flaw | deferred-testing-phase | test_full_suite.py matches test counts as substrings and can crash on error paths | scripts/test_full_suite.py |
| ENG2-25 | low | flaw | deferred-testing-phase | test_full_suite.py requires the legacy files and the wildcard CORS policy | scripts/test_full_suite.py |
| ENG2-26 | low | flaw | skipped | Harness output prints fixed success figures regardless of outcome | scripts/benchmark_all_java_design_patterns.py, scripts/test_full_suite.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-27 | low | flaw | deferred-testing-phase | Functions named test_* return booleans, so pytest would report them as passing | scripts/test_ruthless_popular_datasets.py, scripts/test_diagrams_outputs_refresh.py, scripts/test_full_suite.py |
| ENG2-28 | low | flaw | in-queue | The Python harness cannot run on Windows, the maintainer's platform | ruthless_verify.py, scripts/test_full_suite.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-29 | low | flaw | partial | Python scripts have unused imports, broad excepts and shell=True with unquoted paths | scripts/benchmark_all_java_design_patterns.py, ruthless_verify.py, scripts/test_ruthless_popular_datasets.py |
| ENG2-30 | low | flaw | deferred-testing-phase | test_ruthless_popular_datasets.py fails silently and runs a CLI analysis it never checks | scripts/test_ruthless_popular_datasets.py |
| ENG2-31 | low | flaw | deferred-testing-phase | test_authored_design_patterns.py analyses an untracked copy of a corpus that is in the repo | scripts/test_authored_design_patterns.py, examples/README.md |
| ENG2-32 | low | flaw | in-queue | restart_server.sh and generate_diagrams.py have small robustness bugs | scripts/restart_server.sh, scripts/generate_diagrams.py |
| ENG2-33 | low | flaw | skipped | CI cache key ignores the compiler version and is never refreshed | .github/workflows/ci.yml |
| ENG2-34 | low | flaw | skipped | CI job ordering and matrix settings waste time and let beta block merges | .github/workflows/ci.yml |
| ENG2-35 | low | flaw | partial | No build passes --locked, so Cargo.lock can be silently re-resolved | .github/workflows/ci.yml, .github/workflows/release.yml, Makefile |
| ENG2-36 | low | flaw | skipped | Dependabot does not group the ABI-coupled tree-sitter crates or watch the Dockerfile | .github/dependabot.yml |
| ENG2-37 | low | flaw | fixed | Makefile .PHONY, help and docs targets are incomplete or awkward | Makefile |
| ENG2-38 | low | flaw | fixed | The dev profile compiles at opt-level 2 and the release profile is untuned | Cargo.toml |
| ENG2-39 | low | flaw | fixed | .editorconfig gives .yml files a 4-space indent and puts spaces in Makefile recipes | .editorconfig |
| ENG2-40 | low | flaw | fixed | .gitattributes forces LF on run.bat and marks only one artifact type as binary | .gitattributes |
| ENG2-41 | low | flaw | partial | .gitignore negations never apply and several patterns over-match | .gitignore, examples/README.md |
| ENG2-42 | low | flaw | in-queue | security-architecture.md calls CRC-64 tamper protection and misstates the interner bounds | docs/security-architecture.md, src/ingestion/interner.rs, src/ingestion/mod.rs |
| ENG2-43 | low | flaw | in-queue | README and codebase-guide list artifact magic headers that do not exist | README.md, docs/codebase-guide.md, src/core/io/binary.rs |
| ENG2-44 | low | flaw | in-queue | Docs claim CRC-64 on all ten artifacts, but .scpg uses CRC-32 and inspect skips verification | README.md, docs/plans/10_phase_pipeline_overview.md, src/main.rs |
| ENG2-45 | low | flaw | in-queue | Docs give contradictory token and traceability record layouts | CHANGELOG.md, docs/traceability_and_incremental.md, docs/succinct_structures.md |
| ENG2-46 | low | flaw | in-queue | The 'cache-line aligned' TokenRecord and 'hardware-friendly' CRC claims are inaccurate | docs/technical-decisions.md, CHANGELOG.md, docs/overview.md |
| ENG2-47 | low | flaw | in-queue | overview.md and architecture.md define the layers as both disjoint and strict subsets | docs/overview.md, docs/architecture.md, docs/traceability_and_incremental.md |
| ENG2-48 | low | flaw | in-queue | architecture.md lists ROBDD and BP operations that do not exist | docs/architecture.md, docs/succinct_structures.md, src/psa/bdd/node.rs |
| ENG2-49 | low | flaw | in-queue | succinct_structures.md uses the wrong probe-count formula and a wrong baseline size | docs/succinct_structures.md |
| ENG2-50 | low | flaw | in-queue | The Phase 2 spec understates its sparse-table size by about 140 MB | docs/plans/phase2_ast_reduction_spec.md, docs/plans/phase1_ingestion_spec.md |
| ENG2-51 | low | flaw | in-queue | The codebase-guide.md module map is stale in many places | docs/codebase-guide.md, examples/README.md |
| ENG2-52 | low | flaw | in-queue | Docs name a Monaco editor that is not used and disagree on diagram numbering | README.md, CHANGELOG.md, docs/architecture.md |
| ENG2-53 | low | flaw | in-queue | Getting Started step 5 serves the studio where its API calls cannot reach the engine | docs/getting_started.md, web/index.html, web/js/editor.js |
| ENG2-54 | low | flaw | in-queue | uml_derivation.md gives wrong or missing derivation rules | docs/uml_derivation.md |
| ENG2-55 | low | flaw | in-queue | CHANGELOG has no Unreleased section or compare links and cites untraceable 'Issues 1-100' | CHANGELOG.md, ROADMAP.md |
| ENG2-56 | low | flaw | skipped | Issue and PR templates describe an obsolete 5-phase pipeline and lack key fields | .github/ISSUE_TEMPLATE/feature_request.md, .github/ISSUE_TEMPLATE/bug_report.md, .github/PULL_REQUEST_TEMPLATE.md |
| ENG2-57 | low | flaw | in-queue | The phase plans are the only file-format specs, and they have drifted from the code with no versioning | docs/plans/phase10.md, docs/plans/10_phase_pipeline_overview.md, docs/plans/phase6.md |
| ENG2-58 | low | enhancement | deferred-testing-phase | The scoring helpers have no unit tests | ruthless_verify.py, scripts/test_authored_design_patterns.py |
| ENG2-59 | low | enhancement | in-queue | The ADR log covers four micro-decisions and leaves out the major ones | docs/technical-decisions.md |

## find2:FLOW (49)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| FLOW2-01 | high | flaw | in-queue | Empty then/else and loop bodies become dead ends that the builder wires to EXIT | src/cfg/stmts/if_stmt.rs:37-42, src/cfg/stmts/if_stmt.rs:52-58, src/cfg/stmts/while_stmt.rs:41-43 |
| FLOW2-02 | high | flaw | in-queue | Def extraction ignores ++/--, compound and chained assignments, and multi-declarator lists | src/ssa/liveness.rs:130-158 |
| FLOW2-03 | high | flaw | in-queue | Use extraction stops one operand deep and skips declaration initializers | src/ssa/liveness.rs:130-158 |
| FLOW2-04 | high | flaw | in-queue | No entry definitions for parameters, this, captured variables or fields | src/ssa/renaming.rs:43-61, src/ssa/renaming.rs:149-154, src/ssa/renaming.rs:176-184 |
| FLOW2-05 | medium | flaw | in-queue | Non-variable identifiers and nested closure bodies are counted as uses/defs of the enclosing statement | src/ssa/liveness.rs:159-170, src/cfg/stmts/break_stmt.rs:14, src/cfg/stmts/continue_stmt.rs:14 |
| FLOW2-06 | medium | flaw | in-queue | φ order and SSA numbering depend on HashMap iteration order | src/ssa/placement.rs:25-39, src/ssa/placement.rs:52-56, src/ssa/renaming.rs:30-33 |
| FLOW2-07 | medium | flaw | in-queue | φ operands are recorded as anonymous u32::MAX uses in the def-use CSR | src/ssa/renaming.rs:150-153, src/ssa/renaming.rs:176-184, src/core/types/ssa.rs:79-98 |
| FLOW2-08 | medium | flaw | in-queue | Switch case-label detection marks every case group as default | src/cfg/stmts/switch_stmt.rs:48-68, src/cfg/stmts/switch_stmt.rs:85-87, src/ast/adapter/java.rs:53 |
| FLOW2-09 | medium | flaw | in-queue | Catch bodies collapse into one opaque statement because the catch parameter is dropped | src/cfg/stmts/try_stmt.rs:63-67, src/cfg/stmts/try_stmt.rs:84-87, src/ast/adapter/java.rs:129 |
| FLOW2-10 | medium | flaw | in-queue | Enhanced-for loop variable is defined once in the pre-header | src/cfg/stmts/for_stmt.rs:103-118 |
| FLOW2-11 | medium | flaw | in-queue | while(true) gets a spurious exit edge, inconsistent with for(;;) | src/cfg/stmts/while_stmt.rs:49, src/cfg/stmts/for_stmt.rs:76-78 |
| FLOW2-12 | medium | flaw | in-queue | Expression-bodied lambdas, field initializers and instance initializers get no CFG | src/cfg/mod.rs:46-76 |
| FLOW2-13 | medium | flaw | in-queue | Points-to allocation seeding can never fire | src/cg/points_to.rs:28-44, src/cg/resolution.rs:152-157, src/ssa/renaming.rs:161 |
| FLOW2-14 | medium | flaw | in-queue | Lambda creation sites become lambda-to-lambda self-loops marked self-recursive | src/cg/call_sites.rs:23-35, src/cg/call_sites.rs:70-82, src/cg/resolution.rs:169-175 |
| FLOW2-15 | medium | flaw | in-queue | call_site_id no longer equals vector position after an unstable sort | src/cg/call_sites.rs:45-66, src/tra/backward/cs_bi.rs:12-16, src/tra/forward/cs_span.rs:18-31 |
| FLOW2-16 | medium | flaw | in-queue | CGCSR::edges_of panics on the caller CSR | src/core/types/cg.rs:127-137, src/cg/mod.rs:172-176, src/cg/serializer.rs:210-214 |
| FLOW2-17 | medium | flaw | in-queue | .cga deserializer pre-allocates from untrusted counts | src/cg/serializer.rs:158-171, src/cg/serializer.rs:174-189, src/cg/serializer.rs:198-246 |
| FLOW2-18 | medium | flaw | in-queue | classify_call never emits INTERFACE and dispatches super calls virtually | src/cg/call_sites.rs:102-123 |
| FLOW2-19 | medium | flaw | in-queue | Hot-loop trace logging is always formatted and flushed to two log files | src/cg/call_sites.rs:57-60, src/cfg/mod.rs:78-92, src/ssa/mod.rs:47-51 |
| FLOW2-20 | medium | flaw | in-queue | More quadratic scans in Phases 4-6 that FLOW-13 does not list | src/cg/resolution.rs:132-143, src/cg/resolution.rs:152-163, src/cg/points_to.rs:46-53 |
| FLOW2-41 | medium | enhancement | in-queue | No heap or field SSA (Memory SSA) | src/ssa/liveness.rs:145-158, src/ssa/renaming.rs:142-167, src/core/types/ssa.rs:9 |
| FLOW2-45 | medium | enhancement | deferred-testing-phase | Add golden CFG tests for each statement builder | tests/cfg_tests.rs:16-100, src/cfg/stmts/if_stmt.rs:9, src/cfg/stmts/while_stmt.rs:9 |
| FLOW2-46 | medium | enhancement | deferred-testing-phase | Add unit and property tests for the graph algorithms | tests/cfg_tests.rs:102-129, src/cfg/analysis/dominators.rs:8, src/cfg/analysis/frontier.rs:3 |
| FLOW2-47 | medium | enhancement | deferred-testing-phase | Add SSA construction tests | tests/ssa_tests.rs:16-93, src/ssa/liveness.rs:124, src/ssa/placement.rs:16 |
| FLOW2-48 | medium | enhancement | deferred-testing-phase | Add call-graph and SCC tests | tests/cg_tests.rs:16-88, src/cg/call_sites.rs:14, src/cg/resolution.rs:30 |
| FLOW2-21 | low | flaw | in-queue | Branch edge types are inconsistent across loop and switch builders | src/cfg/stmts/do_while.rs:42-43, src/cfg/stmts/switch_stmt.rs:85-87, src/cfg/stmts/for_stmt.rs:59 |
| FLOW2-22 | low | flaw | in-queue | Empty-statement then-branch drops the condition or swaps branches | src/cfg/stmts/if_stmt.rs:15-25 |
| FLOW2-23 | low | flaw | in-queue | return and throw register both the expression and the statement | src/cfg/stmts/return_stmt.rs:14-17, src/cfg/stmts/throw_stmt.rs:14-17 |
| FLOW2-24 | low | flaw | in-queue | assert statements and ternary expressions are treated as straight-line code | src/cfg/stmts/mod.rs:61-63, src/ast/adapter/java.rs:62, src/ast/adapter/java.rs:71 |
| FLOW2-25 | low | flaw | in-queue | Pending-exit list is global and is flushed or drained at inconsistent points | src/cfg/stmts/if_stmt.rs:36, src/cfg/stmts/if_stmt.rs:51, src/cfg/stmts/while_stmt.rs:25 |
| FLOW2-26 | low | flaw | in-queue | CDG labels every non-False dependence TRUE and has no ENTRY augmentation | src/ssa/cdg.rs:65-79, src/cfg/builder/mod.rs:79-86 |
| FLOW2-27 | low | flaw | in-queue | 1-CFA results replace CHA instead of refining it; a missing symbol falls back to name_id 0 | src/cg/resolution.rs:35-41, src/cg/resolution.rs:126-129 |
| FLOW2-28 | low | flaw | in-queue | CHA and 1-CFA keep bodiless interface methods as concrete targets | src/cg/resolution.rs:89, src/cg/resolution.rs:138, src/symbol/passes/pass1_discovery.rs:924-925 |
| FLOW2-29 | low | flaw | in-queue | Duplicate caller-to-callee pairs keep only the first edge kind | src/cg/mod.rs:114-128 |
| FLOW2-30 | low | flaw | in-queue | Artifact readers skip version and structural validation | src/cg/serializer.rs:147-263, src/cfg/serializer.rs:28, src/cfg/serializer.rs:168 |
| FLOW2-31 | low | flaw | in-queue | Magic comments do not match the bytes on disk, and .cfa u32 arrays are unaligned | src/cfg/serializer.rs:9, src/cg/serializer.rs:11, src/cfg/serializer.rs:123-129 |
| FLOW2-32 | low | flaw | in-queue | Artifact writes are not atomic and buffer the whole file in memory | src/cfg/serializer.rs:191-197, src/ssa/serializer.rs:58-70, src/ssa/serializer.rs:163 |
| FLOW2-33 | low | flaw | in-queue | Misleading algorithm names, invariant logs and missing Phase 6 timer | src/cg/points_to.rs:1-11, src/cg/resolution.rs:1, src/cg/resolution.rs:114-115 |
| FLOW2-34 | low | flaw | in-queue | Phase 4-6 invariant checks are mostly vacuous, yet any failure aborts the whole run | src/cfg/mod.rs:84, src/cfg/mod.rs:112-142, src/ssa/mod.rs:81 |
| FLOW2-35 | low | flaw | in-queue | Silent integer truncation in metrics, versions and offsets | src/cfg/builder/mod.rs:183-185, src/ssa/renaming.rs:27, src/ssa/renaming.rs:135 |
| FLOW2-36 | low | flaw | in-queue | Liveness fixpoint stops silently after 100 iterations | src/ssa/liveness.rs:62-100 |
| FLOW2-37 | low | flaw | in-queue | Recursive AST walks in CFG construction and def/use extraction | src/cfg/stmts/mod.rs:28-65, src/cfg/stmts/if_stmt.rs:34, src/cfg/stmts/if_stmt.rs:49 |
| FLOW2-38 | low | flaw | in-queue | Core type docs overstate layouts and leave sentinels undocumented | src/core/types/ssa.rs:17-26, src/core/types/cg.rs:25, src/core/types/cg.rs:69 |
| FLOW2-39 | low | flaw | in-queue | IFDSResults taint tuple means different things to its producer and its consumer | src/ssa/ifds.rs:24-26, src/core/types/ssa.rs:108-114, src/psa/construction/feasibility.rs:86-93 |
| FLOW2-40 | low | flaw | in-queue | Dead code and unused results across Phases 4-6 | src/cfg/builder/mod.rs:79-86, src/cfg/builder/state.rs:133-142, src/cfg/analysis/frontier.rs:5 |
| FLOW2-42 | low | enhancement | in-queue | No block-merging pass, so CFGs carry many empty pass-through blocks | src/cfg/builder/mod.rs:88-142, src/cfg/stmts/if_stmt.rs:68, src/cfg/stmts/while_stmt.rs:31 |
| FLOW2-43 | low | enhancement | in-queue | Pruned blocks that still contain statements are discarded silently | src/cfg/builder/mod.rs:91-126 |
| FLOW2-44 | low | enhancement | in-queue | Add zero-copy CFG accessors and cache per-statement def/use sets | src/ssa/liveness.rs:112-122, src/ssa/renaming.rs:213-223, src/ssa/placement.rs:75-85 |
| FLOW2-49 | low | enhancement | deferred-testing-phase | Add serializer robustness tests | tests/cfg_tests.rs:93-99, tests/ssa_tests.rs:88-92, tests/cg_tests.rs:83-87 |

## find2:SCPG (38)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SCPG2-01 | medium | flaw | in-queue | Phase10Stage::run panics on I/O failure instead of returning Result | src/scpg/mod.rs, src/core/pipeline.rs, src/main.rs |
| SCPG2-02 | medium | flaw | in-queue | scpg_hash changes on every run, so it is not a content fingerprint | src/scpg/serializer/mod.rs, src/ast/mod.rs, src/ingestion/serializer.rs |
| SCPG2-03 | medium | flaw | in-queue | Two incompatible scpg_hash definitions (Phase 7 links vs Phase 10 header) | src/scpg/serializer/integrity.rs, src/scpg/types.rs, src/tra/uml_link/hash_chain.rs |
| SCPG2-04 | medium | flaw | in-queue | Phase 10 never verifies that the nine input artifacts come from the same build | src/scpg/serializer/mod.rs, src/scpg/mod.rs |
| SCPG2-05 | medium | flaw | in-queue | Query cache key ignores the artifacts passed with each call | src/scpg/query/mod.rs, src/scpg/api/mod.rs |
| SCPG2-06 | medium | flaw | partial | Header source_hash is always zero, so a stale .scpg cannot be detected | src/scpg/serializer/mod.rs, src/ingestion/mod.rs, docs/plans/phase10.md |
| SCPG2-07 | medium | flaw | partial | Comment and whitespace edits are classified as needing no rebuild, which leaves positions stale | src/scpg/incremental/mod.rs, docs/plans/phase10.md |
| SCPG2-08 | medium | enhancement | in-queue | Engine exposes only 2 of the specified queries; the other query modules have no callers | src/scpg/api/mod.rs, src/scpg/query/mod.rs, src/scpg/query/impact.rs |
| SCPG2-09 | medium | enhancement | deferred-testing-phase | End-to-end SCPG test checks only output prefixes and a constant banner | tests/scpg_tests.rs, src/scpg/diagram/mod.rs |
| SCPG2-10 | medium | enhancement | deferred-testing-phase | No tests for the .scpg format: section CRCs, header fields, malformed files | tests/scpg_tests.rs, src/scpg/serializer/mod.rs, src/scpg/serializer/integrity.rs |
| SCPG2-11 | medium | enhancement | deferred-testing-phase | Query engine, cache, incremental planner and most query modules have no tests | src/scpg/query/mod.rs, src/scpg/query/cache.rs, src/scpg/query/impact.rs |
| SCPG2-12 | medium | enhancement | deferred-testing-phase | Surgical test advertises inheritance, dispatch and recursion but asserts none of them | tests/surgical_pipeline_dump_test.rs |
| SCPG2-13 | low | flaw | in-queue | Composite hash arguments are shifted one phase upstream and omit CGA, PSA and UMA content | src/scpg/serializer/mod.rs, src/scpg/serializer/integrity.rs |
| SCPG2-14 | low | flaw | in-queue | A poisoned query mutex silently turns into 'unreachable' and empty slices | src/scpg/api/mod.rs |
| SCPG2-15 | low | flaw | in-queue | language_count header field is hardcoded to 1 | src/scpg/serializer/mod.rs, src/core/types/token.rs |
| SCPG2-16 | low | flaw | in-queue | The .scpg write is non-atomic, unsynced and fully buffered in memory | src/scpg/serializer/mod.rs |
| SCPG2-17 | low | flaw | in-queue | classify() trusts three independent booleans and resolves conflicts to the cheapest scope | src/scpg/incremental/mod.rs, src/scpg/incremental/delta.rs |
| SCPG2-18 | low | flaw | in-queue | Query-type codes are magic numbers that contradict the spec; params_crc is not a CRC | src/scpg/query/mod.rs, src/scpg/types.rs |
| SCPG2-19 | low | flaw | in-queue | EngineBuilder cannot build anything and duplicates SCPGPipelineBuilder | src/scpg/api/builder.rs, src/scpg/mod.rs, src/core/pipeline.rs |
| SCPG2-20 | low | flaw | in-queue | Engine export methods ignore engine state, expose only class diagrams, and hide errors | src/scpg/api/mod.rs |
| SCPG2-21 | low | flaw | in-queue | Dead code and a capacity-0 edge case in the query layer | src/scpg/types.rs, src/scpg/query/cache.rs |
| SCPG2-22 | low | flaw | in-queue | CFL search keeps a redundant pair state and an unreachable final check | src/scpg/query/cfl.rs |
| SCPG2-23 | low | flaw | in-queue | Slice traversals scan the whole site_to_edge_map for every visited symbol | src/scpg/query/slice.rs, src/core/types/cg.rs |
| SCPG2-24 | low | flaw | in-queue | from_source ignores columns and ordering; to_source returns only the first link | src/scpg/query/navigation.rs |
| SCPG2-25 | low | flaw | in-queue | ROBDD lookups return a fabricated 1 for unknown or non-function symbols | src/scpg/query/robdd.rs |
| SCPG2-26 | low | enhancement | in-queue | Sections are unaligned, and count widths vary without type metadata | src/scpg/serializer/mod.rs, src/scpg/types.rs |
| SCPG2-27 | low | flaw | in-queue | StringTable section omits the interner's offset index | src/scpg/serializer/mod.rs, src/ingestion/interner.rs |
| SCPG2-28 | low | flaw | in-queue | Header and directory layout is duplicated as magic offsets, with no codec or checksum | src/scpg/types.rs, src/scpg/serializer/mod.rs, src/scpg/mmap/mod.rs |
| SCPG2-29 | low | flaw | partial | Bitwise CRC-32 is duplicated and slow, and breaks the CRC-64 convention | src/scpg/serializer/integrity.rs, src/tra/uml_link/hash_chain.rs, README.md |
| SCPG2-30 | low | flaw | in-queue | Debug output of MemoryMappedSCPG prints the entire file | src/scpg/mmap/mod.rs |
| SCPG2-31 | low | flaw | partial | Documented SCPG magic number disagrees with the constant, and the magic embeds a version | src/scpg/types.rs, src/scpg/mmap/mod.rs, docs/plans/phase10.md |
| SCPG2-32 | low | flaw | in-queue | Cache hits deep-clone results | src/scpg/query/cache.rs, src/scpg/query/mod.rs |
| SCPG2-33 | low | enhancement | in-queue | Query answers lack witness paths, distances and edge-kind filters | src/scpg/query/cfl.rs, src/scpg/query/impact.rs, src/scpg/query/slice.rs |
| SCPG2-34 | low | flaw | deferred-testing-phase | Surgical test prints 'ALL FIELDS MATCH' after comparing only lengths | tests/surgical_pipeline_dump_test.rs |
| SCPG2-35 | low | enhancement | deferred-testing-phase | Surgical pipeline test stops at Phase 7 | tests/surgical_pipeline_dump_test.rs |
| SCPG2-36 | low | flaw | deferred-testing-phase | Surgical test output is nondeterministic, slightly wrong, and floods CI logs | tests/surgical_pipeline_dump_test.rs, .github/workflows/ci.yml |
| SCPG2-37 | low | enhancement | deferred-testing-phase | SCPG test hand-rolls the 10-phase harness in one monolithic test | tests/scpg_tests.rs, tests/surgical_pipeline_dump_test.rs |
| SCPG2-38 | low | flaw | in-queue | Phase 10 log lines and module docs describe behavior that does not exist | src/scpg/mod.rs, src/scpg/serializer/integrity.rs, src/scpg/api/builder.rs |

## find2:SCPGX (46)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SCPGX2-01 | high | flaw | in-queue | Package diagrams mix segment names with full paths, adding stray root packages | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-02 | high | flaw | in-queue | Primitive and JDK member types render blank or 'Unknown' in every format | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-03 | high | flaw | in-queue | Interaction overview resolves scenario_name (an interner id) as a symbol id | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-04 | high | flaw | in-queue | Collection-typed fields never yield associations to their element type | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-05 | medium | flaw | in-queue | PlantUML class diagram keeps only one relationship per ordered class pair | src/scpg/diagram/export/plantuml.rs |
| SCPGX2-06 | medium | flaw | in-queue | Method return types are drawn as associations, and JSON doubles field edges | src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-07 | medium | flaw | in-queue | JSON detects aggregation by substring of the target class name | src/scpg/diagram/export/json.rs |
| SCPGX2-08 | medium | flaw | in-queue | Unresolved types and constructors render inconsistently; JSON invents Object/void | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-09 | medium | flaw | in-queue | Name-based 'primitive/system' filters silently drop user-defined classes | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/common.rs |
| SCPGX2-10 | medium | flaw | in-queue | The external actor splits into two entities and loses its actor role | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-11 | medium | flaw | in-queue | Mermaid 'Unknown'/'Empty' sentinels make every empty-name guard dead | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-12 | medium | flaw | in-queue | Sequence exports misrepresent call structure and lose scenario identity | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-13 | medium | flaw | in-queue | Communication diagrams are built wrongly in all three formats | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-14 | medium | flaw | in-queue | Empty-data fallbacks fabricate interactions | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-15 | medium | flaw | in-queue | JSON IR emits duplicate node ids for same-named functions and components | src/scpg/diagram/export/json.rs |
| SCPGX2-16 | medium | flaw | in-queue | Diagram-type keys and aliases differ by format; JSON silently falls back to the class diagram | src/scpg/diagram/factory.rs, src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-17 | medium | enhancement | in-queue | Exporter interfaces receive only uma/sta/tca, so IR diagrams cannot use CFA/SSA/CGA/PSA | src/scpg/diagram/factory.rs, src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-18 | medium | flaw | in-queue | Package dependencies use only field and return types, and are labelled «imports» | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-19 | medium | flaw | in-queue | Mermaid package diagram is flat and edge-less; the dead nested renderer would hide com/org | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-20 | medium | flaw | in-queue | Mermaid duplicate-package disambiguation hardcodes frontend/backend and is not unique | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-21 | medium | flaw | in-queue | Raw identifiers become Mermaid ids without reserved-word handling | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-22 | medium | enhancement | in-queue | JSON Graph IR has no version or schema, uses free-form kinds, and bloats output | src/scpg/diagram/export/json.rs |
| SCPGX2-23 | medium | enhancement | deferred-testing-phase | No unit or property tests cover the exporter logic | src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/plantuml.rs |
| SCPGX2-24 | low | flaw | in-queue | Mermaid interaction overview ids collide across scenarios, and there are no flow edges | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-25 | low | flaw | in-queue | Mermaid activity diagram leaves the start node unconnected and ignores node/edge kinds | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-26 | low | flaw | in-queue | PlantUML output hard-codes a dark theme | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-27 | low | flaw | in-queue | Free-text labels go through the identifier sanitizer | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-28 | low | flaw | in-queue | Profile diagram uses wrong UML notation and invents metaclass links | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-29 | low | flaw | in-queue | pattern_kind is truncated from u16 to u8 | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-30 | low | flaw | in-queue | Package-resolution loops hang on cyclic parent_sym chains | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-31 | low | flaw | in-queue | Mermaid resolve_name leaks memory through Box::leak | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-32 | low | flaw | in-queue | Mermaid edge placement and package fallback are super-linear | src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/factory.rs |
| SCPGX2-33 | low | flaw | in-queue | PlantUML name fallback borrows a nearby token's text for unnamed symbols | src/scpg/diagram/export/plantuml.rs |
| SCPGX2-34 | low | flaw | in-queue | DiagramGenerator/DiagramRenderers names and summary text misstate what they do | src/scpg/diagram/mod.rs, src/scpg/diagram/renderers.rs |
| SCPGX2-35 | low | flaw | in-queue | Module and section doc comments claim properties the code lacks | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-36 | low | flaw | in-queue | Dead code hidden by #[allow(dead_code)], unused parameters and redundant checks | src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/common.rs |
| SCPGX2-37 | low | flaw | in-queue | DiagramUtils has latent bugs that would surface if exporters are moved onto it | src/scpg/diagram/common.rs |
| SCPGX2-38 | low | flaw | in-queue | PlantUMLOptimizer drops inheritance edges it claims to preserve | src/scpg/diagram/export/plantuml_optimizer.rs |
| SCPGX2-39 | low | flaw | in-queue | Sequence and timing participant order follows HashMap iteration | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-40 | low | flaw | in-queue | Nested classes are rendered with three different relationship meanings | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-41 | low | flaw | in-queue | Record and annotation types lose their kind | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-42 | low | flaw | in-queue | Mermaid package tree collapses repeated consecutive segments | src/scpg/diagram/export/mermaid.rs |
| SCPGX2-43 | low | flaw | in-queue | Package ids and labels are generated inconsistently | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs |
| SCPGX2-44 | low | enhancement | in-queue | Design-pattern results are rendered without confidence and one pattern per class | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| SCPGX2-45 | low | enhancement | in-queue | No DOT, GraphML or ELK export | src/scpg/diagram/factory.rs |
| SCPGX2-46 | low | enhancement | in-queue | Exporters do not check that uma, sta and tca belong together | src/scpg/diagram/factory.rs, src/scpg/diagram/export/plantuml.rs |

## find2:SRV (77)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SRV2-01 | high | flaw | in-queue | Git credential prompt freezes the whole server | src/adapters/server.rs |
| SRV2-02 | high | flaw | in-queue | Clones run with the operator's git credentials (confused deputy) | src/adapters/server.rs |
| SRV2-03 | high | flaw | in-queue | A panic in any request or phase kills the server process | src/adapters/server.rs, src/ast/builder.rs, src/core/types/token.rs |
| SRV2-04 | high | flaw | in-queue | Server directory walkers follow symlinks (loops, exponential traversal, escapes) | src/adapters/server.rs |
| SRV2-05 | high | flaw | in-queue | Symlinked source files are read through to devices and host files | src/adapters/server.rs, src/main.rs, src/ingestion/mod.rs |
| SRV2-06 | high | flaw | in-queue | Clone cache keyed only by the URL's last path segment | src/adapters/server.rs, scripts/restart_server.sh |
| SRV2-07 | high | flaw | partial | restart_server.sh SIGKILLs every process using the port, including clients | scripts/restart_server.sh, Makefile |
| SRV2-08 | high | flaw | partial | Docker builder toolchain is likely too old for the locked dependencies | Dockerfile, Cargo.lock, Cargo.toml |
| SRV2-09 | medium | flaw | in-queue | Server collects ~60 extensions the pipeline cannot parse | src/adapters/server.rs, src/ingestion/adapter/registry.rs, src/ingestion/adapter/kotlin.rs |
| SRV2-10 | medium | flaw | in-queue | Directory and test-name heuristics drop production sources | src/adapters/server.rs |
| SRV2-11 | medium | flaw | in-queue | CLI and server select different files for the same repository | src/main.rs, src/adapters/server.rs |
| SRV2-12 | medium | flaw | in-queue | CLI scan aborts or double-counts on symlinks and unreadable entries | src/main.rs |
| SRV2-13 | medium | flaw | in-queue | Hand-built JSON responses are invalid for common error strings | src/adapters/server.rs, src/ingestion/mod.rs |
| SRV2-14 | medium | flaw | in-queue | Status codes and error schema do not reflect outcomes | src/adapters/server.rs |
| SRV2-15 | medium | flaw | in-queue | repo_url is extracted by splitting on quotes, not by parsing JSON | src/adapters/server.rs |
| SRV2-16 | medium | flaw | in-queue | Session temp directory leaked on every error or panic path | src/adapters/server.rs |
| SRV2-17 | medium | flaw | in-queue | On Windows the temp dir is not deleted while corpus.tca is still mapped | src/adapters/server.rs, src/core/io/mmap.rs |
| SRV2-18 | medium | flaw | in-queue | Server swallows I/O failures that the CLI treats as fatal | src/adapters/server.rs, src/main.rs |
| SRV2-19 | medium | flaw | in-queue | Synchronous /api/analyze runs longer than the studio's 2.5 s fetch abort | src/adapters/server.rs, web/index.html |
| SRV2-20 | medium | flaw | in-queue | /api/source misses trigger full recursive walks of all roots | src/adapters/server.rs |
| SRV2-21 | medium | flaw | in-queue | /api/source returns an arbitrary or wrong file for common names | src/adapters/server.rs |
| SRV2-22 | medium | flaw | in-queue | /api/source query parsing and percent-decoding are incomplete | src/adapters/server.rs, web/js/editor.js |
| SRV2-23 | medium | flaw | in-queue | /api/supported-languages advertises languages that are not supported | src/adapters/server.rs, src/ingestion/adapter/registry.rs |
| SRV2-24 | medium | flaw | in-queue | No write timeout: a client that stops reading blocks the server | src/adapters/server.rs |
| SRV2-25 | medium | flaw | in-queue | No Host-header validation enables DNS rebinding | src/adapters/server.rs |
| SRV2-26 | medium | flaw | in-queue | Credentials embedded in repo_url are logged, echoed and persisted | src/adapters/server.rs |
| SRV2-27 | medium | flaw | in-queue | GitHub tree/sub-directory URLs are mapped to wrong paths | src/adapters/server.rs |
| SRV2-28 | medium | flaw | in-queue | ./target_repos grows without bound | src/adapters/server.rs |
| SRV2-29 | medium | flaw | in-queue | A lone verbosity flag panics the CLI | src/main.rs |
| SRV2-30 | medium | flaw | in-queue | inspect reports 'CRC-64 VERIFIED VALID' without verifying | src/main.rs, src/scpg/mmap/mod.rs |
| SRV2-31 | medium | flaw | in-queue | Requested diagram types and branch are ignored; web_repo config is dead code | src/adapters/server.rs, src/adapters/web_repo.rs, web/js/api.js |
| SRV2-32 | medium | enhancement | in-queue | Monolithic connection handler with no Request/Response abstraction | src/adapters/server.rs |
| SRV2-33 | medium | flaw | in-queue | Server is hard-wired to CWD-relative paths with no configuration | src/adapters/server.rs |
| SRV2-34 | medium | flaw | in-queue | Dockerfile PORT and HOST environment variables are ignored | Dockerfile, src/main.rs, src/adapters/server.rs |
| SRV2-35 | medium | flaw | in-queue | Security doc presents CRC-64 and SHA-256 as tamper protection | docs/security-architecture.md, src/ingestion/serializer.rs, src/ingestion/mod.rs |
| SRV2-36 | medium | flaw | in-queue | Security docs omit the network-facing server and clone path | docs/security-architecture.md, SECURITY.md |
| SRV2-37 | medium | enhancement | in-queue | SCPG query engine and XMI exporter are unreachable from CLI and API | src/main.rs, src/adapters/server.rs, src/scpg/api/mod.rs |
| SRV2-38 | low | flaw | in-queue | Predictable temp path; pre-existing dirs accepted; create errors ignored | src/adapters/server.rs |
| SRV2-39 | low | flaw | in-queue | Non-UTF-8 sources are reported missing and may be substituted | src/adapters/server.rs |
| SRV2-40 | low | flaw | in-queue | Whole-file reads with no size cap in /api/source and static serving | src/adapters/server.rs |
| SRV2-41 | low | flaw | in-queue | Hard-coded demo paths baked into /api/source | src/adapters/server.rs |
| SRV2-42 | low | flaw | in-queue | Version, health and capability endpoints are hard-coded | src/adapters/server.rs, src/main.rs |
| SRV2-43 | low | flaw | in-queue | HTTP/1.1 framing gaps: no Connection: close, abrupt close, absolute-form | src/adapters/server.rs |
| SRV2-44 | low | flaw | in-queue | Every CLI command creates log files in the current directory | src/main.rs, src/core/logger.rs |
| SRV2-45 | low | flaw | in-queue | Combined or misplaced flags become positional arguments | src/main.rs |
| SRV2-46 | low | flaw | in-queue | Command output goes through the logger and can be silenced | src/main.rs, src/core/logger.rs |
| SRV2-47 | low | flaw | in-queue | analyze reports success when diagram export fails and keeps stale files | src/main.rs |
| SRV2-48 | low | flaw | in-queue | Artifact bytes depend on how the source path was spelled | src/main.rs, src/ingestion/mod.rs |
| SRV2-49 | low | flaw | in-queue | CLI usage text misdescribes inputs, outputs and checks | src/main.rs |
| SRV2-50 | low | flaw | in-queue | Usage errors exit 0 or print to stdout | src/main.rs |
| SRV2-51 | low | flaw | in-queue | Startup banner prints an unusable or wrong address; IPv4-only bind | src/adapters/server.rs, scripts/restart_server.sh |
| SRV2-52 | low | flaw | in-queue | Accept errors are silently discarded and can spin the CPU | src/adapters/server.rs |
| SRV2-53 | low | flaw | in-queue | Header and body sent in separate writes with Nagle enabled | src/adapters/server.rs |
| SRV2-54 | low | flaw | in-queue | Static file serving gaps (MIME, charset, decoding, directory index) | src/adapters/server.rs |
| SRV2-55 | low | enhancement | in-queue | No caching validators or compression | src/adapters/server.rs |
| SRV2-56 | low | flaw | in-queue | PlantUML diagrams generated twice; alias types written redundantly | src/adapters/server.rs |
| SRV2-57 | low | flaw | in-queue | Traceability payload is truncated and mislabeled | src/adapters/server.rs, web/js/viewer.js |
| SRV2-58 | low | flaw | in-queue | Session id is a seconds timestamp; no real SCPG hash is returned | src/adapters/server.rs, web/index.html |
| SRV2-59 | low | flaw | in-queue | validate_url silently rewrites names and accepts plaintext http | src/adapters/web_repo.rs |
| SRV2-60 | low | enhancement | partial | HTTP server and git shell-out compiled into the library crate | src/lib.rs, src/adapters/mod.rs, Cargo.toml |
| SRV2-61 | low | enhancement | in-queue | No request logging or structured observability | src/adapters/server.rs |
| SRV2-62 | low | enhancement | in-queue | No graceful shutdown | src/adapters/server.rs |
| SRV2-63 | low | enhancement | in-queue | No result cache: identical requests rerun the whole pipeline | src/adapters/server.rs |
| SRV2-64 | low | flaw | in-queue | No file-count or file-size caps although file ids are 16-bit | src/adapters/server.rs, src/ingestion/mod.rs, src/ast/mod.rs |
| SRV2-65 | low | flaw | in-queue | Expect: 100-continue is not supported | src/adapters/server.rs |
| SRV2-66 | low | flaw | in-queue | Redundant and unreachable checks in collect_files | src/adapters/server.rs |
| SRV2-67 | low | flaw | in-queue | Docker build is not reproducible and copies the wrong inputs | Dockerfile, src/adapters/server.rs |
| SRV2-68 | low | flaw | in-queue | Launchers open the browser too early, run debug builds and lose fallbacks | run.sh, run.bat |
| SRV2-69 | low | flaw | in-queue | Direct-file fallback cannot load the module-based studio | run.sh, run.bat, web/index.html |
| SRV2-70 | low | flaw | in-queue | run.bat batch-parsing hazards: unescaped ')' in a block, delayed expansion | run.bat |
| SRV2-71 | low | flaw | in-queue | run.bat trusts 'where python', which finds the Microsoft Store stub | run.bat |
| SRV2-72 | low | flaw | partial | run.bat is checked out with LF line endings | run.bat, .gitattributes |
| SRV2-73 | low | flaw | in-queue | Python fallback listens on all interfaces and assumes Python 3.7+ | run.sh, run.bat |
| SRV2-74 | low | flaw | partial | restart_server.sh misreports when lsof is missing and runs a debug build | scripts/restart_server.sh, Makefile |
| SRV2-75 | low | flaw | fixed | Makefile target hygiene issues | Makefile |
| SRV2-76 | low | flaw | in-queue | Security doc's input-bounds claims contradict the code | docs/security-architecture.md, src/core/types/token.rs, src/ingestion/walker.rs |
| SRV2-77 | low | flaw | in-queue | Security contact address is unverifiable | SECURITY.md, SUPPORT.md |

## find2:SYM (44)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SYM2-01 | high | flaw | in-queue | Package declarations are truncated at segments named data, object, open, sealed, abstract, val, var or fun | src/symbol/passes/pass1_discovery.rs:493-519, src/symbol/passes/pass1_discovery.rs:529-569 |
| SYM2-02 | high | flaw | in-queue | Primitive and JDK-stub type ids are not written to .sta, so int, void, String and List members render with blank types | src/symbol/serializer.rs:37-39, src/symbol/serializer.rs:64, src/symbol/builder.rs:240-246 |
| SYM2-03 | high | flaw | in-queue | TH_EXTENDS vs TH_IMPLEMENTS is guessed from the target's kind, so implements clauses become extends edges | src/symbol/passes/pass5_hierarchy.rs:56-71, src/symbol/passes/pass5_hierarchy.rs:141-147, src/ast/adapter/java.rs:115-116 |
| SYM2-04 | high | flaw | in-queue | variable_declarator becomes its own SK_LOCAL_VAR, duplicating fields and locals and losing all but one field of `int a, b;` | src/ast/adapter/java.rs:42, src/symbol/passes/pass1_discovery.rs:134-190, src/symbol/passes/pass1_discovery.rs:748-791 |
| SYM2-05 | high | flaw | in-queue | The token sweep rewrites an existing AST symbol's kind to class, turning parameters into phantom classes | src/symbol/passes/pass1_discovery.rs:372-395, src/symbol/passes/pass1_discovery.rs:793-889 |
| SYM2-06 | high | flaw | in-queue | Constructor bodies, switch blocks and annotation bodies are dropped before Phase 3, so declarations inside them never become symbols | src/ast/adapter/java.rs:129, src/symbol/uml_meta/associations.rs:64-111, src/symbol/passes/pass1_discovery.rs:134-190 |
| SYM2-07 | medium | flaw | in-queue | sanitize_th_graph deletes legitimate edges that merely lead into a cycle | src/symbol/builder.rs:173-233, src/symbol/passes/pass5_hierarchy.rs:218 |
| SYM2-08 | medium | flaw | in-queue | Primitive-typed fields create TH_USES edges and Dependency associations to ids that are not symbols | src/symbol/passes/pass5_hierarchy.rs:166-213, src/symbol/uml_meta/associations.rs:24-27, src/uma/patterns/query.rs:41-43 |
| SYM2-09 | medium | flaw | in-queue | Members found by the token sweep have no token range, so Pass 4's Kotlin type binding never runs | src/symbol/passes/pass1_discovery.rs:439-447, src/symbol/passes/pass1_discovery.rs:403-404, src/symbol/passes/pass4_members.rs:60-119 |
| SYM2-10 | medium | flaw | in-queue | The sweep de-duplicates members per (file, name), dropping members of later classes and overloads | src/symbol/passes/pass1_discovery.rs:201-227, src/symbol/passes/pass1_discovery.rs:433-448 |
| SYM2-11 | medium | flaw | in-queue | The sweep names members from the wrong token (type parameters, receiver types, soft keywords) | src/symbol/passes/pass1_discovery.rs:421-453, src/symbol/passes/pass1_discovery.rs:33-93 |
| SYM2-12 | medium | flaw | in-queue | Passes 4 and 5 bind types through a global simple-name map in which the last-created class wins | src/symbol/passes/pass4_members.rs:18-37, src/symbol/passes/pass4_members.rs:108-111, src/symbol/passes/pass5_hierarchy.rs:19-38 |
| SYM2-13 | medium | flaw | in-queue | Java `var` locals are typed as a fabricated external type named "var" | src/symbol/passes/pass3_types.rs:38-89, src/symbol/adapter/java.rs:88-101, src/symbol/passes/pass4_members.rs:46-57 |
| SYM2-14 | medium | flaw | in-queue | Lambda parameters never become SK_PARAM symbols | src/ast/adapter/java.rs:41, src/ast/adapter/java.rs:86, src/ast/adapter/java.rs:129 |
| SYM2-15 | medium | flaw | in-queue | Blocks do not open scopes, while every leaf declaration does | src/symbol/passes/pass1_discovery.rs:185-189, src/symbol/adapter/java.rs:32-51, src/symbol/adapter/java.rs:74-86 |
| SYM2-16 | medium | enhancement | in-queue | Project symbols never get fully qualified names | src/symbol/passes/pass1_discovery.rs:160-181, src/symbol/qual_name_table.rs:5-38, src/symbol/builder.rs:31-32 |
| SYM2-17 | medium | flaw | in-queue | Scope-graph serialization is lossy and its fields mislead consumers | src/symbol/serializer.rs:124-136, src/symbol/scope_graph/mod.rs:68-98, src/core/types/symbol.rs:148-187 |
| SYM2-18 | medium | flaw | in-queue | Collection detection matches substrings of the qualified name | src/symbol/uml_meta/associations.rs:39-50, src/symbol/adapter/java.rs:103-123 |
| SYM2-19 | medium | flaw | in-queue | The composition test ignores which field a constructor creates | src/symbol/uml_meta/associations.rs:52-111 |
| SYM2-20 | medium | enhancement | in-queue | There is no Kotlin semantic adapter or Kotlin standard-library model | src/symbol/mod.rs:38, src/symbol/adapter/java.rs:88-101, src/symbol/adapter/java.rs:125-133 |
| SYM2-21 | medium | enhancement | in-queue | The documented annotation table and UML metadata section do not exist, and annotations are never recorded | src/symbol/serializer.rs:77-195, src/core/types/symbol.rs:111, docs/plans/phase3.md:15 |
| SYM2-22 | medium | enhancement | in-queue | Phase 3 produces no resolution diagnostics | src/symbol/passes/pass3_types.rs:86-89, src/symbol/builder.rs:224-229, src/symbol/passes/pass1_discovery.rs:372-411 |
| SYM2-23 | medium | enhancement | deferred-testing-phase | No regression tests cover the defects in this list | tests/symbol_tests.rs:1-175 |
| SYM2-24 | low | flaw | in-queue | STA deserialization allocates memory from untrusted counts and lengths | src/symbol/serializer.rs:240, src/symbol/serializer.rs:287-294, src/symbol/serializer.rs:321-333 |
| SYM2-25 | low | flaw | in-queue | Deserialization does no semantic validation, and the From<u8> enum conversions silently coerce unknown values | src/symbol/serializer.rs:230, src/symbol/serializer.rs:325, src/symbol/serializer.rs:373-397 |
| SYM2-26 | low | flaw | in-queue | verify_invariants skips range and link checks and the BPA/TCA hash link | src/symbol/builder.rs:256-309, src/symbol/builder.rs:350-354, src/symbol/mod.rs:75-82 |
| SYM2-27 | low | flaw | in-queue | Per-class diagnostic logging at INFO level on the hot path | src/symbol/passes/pass1_discovery.rs:126-133, src/symbol/passes/pass1_discovery.rs:141-158, src/core/logger.rs:124-158 |
| SYM2-28 | low | flaw | in-queue | Pass 2 has no handling for static imports or import aliases | src/symbol/passes/pass2_imports.rs:1-2, src/symbol/passes/pass2_imports.rs:28-43, docs/plans/phase3.md:364 |
| SYM2-29 | low | flaw | in-queue | Empty or mismatched inputs crash Pass 2 or silently skip discovery | src/symbol/passes/pass2_imports.rs:53-57, src/symbol/passes/pass1_discovery.rs:24-26 |
| SYM2-30 | low | flaw | in-queue | ScopeRecord.import_count is a u16 incremented without overflow checks | src/symbol/scope_graph/mod.rs:81-98, src/core/types/symbol.rs:182 |
| SYM2-31 | low | flaw | in-queue | Passes disagree on which symbol kinds count as types | src/symbol/passes/pass1_discovery.rs:211-218, src/symbol/passes/pass1_discovery.rs:579-587, src/symbol/passes/pass4_members.rs:22-28 |
| SYM2-32 | low | flaw | in-queue | Modules, lambdas, initializers and type-parameter lists are named after their first identifier | src/symbol/passes/pass1_discovery.rs:793-890, src/symbol/adapter/java.rs:35, src/symbol/adapter/java.rs:46-48 |
| SYM2-33 | low | flaw | in-queue | Instance initializer blocks are never recognized | src/symbol/adapter/java.rs:48-49, src/symbol/adapter/java.rs:69, src/ast/adapter/java.rs:45 |
| SYM2-34 | low | flaw | in-queue | Several SymbolRecord fields are never filled in | src/symbol/passes/pass1_discovery.rs:160-181, src/core/types/symbol.rs:97-121, src/symbol/uml_meta/associations.rs:46 |
| SYM2-35 | low | enhancement | in-queue | The modifier and visibility model cannot carry half of SymbolModifiers or Kotlin visibility | src/symbol/passes/pass1_discovery.rs:895-943, src/core/types/symbol.rs:55-95, src/core/types/ast.rs:193-200 |
| SYM2-36 | low | flaw | in-queue | Layout comments misdescribe the records, the magic number and the hierarchy encoding | src/core/types/symbol.rs:97, src/core/types/symbol.rs:212, src/symbol/serializer.rs:2 |
| SYM2-37 | low | flaw | in-queue | Dead or redundant code not covered by SYM-17 | src/symbol/passes/pass1_discovery.rs:241-247, src/symbol/passes/pass1_discovery.rs:313, src/symbol/passes/pass1_discovery.rs:333 |
| SYM2-38 | low | flaw | in-queue | A second, unused PatternDetector shares the uma type's name and has wrong rules | src/symbol/uml_meta/patterns.rs:13-98, src/symbol/mod.rs:20, src/uma/patterns/mod.rs:37 |
| SYM2-39 | low | flaw | in-queue | Building the artifact clones all builder data, and serialization trusts cached counts | src/symbol/serializer.rs:41-75, src/symbol/serializer.rs:83-89, src/symbol/builder.rs:22-34 |
| SYM2-40 | low | flaw | in-queue | Name extraction allocates a lowercase string per token and over-filters identifiers | src/symbol/passes/pass1_discovery.rs:711-713, src/symbol/passes/pass1_discovery.rs:763-765 |
| SYM2-41 | low | enhancement | deferred-testing-phase | The round-trip test compares only header fields | tests/symbol_tests.rs:94-103 |
| SYM2-42 | low | enhancement | deferred-testing-phase | The unit tests skip the code they are named after | tests/symbol_tests.rs:106-143, tests/symbol_tests.rs:145-175 |
| SYM2-43 | low | enhancement | in-queue | Dependency edges come only from field types, and TH_CREATES is never emitted | src/core/types/symbol.rs:249-254, src/symbol/passes/pass5_hierarchy.rs:159-215 |
| SYM2-44 | low | enhancement | in-queue | Phase 3 runs single-threaded and re-hashes inputs that Phase 2 already hashed | src/symbol/mod.rs:30-93, src/symbol/passes/pass1_discovery.rs:109-194, src/symbol/passes/pass1_discovery.rs:259-573 |

## find2:UML (49)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| UML2-01 | high | flaw | in-queue | LabelExtractor slices UTF-8 labels by byte and panics on non-ASCII tokens, aborting Phase 9 | src/uma/label_extraction.rs:33-34, src/uma/behavioral/activity_diagram.rs:82-90, src/ast/adapter/java.rs:178 |
| UML2-02 | medium | flaw | in-queue | Sequence records mix interner name ids and symbol ids, so scenario names resolve to unrelated symbols | src/uma/behavioral/sequence_diagram.rs:58-61, src/uma/behavioral/sequence_diagram.rs:117-120, src/uma/behavioral/sequence_diagram.rs:156 |
| UML2-03 | medium | flaw | in-queue | extends_sym keeps only the last TH_EXTENDS edge, so extra super-interfaces and enum/record realizations are lost | src/uma/structural/class_diagram.rs:281-295, src/symbol/passes/pass5_hierarchy.rs:58-67 |
| UML2-04 | medium | flaw | in-queue | Associations skip record-typed targets and all self-references | src/uma/structural/class_diagram.rs:253-279 |
| UML2-05 | medium | flaw | in-queue | The test-source filter only matches absolute forward-slash paths, so Windows and relative runs include test classes | src/uma/structural/class_diagram.rs:134-141, src/ingestion/mod.rs:137-138 |
| UML2-06 | medium | flaw | in-queue | Activity nodes misclassify switch dispatch and exception successors | src/uma/behavioral/activity_diagram.rs:52-78, src/uma/behavioral/activity_diagram.rs:134, src/cfg/stmts/switch_stmt.rs:41 |
| UML2-07 | medium | flaw | partial | Sequence extraction has no size bound, and its u16 ordinals wrap | src/uma/behavioral/sequence_diagram.rs:42, src/uma/behavioral/sequence_diagram.rs:84, src/uma/behavioral/sequence_diagram.rs:96-151 |
| UML2-08 | medium | flaw | in-queue | Entry-point detection drops protected framework callbacks and promotes bodyless and dead methods | src/uma/actor_identification.rs:24-45, src/cg/resolution.rs:82-90 |
| UML2-09 | medium | flaw | in-queue | Call sites with no in-project target are left out of sequence diagrams | src/uma/behavioral/sequence_diagram.rs:109-149, src/cg/resolution.rs:49-55 |
| UML2-10 | medium | flaw | in-queue | Composite detection copies the Decorator predicate and never checks for a collection | src/uma/patterns/composite.rs:15-34, src/uma/patterns/decorator.rs:14-33, src/uma/patterns/query.rs:103-121 |
| UML2-11 | medium | flaw | in-queue | Phase9Stage::run panics when the .uma write fails and cannot return an error | src/uma/mod.rs:45-56, src/uma/mod.rs:97-99, src/core/pipeline.rs:217-220 |
| UML2-12 | medium | flaw | in-queue | UMASerializer::read trusts every count and offset, so malformed files panic or abort | src/uma/serializer.rs:259-274, src/uma/serializer.rs:650-679, src/main.rs:459-466 |
| UML2-13 | medium | flaw | in-queue | Much of the .uma schema is write-only: source links, confidence, counts and fragments are never read | src/uma/types.rs:51-267, src/scpg/diagram/export/json.rs:580-581 |
| UML2-14 | medium | enhancement | deferred-testing-phase | src/uma has no unit tests | src/uma/mod.rs:9-17, tests/uma_tests.rs:19-20 |
| UML2-15 | medium | flaw | deferred-testing-phase | pipeline_accuracy_tests never reaches Phase 9 and locks in the missing constructor CFG | tests/pipeline_accuracy_tests.rs:22-23, tests/pipeline_accuracy_tests.rs:142, tests/pipeline_accuracy_tests.rs:222 |
| UML2-16 | medium | enhancement | in-queue | UMA has no use-case, actor or profile records | src/uma/types.rs:254-267, src/uma/mod.rs:3-5, src/uma/actor_identification.rs:6 |
| UML2-17 | low | flaw | in-queue | Annotation types never reach the class diagram, and the STEREOTYPE_ANNOTATION and SK_LAMBDA arms are unreachable | src/uma/structural/class_diagram.rs:79-87, src/uma/structural/class_diagram.rs:180-193, src/uma/types.rs:20 |
| UML2-18 | low | flaw | in-queue | The local-type filter is inconsistent: local classes are kept but local records, enums and interfaces are dropped | src/uma/structural/class_diagram.rs:64-77 |
| UML2-19 | low | flaw | in-queue | A missing UML link is replaced with a made-up file 0, line 1 location that the tests cannot detect | src/uma/structural/class_diagram.rs:304-319, src/tra/uml_link/mod.rs:49-51, tests/uma_tests.rs:139-145 |
| UML2-20 | low | flaw | in-queue | Methods without a PSA summary get an invented cyclomatic=1 and sat_count=1 | src/uma/structural/class_diagram.rs:222-227, src/uma/visitor.rs:205-215 |
| UML2-21 | low | flaw | in-queue | Field and method modifiers are truncated from u16 to u8 | src/uma/structural/class_diagram.rs:214, src/uma/structural/class_diagram.rs:233, src/uma/types.rs:57 |
| UML2-22 | low | enhancement | in-queue | ClassRecord.type_param_count is always 0, and type parameters are never captured | src/uma/structural/class_diagram.rs:332, src/symbol/passes/pass1_discovery.rs:176, src/symbol/passes/pass3_types.rs:135 |
| UML2-23 | low | flaw | in-queue | Debug diagnostics are logged at INFO, and the kind check is written three times | src/uma/structural/class_diagram.rs:20-23, src/uma/structural/class_diagram.rs:44-62, src/uma/structural/class_diagram.rs:167-178 |
| UML2-24 | low | flaw | in-queue | Activity node indices and counts are truncated to u16 | src/uma/behavioral/activity_diagram.rs:68, src/uma/behavioral/activity_diagram.rs:113-114, src/uma/behavioral/activity_diagram.rs:130-131 |
| UML2-25 | low | flaw | in-queue | Activity labels come from a block's first statement, not the branch condition, and always end in '()' | src/uma/behavioral/activity_diagram.rs:80-90, src/cfg/stmts/if_stmt.rs:27, src/uma/label_extraction.rs:28 |
| UML2-26 | low | flaw | in-queue | message_kind stores raw call-graph edge codes, and the entry message reuses call_site_id 0 | src/uma/behavioral/sequence_diagram.rs:74-83, src/uma/behavioral/sequence_diagram.rs:137, src/core/types/cg.rs:7-13 |
| UML2-27 | low | flaw | in-queue | Behavioral views ignore the class-diagram filters and cover test and excluded classes | src/uma/behavioral/activity_diagram.rs:24-28, src/uma/actor_identification.rs:24-45, src/uma/behavioral/state_machine.rs:13-18 |
| UML2-28 | low | flaw | in-queue | ClassRecords refer to sym ids that have no ClassRecord of their own | src/uma/structural/class_diagram.rs:239-244, src/uma/structural/class_diagram.rs:253-279, src/uma/structural/class_diagram.rs:283-301 |
| UML2-29 | low | flaw | in-queue | The Builder heuristic counts static self-returning methods, so Singletons and static factories become Builders | src/uma/patterns/builder.rs:21-45, tests/uma_tests.rs:27-48 |
| UML2-30 | low | flaw | in-queue | Facade and Adapter count primitive-typed and stub-typed fields as subsystems or adaptees | src/uma/patterns/facade.rs:11-14, src/uma/patterns/adapter.rs:15-17, src/symbol/adapter/java.rs:90-98 |
| UML2-31 | low | flaw | in-queue | Four PatternInspector helpers are never called, and their doc comments do not match the code | src/uma/patterns/query.rs:29-38, src/uma/patterns/query.rs:77-132 |
| UML2-32 | low | flaw | in-queue | Three separate pattern-detection paths, and the pattern-name tables are duplicated | src/uma/patterns/mod.rs:40-194, src/uma/patterns/chain.rs:297-353, src/symbol/uml_meta/patterns.rs:1-25 |
| UML2-33 | low | flaw | in-queue | .uma writes are not atomic | src/uma/serializer.rs:14-15, src/uma/serializer.rs:224 |
| UML2-34 | low | flaw | in-queue | Record size comments disagree with the serialized layout, and the layout is unaligned | src/uma/types.rs:51, src/uma/types.rs:64, src/uma/types.rs:180 |
| UML2-35 | low | flaw | in-queue | UMA records use inconsistent id widths, carry redundant fields and lack equality derives | src/uma/types.rs:78-97, src/uma/types.rs:101-108, src/uma/types.rs:172-178 |
| UML2-36 | low | flaw | in-queue | Package records inherit a wrong parent and corrupted sibling chains from Phase 3 | src/uma/structural/package_diagram.rs:29-53, src/uma/structural/component_diagram.rs:18-27, src/symbol/passes/pass1_discovery.rs:549 |
| UML2-37 | low | flaw | in-queue | Package and component counts disagree with the class diagram and miscount interfaces | src/uma/structural/package_diagram.rs:19-45, src/uma/structural/component_diagram.rs:10-42, src/uma/structural/mod.rs:41-42 |
| UML2-38 | low | flaw | in-queue | The metrics visitor reports an inheritance depth it never computes, and its path-count sum wraps | src/uma/visitor.rs:144-225, src/psa/bdd/sat_count.rs:79-92 |
| UML2-39 | low | enhancement | in-queue | The attack-surface visitor only matches name suffixes, and neither visitor is wired into the product | src/uma/visitor.rs:89-140, src/uma/visitor.rs:227-288 |
| UML2-40 | low | flaw | in-queue | Doc comments in Phase 9 promise behavior that does not exist | src/uma/actor_identification.rs:1, src/uma/actor_identification.rs:11, src/uma/label_extraction.rs:10 |
| UML2-41 | low | enhancement | in-queue | UMABuilder::finalize checks no invariants in release builds | src/uma/builder.rs:48-57 |
| UML2-42 | low | flaw | in-queue | EXTERNAL_ACTOR_ID is redefined as a literal in the exporters | src/uma/actor_identification.rs:6, src/scpg/diagram/export/mermaid.rs:11, src/scpg/diagram/export/plantuml.rs:10 |
| UML2-43 | low | flaw | deferred-testing-phase | Pattern test inputs are not valid Java, and the file headers claim coverage that is not there | tests/design_pattern_architecture_tests.rs:1-9, tests/design_pattern_architecture_tests.rs:23-42, tests/adversarial_trick_tests.rs:3-12 |
| UML2-44 | low | flaw | deferred-testing-phase | uma_tests depends on symbol id 1 and has invariants that cannot fail | tests/uma_tests.rs:116, tests/uma_tests.rs:124-137, tests/uma_tests.rs:147-153 |
| UML2-45 | low | flaw | not-yet-dispatched | The 40-line pipeline setup is copied across the test files | tests/uma_tests.rs:52-100, tests/strategy_diagram_tests.rs:65-107, tests/strategy_diagram_tests.rs:159-201 |
| UML2-46 | low | enhancement | in-queue | UMA records have no identity that stays stable across runs | src/uma/types.rs:78-97, src/uma/types.rs:246-252 |
| UML2-47 | low | enhancement | in-queue | Phase 9 runs on one thread although the per-class and per-function work is independent | src/uma/mod.rs:61-70, src/uma/behavioral/mod.rs:44-46, src/uma/structural/mod.rs:39-44 |
| UML2-48 | low | enhancement | in-queue | The stereotype field cannot express sealed, data, object, companion or value types | src/uma/types.rs:14-20, src/uma/structural/class_diagram.rs:180-193 |
| UML2-49 | low | flaw | in-queue | The class extractor treats path_str_offset as an interner id, which a disk-loaded TCA does not hold | src/uma/structural/class_diagram.rs:131-134, src/ingestion/serializer.rs:97, src/ingestion/serializer.rs:228 |

## find2:WEB (68)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| WEB2-01 | high | flaw | in-queue | PNG export always throws ReferenceError (isDarkMode not imported) | web/js/graph-canvas.js:16, web/js/graph-canvas.js:1414-1422, web/index.html:1453-1468 |
| WEB2-02 | high | flaw | in-queue | Java extends/implements parsing loses or misclassifies inheritance edges | web/js/github-engine.js:291, web/js/github-engine.js:304-334, web/js/github-engine.js:358-367 |
| WEB2-03 | high | flaw | in-queue | Kotlin classes, Java records and nested generics are not detected; Kotlin packages not extracted | web/js/github-engine.js:271, web/js/github-engine.js:291, web/js/github-engine.js:298 |
| WEB2-04 | high | flaw | in-queue | ROBDD terminals both render as FALSE for engine-produced IR | web/js/graph-loader.js:108-114, web/js/uml-card-renderer.js:528-544, web/diagrams/robdd.json |
| WEB2-05 | medium | flaw | in-queue | Final pseudo-state rendered as an initial state | web/js/uml-card-renderer.js:236-246, web/js/graph-loader.js:122-129, web/diagrams/statemachine.json |
| WEB2-06 | medium | flaw | in-queue | Zero metrics coerced to undefined: infeasible paths shown as '1 Canonical SAT' | web/js/graph-loader.js:228-230, web/index.html:658-667 |
| WEB2-07 | medium | flaw | in-queue | Bundled communication.json is a copy of sequence.json; class.json has zero edges | web/diagrams/communication.json, web/diagrams/sequence.json, web/diagrams/class.json |
| WEB2-08 | medium | flaw | in-queue | Custom projections emit edges whose endpoints are not in the node set; edges silently dropped | web/js/graph-canvas.js:180-236, web/js/graph-canvas.js:239-252, web/js/graph-canvas.js:255-274 |
| WEB2-09 | medium | flaw | in-queue | Fabrication sites not cited by WEB-02 | web/js/graph-canvas.js:255-274, web/js/graph-canvas.js:302-364, web/js/graph-canvas.js:386-474 |
| WEB2-10 | medium | flaw | in-queue | In-browser package hierarchy flattened and every package labelled DOMAIN | web/js/github-engine.js:493-510, web/js/graph-loader.js:43-47 |
| WEB2-11 | medium | flaw | in-queue | Collection-typed fields never produce associations; duplicate parallel edges | web/js/github-engine.js:370-380, web/js/github-engine.js:553-566 |
| WEB2-12 | medium | flaw | in-queue | Explicit /tree/<branch> in the URL is overridden by the repo default branch | web/js/github-engine.js:52-61, web/js/github-engine.js:65-80 |
| WEB2-13 | medium | flaw | in-queue | GitHub ingestion lacks the local path's ignore list; filters are inconsistent | web/js/github-engine.js:89-94, web/js/github-engine.js:183-191, web/js/github-engine.js:103 |
| WEB2-16 | medium | flaw | in-queue | Concurrent imports possible via Enter key, presets and history | web/index.html:1119-1135, web/index.html:1137-1160, web/index.html:1240-1264 |
| WEB2-17 | medium | flaw | in-queue | Folder drag-and-drop never works; ZIP drop errors are unhandled | web/index.html:1088-1117 |
| WEB2-18 | medium | flaw | in-queue | No size limits on ZIP or folder ingestion (zip bomb or huge file crashes the tab) | web/index.html:1055-1080, web/index.html:1031-1043, web/js/github-engine.js:207-214 |
| WEB2-19 | medium | flaw | in-queue | Unguarded localStorage access aborts the whole studio when storage is blocked | web/js/graph-canvas.js:28, web/js/graph-canvas.js:72, web/js/themes/theme-manager.js:204-206 |
| WEB2-20 | medium | flaw | in-queue | Global Space and arrow handlers swallow native keyboard behavior | web/js/graph-canvas.js:1139-1152, web/js/graph-canvas.js:1180-1184, web/js/graph-canvas.js:1247-1256 |
| WEB2-21 | medium | flaw | in-queue | Missing ARIA roles and states on tabs, toggles and icon buttons (WCAG 4.1.2) | web/index.html:317-327, web/index.html:190-198, web/index.html:286-288 |
| WEB2-22 | medium | flaw | in-queue | Text colour contrast below WCAG 1.4.3 in both themes and on cards | web/themes/dark.css:47, web/themes/light.css:47, web/themes/light.css:67-69 |
| WEB2-23 | medium | flaw | in-queue | Word wrap makes code lines overlap and misaligns the line numbers | web/style.css:1809-1820, web/style.css:1773-1782, web/js/editor.js:523-558 |
| WEB2-24 | medium | flaw | in-queue | Code viewer race: a slower earlier load overwrites the newer file | web/js/editor.js:139-225 |
| WEB2-26 | medium | flaw | in-queue | Project Explorer shows fabricated and duplicate files; the badge counts nodes | web/js/file-tree.js:40, web/js/file-tree.js:103-119, web/js/file-tree.js:25-26 |
| WEB2-28 | medium | flaw | in-queue | Hover HUD and minimap tooltip show '<>' or '>' instead of the class name | web/index.html:678-683, web/js/minimap-navigator.js:223-225, web/js/graph-loader.js:212 |
| WEB2-30 | medium | flaw | in-queue | AI export reflects the on-screen projection, not the codebase; counts are wrong | web/js/graph-canvas.js:1706-1910, web/js/graph-canvas.js:1798-1808, web/js/graph-canvas.js:1871 |
| WEB2-36 | medium | flaw | in-queue | Use-case actors drawn as sequence lifeline boxes | web/js/graph-loader.js:174-177, web/js/uml-card-renderer.js:420-440 |
| WEB2-37 | medium | flaw | in-queue | Timing tracks show no states and have a non-proportional time axis | web/js/uml-card-renderer.js:625-655, web/diagrams/timing.json |
| WEB2-53 | medium | flaw | in-queue | Docs pages are chat-widget fragments: no document shell, undefined sendPrompt and CSS tokens | web/docs/openheart_10_phase_pipeline.html:1-20, web/docs/phase1_architecture_and_bit_layout.html:32-42, web/docs/phase2_bp_architecture.html |
| WEB2-65 | medium | enhancement | in-queue | Run in-browser ingestion in a cancellable Web Worker | web/js/github-engine.js:176-261, web/js/github-engine.js:266-400, web/index.html:956-1020 |
| WEB2-66 | medium | enhancement | in-queue | Studio cannot export PlantUML, Mermaid or XMI | web/index.html:204-262, web/diagrams/class.puml, web/diagrams/class.mmd |
| WEB2-68 | medium | enhancement | deferred-testing-phase | Concrete unit tests to add for the defects above | web/js/graph-canvas.js, web/js/github-engine.js, web/js/graph-loader.js |
| WEB2-14 | low | flaw | in-queue | files_processed reports discovered files, not parsed files | web/js/github-engine.js:160-165, web/js/github-engine.js:252-257, web/index.html:922-926 |
| WEB2-15 | low | flaw | in-queue | Local imports get bogus raw.githubusercontent.com URLs that are fetched on cache miss | web/js/github-engine.js:211, web/js/github-engine.js:267, web/js/editor.js:37-39 |
| WEB2-25 | low | flaw | in-queue | Selection feedback loop double-loads files and double-selects tree rows | web/index.html:547-556, web/index.html:642-676, web/js/graph-canvas.js:728-752 |
| WEB2-27 | low | flaw | in-queue | File tree selection over-matches and the folder model desyncs | web/js/file-tree.js:285-324, web/js/file-tree.js:212, web/js/file-tree.js:243-249 |
| WEB2-29 | low | flaw | in-queue | describeRelationSemantics labels transitions and control flow as 'association/calls' | web/js/graph-canvas.js:1665-1700, web/js/graph-loader.js:253 |
| WEB2-31 | low | flaw | in-queue | Expanding a package re-shows filtered edges; edges without a kind ignore filters | web/js/graph-canvas.js:2160-2161, web/js/graph-canvas.js:796-808, web/js/graph-canvas.js:778-794 |
| WEB2-32 | low | flaw | in-queue | Package collapse hides relations, uses dead sizing code and drifts labels | web/js/graph-canvas.js:2142-2186, web/js/uml-layout.js:147-174 |
| WEB2-33 | low | flaw | in-queue | Standalone-node grid overlaps cards of different sizes | web/js/uml-layout.js:259-273 |
| WEB2-34 | low | flaw | in-queue | Hierarchical layout pins any id containing 'start' or 'init' to rank 0 | web/js/uml-layout.js:352-361 |
| WEB2-35 | low | flaw | in-queue | PUML fallback parser: stereotypes swallowed, left arrows reversed, many [*] states | web/js/puml-parser.js:42-44, web/js/puml-parser.js:74-76, web/js/puml-parser.js:164-171 |
| WEB2-38 | low | flaw | in-queue | Most card types use fixed widths; text overflows and content is silently dropped | web/js/uml-card-renderer.js:42, web/js/uml-card-renderer.js:172, web/js/uml-card-renderer.js:487-525 |
| WEB2-39 | low | flaw | in-queue | Card SVGs request web fonts that SVG-as-image cannot load | web/js/uml-card-renderer.js:86-155, web/index.html:22 |
| WEB2-40 | low | flaw | in-queue | A node with a null name crashes the whole render | web/js/uml-card-renderer.js:10-32, web/js/uml-card-renderer.js:233-237, web/js/graph-loader.js:46 |
| WEB2-41 | low | flaw | in-queue | Magnifier loupe shows the diagram on a black background | web/js/minimap-navigator.js:129, web/js/minimap-navigator.js:622-631, web/style.css:1320-1336 |
| WEB2-42 | low | flaw | in-queue | M/L shortcuts ignore modifier keys and swallow Ctrl/Cmd+L | web/js/minimap-navigator.js:272-287 |
| WEB2-43 | low | flaw | in-queue | Shared HUD status line clobbers errors and results | web/js/graph-canvas.js:2332-2349, web/js/graph-canvas.js:2264-2266, web/index.html:678-683 |
| WEB2-44 | low | flaw | in-queue | Import renders a class diagram while the sidebar still highlights another tab | web/js/graph-canvas.js:98-103, web/index.html:935, web/index.html:1010 |
| WEB2-45 | low | flaw | in-queue | Dead preset, inconsistent preset lists, and lossy history dedupe | web/index.html:89-94, web/index.html:1137-1160, web/index.html:1208-1227 |
| WEB2-46 | low | flaw | in-queue | Startup waits on the manifest fetch; theme applied late (flash of wrong theme) | web/index.html:416-421, web/index.html:778, web/js/themes/theme-manager.js:103-116 |
| WEB2-47 | low | flaw | in-queue | Static HTML placeholders present fabricated status before any analysis | web/index.html:100, web/index.html:321, web/index.html:340-359 |
| WEB2-48 | low | flaw | in-queue | Empty-view placeholder node renders as an unlabelled grey dot | web/js/graph-canvas.js:612-618, web/js/graph-canvas.js:650-654, web/js/themes/theme-manager.js:244-582 |
| WEB2-49 | low | flaw | in-queue | Saved state embeds card SVGs: bloat, stale theme, and an image-URL load from untrusted files | web/js/graph-canvas.js:1354-1381, web/js/graph-canvas.js:1383-1412, web/js/graph-canvas.js:1368 |
| WEB2-50 | low | flaw | in-queue | Dead and misleading code, and conflicting colour sources | web/js/graph-canvas.js:15, web/js/graph-canvas.js:407, web/js/graph-canvas.js:685 |
| WEB2-51 | low | flaw | in-queue | Manifest is not authoritative: duplicated in JS, fields ignored, relation kinds missing | web/diagrams/manifest.json:16-19, web/diagrams/manifest.json:396-405, web/js/themes/theme-manager.js:8-98 |
| WEB2-52 | low | flaw | in-queue | Unreferenced assets deployed (.mmd files, duplicate JSON, unlinked docs, garbage demo names) | web/diagrams/state.json, web/diagrams/use_case.json, web/diagrams/class.mmd |
| WEB2-54 | low | flaw | in-queue | Docs claim Lengauer-Tarjan dominators; the code uses Cooper-Harvey-Kennedy | web/docs/openheart_10_phase_pipeline.html:50, web/docs/scpg_architecture_diagram.html:61, web/docs/scpg_architecture_diagram.html:218 |
| WEB2-55 | low | flaw | skipped | Pages workflow: mutable action tags, persisted credentials, no path filter | .github/workflows/deploy_pages.yml:3-6, .github/workflows/deploy_pages.yml:24-37 |
| WEB2-56 | low | flaw | in-queue | No content hashing, so modules can mismatch after a deploy; duplicate theme CSS | web/index.html:24-26, web/index.html:408-414, web/style.css:1-2 |
| WEB2-57 | low | flaw | in-queue | Spinner invisible in dark mode; no color-scheme or reduced-motion support | web/style.css:547-556, web/style.css:202-223, web/themes/dark.css:5 |
| WEB2-58 | low | flaw | in-queue | Head metadata issues: verification meta, apple-touch-icon, brand link | web/index.html:11, web/index.html:17, web/index.html:43 |
| WEB2-59 | low | flaw | in-queue | Orphaned legacy modules contain fake exports and leaks (beyond WSEC-05's security sinks) | web/js/ui.js:123-129, web/js/ui.js:140-148, web/js/viewer.js:220-266 |
| WEB2-60 | low | flaw | in-queue | Touch pinch and pan are not 1:1 | web/js/graph-canvas.js:1264-1307 |
| WEB2-61 | low | flaw | in-queue | Pan Lock says 'Layout locked' but collapse and delete still edit the graph | web/js/graph-canvas.js:1114-1124, web/js/graph-canvas.js:1204-1209, web/js/graph-canvas.js:2351-2392 |
| WEB2-62 | low | flaw | in-queue | Serial GitHub fetches and fragile URL parsing | web/js/github-engine.js:10-37, web/js/github-engine.js:109-125, web/js/github-engine.js:65-80 |
| WEB2-63 | low | enhancement | in-queue | Code viewer re-renders the whole file with per-line regexes and listeners | web/js/editor.js:523-558, web/js/editor.js:575-589 |
| WEB2-64 | low | flaw | in-queue | Copy button can stick on 'Copied'; source cache collides on base names | web/js/editor.js:92-105, web/js/editor.js:37-44 |
| WEB2-67 | low | enhancement | in-queue | No node search and no shareable deep links | web/index.html:170-313, web/index.html:425 |

## lens:E2E (18)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| E2E-01 | critical | flaw | in-queue | Call resolution returns the caller itself; the error cascades into SCCs, PSA, entry points, sequence diagrams and queries | src/cg/resolution.rs, src/cg/call_sites.rs, src/cg/mod.rs |
| E2E-02 | critical | flaw | partial | The five compiler-IR diagrams and several UML views are hard-coded templates; Phase 4-8 data has no path to any exporter | src/scpg/diagram/factory.rs, src/scpg/diagram/export/json.rs, src/scpg/diagram/export/plantuml.rs |
| E2E-03 | critical | flaw | in-queue | Kotlin and every non-Java language are parsed by different grammars in different phases; Phase 3 covers this with a raw-token keyword sweep | src/ingestion/adapter/kotlin.rs, src/ingestion/adapter/registry.rs, src/ast/adapter/registry.rs |
| E2E-04 | high | flaw | in-queue | Phase 5 ignores the symbol table: SSA variables are AST node ids, so def-use chains and phi functions are empty by construction | src/ssa/mod.rs, src/ssa/liveness.rs, src/ssa/renaming.rs |
| E2E-05 | high | flaw | in-queue | No typed ID discipline: ids from one space are read as another at six phase boundaries | src/cg/points_to.rs, src/uma/structural/object_diagram.rs, src/ssa/renaming.rs |
| E2E-06 | high | flaw | partial | Traceability stops at Phase 9: exporters ignore UMLLink spans and fabricate file names and line numbers | src/uma/structural/class_diagram.rs, src/scpg/diagram/export/json.rs, web/js/editor.js |
| E2E-07 | high | flaw | in-queue | No two runs produce identical artifacts or diagrams: timestamps, mtimes, HashMap order and raw path strings leak into outputs | src/ingestion/serializer.rs, src/ingestion/mod.rs, src/ast/serializer.rs |
| E2E-08 | high | flaw | partial | Four separate pipelines (CLI, server, test builder, browser) give different results for the same repository | src/main.rs, src/adapters/server.rs, src/core/pipeline.rs |
| E2E-09 | high | flaw | in-queue | Most computed IR is never consumed, and the persisted artifacts cannot be reloaded: .scpg stores counts and .ssa reads back empty | src/scpg/serializer/mod.rs, src/scpg/api/mod.rs, src/ssa/serializer.rs |
| E2E-10 | high | flaw | in-queue | Error handling is inconsistent across phases: global aborts on local faults, silent skips, panics, and invariants met by editing data | src/ingestion/mod.rs, src/ast/mod.rs, src/cfg/mod.rs |
| E2E-11 | high | flaw | partial | Name blocklists silently drop real user classes, and the F1 = 1.0 oracle excludes the same names | src/uma/structural/class_diagram.rs, ruthless_verify.py, ruthless_config.json |
| E2E-12 | high | flaw | deferred-testing-phase | End-to-end tests have no semantic oracle, so cross-phase defects pass CI | tests/scpg_tests.rs, tests/cg_tests.rs, tests/pipeline_accuracy_tests.rs |
| E2E-13 | medium | flaw | in-queue | Phase 2 re-reads and re-parses every file and re-derives token ids by position; the two token-id spaces match only by accident | src/ast/mod.rs, src/ast/reducer.rs, src/ingestion/mod.rs |
| E2E-14 | medium | flaw | in-queue | Phase 3 drops its resolution indices at the boundary, so later phases rescan symbols linearly without name resolution | src/symbol/builder.rs, src/symbol/scope_graph/mod.rs, src/symbol/serializer.rs |
| E2E-15 | medium | flaw | in-queue | The provenance hash chain is inconsistent and partly decorative; each run has two different scpg_hash values | src/tra/uml_link/hash_chain.rs, src/tra/builder.rs, src/scpg/serializer/mod.rs |
| E2E-16 | medium | flaw | in-queue | Cross-phase id fields are narrowed to u8/u16 without checks, and exporters silently keep only the first 3-12 items, with different caps per format | src/core/types/source.rs, src/core/types/token.rs, src/ingestion/builder.rs |
| E2E-17 | medium | enhancement | in-queue | No incremental recomputation: ids are positions in the whole corpus and the incremental engine is a stub | src/scpg/incremental/mod.rs, src/tra/delta/invalidation.rs, src/symbol/builder.rs |
| E2E-18 | low | flaw | in-queue | Phase 8 inverts the SCC order it says it needs and never uses callee summaries | src/psa/mod.rs, src/cg/scc.rs, src/psa/construction/mod.rs |

## lens:FIT (17)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| FIT-01 | critical | flaw | in-queue | Compiler-IR diagrams are constant templates, and the CDG hard-codes the paper's PaymentService example | src/scpg/diagram/export/json.rs, src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/mermaid.rs |
| FIT-02 | critical | flaw | deferred-testing-phase | Ground truth was co-edited with the tool: a shared 184-name exclusion list hides misses and phantoms | ruthless_config.json, ruthless_verify.py, src/uma/structural/class_diagram.rs |
| FIT-03 | critical | flaw | in-queue | The deployed Live Studio fabricates results for user repos: regex on 30 files, template diagrams, random 'verified' hashes | web/js/github-engine.js, web/js/graph-canvas.js, web/js/api.js |
| FIT-04 | high | flaw | deferred-testing-phase | F1 compares two keyword scanners on simple class names; non-Java languages use the wrong grammar | ruthless_verify.py, ruthless_config.json, src/ingestion/adapter/registry.rs |
| FIT-05 | high | flaw | in-queue | GoF detection is keyed on class-name substrings; the structural detectors that exist are never called | src/uma/patterns/factory.rs, src/uma/patterns/state.rs, src/uma/patterns/strategy.rs |
| FIT-06 | high | flaw | in-queue | The structural checks that do exist are wrong: Singleton on any self-reference, Composite identical to Decorator, Facade counts primitives | src/uma/patterns/singleton.rs, src/uma/patterns/composite.rs, src/uma/patterns/decorator.rs |
| FIT-07 | high | flaw | in-queue | Most UML behavioral views and the XMI export are templates or empty stubs presented as derived | src/uma/behavioral/state_machine.rs, src/uma/behavioral/timing_diagram.rs, src/uma/behavioral/communication_diagram.rs |
| FIT-08 | high | flaw | in-queue | Traceability in the delivered UI is fabricated, and source lookup is overfit to the example corpus | src/scpg/diagram/export/json.rs, src/adapters/server.rs, web/js/editor.js |
| FIT-09 | high | flaw | in-queue | Hard-coded name deny-lists and suffix rules silently drop real user classes | src/uma/structural/class_diagram.rs, src/scpg/diagram/common.rs, src/scpg/diagram/export/plantuml.rs |
| FIT-10 | high | flaw | in-queue | Identity is the simple class name end to end; same-named classes merge, and the benchmark cannot see it | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs, src/symbol/passes/pass4_members.rs |
| FIT-11 | high | flaw | in-queue | A missing or failed repository is silently replaced by another repo and reported as success | src/adapters/server.rs, web/index.html, scripts/verify_all_14_uml_diagrams.py |
| FIT-12 | high | flaw | deferred-testing-phase | Benchmark scripts cannot fail on wrong content; several checks are hard-wired to pass | scripts/test_ruthless_popular_datasets.py, scripts/verify_all_14_uml_diagrams.py, scripts/benchmark_all_java_design_patterns.py |
| FIT-13 | high | flaw | deferred-testing-phase | Rust tests are mostly existence and round-trip checks; obvious ground truth is unused and the 'adversarial' cases are tuned to name matching | tests/adversarial_trick_tests.rs, tests/cg_tests.rs, tests/pipeline_accuracy_tests.rs |
| FIT-14 | high | flaw | deferred-testing-phase | Design-pattern detection and class relationships are never scored against labels | scripts/test_ruthless_popular_datasets.py, scripts/test_authored_design_patterns.py, scripts/benchmark_all_java_design_patterns.py |
| FIT-15 | medium | flaw | partial | Headline numbers are unmeasured or unreproducible (128×, the F1 table, the CI badge, test counts) | README.md, docs/succinct_structures.md, docs/overview.md |
| FIT-16 | medium | enhancement | deferred-testing-phase | Build an independent, oracle-backed benchmark harness for each pipeline phase | ruthless_verify.py, scripts/benchmark_all_java_design_patterns.py, tests/ |
| FIT-17 | medium | enhancement | deferred-testing-phase | Add metamorphic rename-invariance and mutation testing to catch overfitting automatically | src/uma/patterns/query.rs, src/scpg/diagram/export/plantuml.rs, src/uma/structural/class_diagram.rs |

## lens:LANG (14)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| LANG-01 | critical | flaw | in-queue | Every non-Java language is parsed with a foreign grammar, silently | Cargo.toml, src/ingestion/adapter/kotlin.rs, src/ingestion/adapter/registry.rs |
| LANG-02 | critical | flaw | in-queue | Kotlin token heuristics run on every file and invent members and inheritance, including in Java | src/symbol/passes/pass1_discovery.rs, src/symbol/passes/pass4_members.rs, src/symbol/passes/pass5_hierarchy.rs |
| LANG-03 | high | flaw | in-queue | Phase 9 drops real classes through a repo-tuned name blocklist that the F1 oracle mirrors | src/uma/structural/class_diagram.rs, ruthless_config.json, ruthless_verify.py |
| LANG-04 | high | flaw | in-queue | Import, package and module semantics are dead or based on Java strings; types are joined by simple name | src/ast/adapter/java.rs, src/symbol/passes/pass2_imports.rs, src/symbol/scope_graph/resolver.rs |
| LANG-05 | high | flaw | in-queue | Phases 3-9 ignore the stored language and hardcode JVM semantics for every file | src/symbol/mod.rs, src/symbol/builder.rs, src/symbol/std_library/java_stubs.rs |
| LANG-06 | high | flaw | in-queue | The generic adapter is unsound even for JavaScript, the only other real grammar | src/ingestion/adapter/generic.rs, src/ingestion/adapter/mod.rs, src/ingestion/walker.rs |
| LANG-07 | high | flaw | in-queue | Position and identifier model is byte- and ASCII-based; one long line aborts Phase 1 | src/ingestion/walker.rs, src/core/types/token.rs, src/ingestion/mod.rs |
| LANG-08 | high | flaw | partial | Advertised language support is not backed by code | README.md, src/adapters/server.rs, CHANGELOG.md |
| LANG-09 | high | flaw | deferred-testing-phase | The multi-language F1 benchmark is circular, name-only and not run in CI | ruthless_verify.py, ruthless_config.json, .github/workflows/ci.yml |
| LANG-10 | high | enhancement | in-queue | No language-neutral IR: a Java-shaped AST with no node roles is the contract for every phase | src/core/types/ast.rs, src/ast/builder.rs, src/cfg/stmts/for_stmt.rs |
| LANG-11 | medium | flaw | in-queue | The adapter and registry design forces 25+ file edits per new language | src/ingestion/adapter/registry.rs, src/ast/adapter/registry.rs, src/ast/mod.rs |
| LANG-12 | medium | flaw | in-queue | Design-pattern detection and exporters rely on Java and Spring naming habits | src/uma/patterns/singleton.rs, src/uma/patterns/query.rs, src/uma/patterns/factory.rs |
| LANG-13 | medium | enhancement | partial | No polyglot model: Java-Kotlin interop and language boundaries are not represented | src/core/types/symbol.rs, src/cg/resolution.rs, src/ast/adapter/java.rs |
| LANG-14 | low | flaw | in-queue | The token types are Java-centric and doc comments are never classified | src/ingestion/adapter/java.rs, src/ingestion/adapter/generic.rs, src/core/types/source.rs |

## lens:ROB (16)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| ROB-01 | critical | flaw | in-queue | Unsound unsafe in BDDLibrary: reads through a stale slice after Vec reallocation (use-after-free) | src/psa/bdd/mod.rs, src/psa/bdd/apply.rs, src/psa/bdd/restrict.rs |
| ROB-02 | critical | flaw | in-queue | Server reads arbitrary files and analyzes arbitrary directories named in requests (no confinement, wildcard CORS, 0.0.0.0) | src/adapters/server.rs, src/adapters/web_repo.rs |
| ROB-03 | critical | flaw | in-queue | Exponential path enumeration in compute_max_path_length makes Phase 8 hang on common methods | src/psa/construction/mod.rs, src/cfg/stmts/if_stmt.rs |
| ROB-04 | critical | flaw | in-queue | Server turns every panic, abort or hang into a remote outage (single-threaded inline pipeline) | src/adapters/server.rs, src/main.rs, Dockerfile |
| ROB-05 | high | flaw | in-queue | Byte-index slicing in LabelExtractor panics on non-ASCII text, and labels come from the wrong token | src/uma/label_extraction.rs, src/uma/behavioral/activity_diagram.rs, src/ast/adapter/java.rs |
| ROB-06 | high | flaw | in-queue | 16-bit column in sort keys: one line longer than 64 KB aborts the whole repository run | src/ingestion/walker.rs, src/core/types/token.rs, src/ingestion/mod.rs |
| ROB-07 | high | flaw | in-queue | Packed u8/u16 fields silently truncate at realistic sizes, corrupting SSA, PSA, traceability and UMA data | src/core/types/ssa.rs, src/psa/construction/feasibility.rs, src/tra/backward/ssa_bi.rs |
| ROB-08 | high | flaw | in-queue | Unbounded native recursion on input-controlled depth causes stack-overflow aborts that cannot be caught | src/ingestion/walker.rs, src/ast/reducer.rs, src/cfg/stmts/mod.rs |
| ROB-09 | high | flaw | in-queue | select1 returns the root when a superblock has no 1-bits, corrupting AST navigation on long else-if chains | src/ast/rank_select.rs, src/ast/builder.rs, src/ssa/liveness.rs |
| ROB-10 | high | flaw | in-queue | Source discovery follows symlinks with no cycle, file-type or root checks, and one bad file aborts the run | src/main.rs, src/adapters/server.rs, src/ingestion/mod.rs |
| ROB-11 | medium | flaw | in-queue | Artifact readers trust header counts and offsets, and inspect reports integrity it never checks | src/tra/serializer.rs, src/psa/serializer.rs, src/uma/serializer.rs |
| ROB-12 | medium | flaw | in-queue | No work or memory budgets, and several phases scale super-linearly on realistic large inputs | src/psa/construction/constraints.rs, src/psa/bdd/mod.rs, src/psa/ordering/rpo.rs |
| ROB-13 | medium | flaw | in-queue | Phase 2 re-reads sources by a lossy path and silently drops files it cannot re-read or re-parse | src/ast/mod.rs, src/ingestion/serializer.rs, src/ingestion/mod.rs |
| ROB-14 | medium | flaw | in-queue | Error model is panic or abort-the-run: I/O panics, release-mode asserts, and per-unit failures that fail everything | src/tra/mod.rs, src/psa/mod.rs, src/uma/mod.rs |
| ROB-15 | medium | flaw | deferred-testing-phase | Overflow checks are disabled in every profile, and there is no fuzzing, Miri or sanitizer coverage | Cargo.toml, .github/workflows/ci.yml, tests/adversarial_trick_tests.rs |
| ROB-16 | low | flaw | in-queue | Hand-rolled server JSON breaks on error text and control characters, and session temp directories leak | src/adapters/server.rs, src/ingestion/mod.rs |

## lens:SCALE (19)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SCALE-01 | critical | flaw | in-queue | Phase 8 longest-path DFS enumerates every CFG path: exponential hang on ordinary methods | src/psa/construction/mod.rs, src/psa/mod.rs, src/cfg/stmts/if_stmt.rs |
| SCALE-02 | high | flaw | in-queue | Phase 6 scans the whole symbol table per call site and per AST ancestor: O(C·S·depth) | src/cg/call_sites.rs, src/cg/resolution.rs, src/cg/mod.rs |
| SCALE-03 | high | flaw | in-queue | Sequence extraction rebuilds a map of all call edges per entry point and scans all call sites per visited method | src/uma/behavioral/sequence_diagram.rs, src/uma/actor_identification.rs, src/cg/call_sites.rs |
| SCALE-04 | high | flaw | in-queue | Phase 3 resolves most type references with a full symbol-table scan (O(R·S)), plus other quadratic loops | src/symbol/passes/pass3_types.rs, src/symbol/passes/pass1_discovery.rs, src/symbol/builder.rs |
| SCALE-05 | high | flaw | in-queue | Unbounded recursion on the 1 MB main thread: deep CST, CFG, dominator, BDD or call-graph inputs abort the process | src/ingestion/walker.rs, src/ast/reducer.rs, src/cfg/stmts/if_stmt.rs |
| SCALE-06 | high | flaw | in-queue | Fixed-width ids overflow at scale: u16 file ids, u8 block ids, u16 counts | src/ingestion/mod.rs, src/core/types/token.rs, src/ingestion/builder.rs |
| SCALE-07 | high | flaw | in-queue | SSA liveness and renaming are quadratic per method: per-occurrence variable ids give O(blocks × uses) live sets | src/ssa/liveness.rs, src/ssa/renaming.rs, src/ssa/version_stack.rs |
| SCALE-08 | high | flaw | partial | The 'succinct' AST costs about 210–236 B/node, roughly 45× the documented 4.65 B, and select1 scans bit by bit | src/ast/rmq.rs, src/ast/jump_table.rs, src/ast/rank_select.rs |
| SCALE-09 | high | flaw | in-queue | ROBDD engine: use-after-realloc UB in apply, no node budget or GC, O(E²) edge lookups, no-op O(n²) sifting | src/psa/bdd/mod.rs, src/psa/bdd/apply.rs, src/psa/ordering/rpo.rs |
| SCALE-10 | high | flaw | in-queue | Single-threaded pipeline that reads every file three times and parses it twice | Cargo.toml, src/core/pipeline.rs, src/main.rs |
| SCALE-11 | high | flaw | partial | No incremental or persistent reuse: .scpg is a stub, the delta engine is a no-op, and global ids renumber on every edit | src/scpg/serializer/mod.rs, src/scpg/mmap/mod.rs, src/scpg/incremental/mod.rs |
| SCALE-12 | high | flaw | in-queue | HTTP server runs analyses synchronously on a single-threaded accept loop with no budgets, queue or cache | src/adapters/server.rs |
| SCALE-13 | high | enhancement | deferred-testing-phase | No scale benchmarks, perf CI or release tuning; published performance figures are unmeasured | Cargo.toml, tests, examples/test_patterns_codebase |
| SCALE-14 | medium | flaw | in-queue | Logger writes every level, including trace, to two unbuffered files from hot loops | src/core/logger.rs, src/cg/call_sites.rs, src/cfg/mod.rs |
| SCALE-15 | medium | flaw | in-queue | Bitwise CRC-64 recomputed 19 times over re-read artifacts, whole-file buffers, and duplicate token tables | src/ingestion/serializer.rs, src/scpg/serializer/integrity.rs, src/tra/uml_link/hash_chain.rs |
| SCALE-16 | medium | flaw | in-queue | Query engine scans every call edge per worklist pop instead of using the CSR it already has | src/scpg/query/cfl.rs, src/scpg/query/impact.rs, src/scpg/query/slice.rs |
| SCALE-17 | medium | flaw | in-queue | Remaining quadratic helpers: per-function CSR builds, adjacency dedup, per-class TH scans, O(depth) node closes | src/cfg/builder/mod.rs, src/cg/mod.rs, src/uma/structural/class_diagram.rs |
| SCALE-18 | medium | flaw | partial | Web studio's in-browser ingestion samples 30–80 files sequentially and reports them as the whole repo | web/js/github-engine.js, web/js/graph-canvas.js, web/js/uml-layout.js |
| SCALE-19 | medium | enhancement | in-queue | Exporters emit monolithic, unpartitioned diagrams and recompute shared indexes 57 times | src/scpg/diagram/export/plantuml.rs, src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs |

## lens:TST (21)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| TST-01 | critical | flaw | in-queue | SSA produces no φ-functions or def-use edges for any input; the SSA tests check only counts and CSR shape | src/ssa/liveness.rs, src/ssa/placement.rs, src/ssa/renaming.rs |
| TST-02 | critical | flaw | in-queue | Call resolution returns the enclosing method, so the call graph is mostly self-loops; no test asserts a single edge | src/cg/resolution.rs, src/cg/call_sites.rs, src/cg/mod.rs |
| TST-03 | critical | flaw | in-queue | The HTTP server has zero tests; /api/source serves arbitrary files and /api/analyze runs git clone on unvalidated input | src/adapters/server.rs, src/adapters/web_repo.rs |
| TST-04 | high | flaw | in-queue | Succinct-tree LCA is wrong on most queries and select1 breaks on deep nesting; the tests use 3-node and 6-bit toys | src/ast/rmq.rs, src/ast/rank_select.rs, src/ast/builder.rs |
| TST-05 | high | flaw | partial | BDDLibrary reads through an unsound raw slice that dangles when the node Vec grows; no Miri or sanitizer job | src/psa/bdd/mod.rs, src/psa/bdd/unique_table.rs, src/psa/construction/mod.rs |
| TST-06 | high | flaw | in-queue | `_ => Drop` silently deletes constructor bodies and other constructs, and a test pins the defect as expected | src/ast/adapter/java.rs, src/ast/reducer.rs, src/cfg/mod.rs |
| TST-07 | high | flaw | deferred-testing-phase | IFDS, CDG, liveness, points-to, SCC, post-dominators and loops have zero semantic tests; 'IFDS' is a local pattern match | src/ssa/ifds.rs, src/ssa/cdg.rs, src/ssa/liveness.rs |
| TST-08 | high | flaw | deferred-testing-phase | SCPG and exporter tests are vacuous: tautological asserts, a counts-only .scpg file, placeholder XMI, no golden outputs | tests/scpg_tests.rs, src/scpg/serializer/mod.rs, src/scpg/mmap/mod.rs |
| TST-09 | high | flaw | partial | The README's F1 = 1.0000 claim comes from a circular, hand-tuned Python harness that never runs in CI | ruthless_verify.py, ruthless_config.json, scripts/test_ruthless_popular_datasets.py |
| TST-10 | high | flaw | deferred-testing-phase | Pattern-detection tests are satisfied by identifier names; there are no negative fixtures and no precision measurement | src/uma/patterns/strategy.rs, src/uma/patterns/facade.rs, src/uma/patterns/decorator.rs |
| TST-11 | high | flaw | deferred-testing-phase | No benchmark or memory harness; static accounting contradicts '128× compression' and 'O(1)'; pathological inputs untested | src/ast/rmq.rs, src/ast/jump_table.rs, src/ast/rank_select.rs |
| TST-12 | high | enhancement | deferred-testing-phase | Coverage map and target test architecture for benchmark grade | tests/, src/, Cargo.toml |
| TST-13 | medium | flaw | deferred-testing-phase | BDD tests cover terminal shortcuts only; sifting is a no-op; a PSA 'invariant' compares a value with its own copy | src/psa/bdd/apply.rs, src/psa/bdd/sat_count.rs, src/psa/bdd/restrict.rs |
| TST-14 | medium | flaw | in-queue | Integrity checks are untested and partly fake: 'VERIFIED VALID' without a CRC, a non-standard PSA CRC, a count-based hash chain | src/main.rs, src/psa/serializer.rs, src/tra/uml_link/hash_chain.rs |
| TST-15 | medium | flaw | deferred-testing-phase | Outputs are non-deterministic (wall clock, file mtime, HashMap order), which blocks golden tests; no determinism test exists | src/ingestion/serializer.rs, src/scpg/serializer/mod.rs, src/ingestion/mod.rs |
| TST-16 | medium | flaw | in-queue | Artifact loaders and the request parser are unfuzzed; malformed files with a valid CRC panic or abort | src/ast/serializer.rs, src/scpg/mmap/mod.rs, src/uma/serializer.rs |
| TST-17 | medium | flaw | partial | CI is a single Linux check/fmt/test job; releases and Pages deploys are ungated, and overflow checks are off in tests | .github/workflows/ci.yml, .github/workflows/release.yml, .github/workflows/deploy_pages.yml |
| TST-18 | medium | flaw | deferred-testing-phase | The web studio's 9,035 lines of JS have no tests, and the deployed path fabricates classes and timings | web/js/github-engine.js, web/js/puml-parser.js, web/js/graph-canvas.js |
| TST-19 | medium | flaw | deferred-testing-phase | Many tests are tautological, print instead of asserting, or check invariants that hold by construction | src/psa/metrics.rs, src/psa/serializer.rs, tests/design_pattern_architecture_tests.rs |
| TST-20 | medium | enhancement | deferred-testing-phase | Adopt cargo-mutants as a quality gate; predicted surviving mutants show where assertions are missing | src/ssa/placement.rs, src/cg/resolution.rs, src/ssa/ifds.rs |
| TST-21 | low | flaw | deferred-testing-phase | The only nested-package test is #[ignore]d with a misleading comment, and tests have no timeouts | tests/scpg_tests.rs, src/adapters/server.rs, src/symbol/scope_graph/resolver.rs |

## lens:WSEC (8)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| WSEC-01 | critical | flaw | fixed | DOM XSS: attacker-controlled repo/ZIP file paths and names flow unescaped into innerHTML (and persist via localStorage history) | web/js/github-engine.js, web/index.html |
| WSEC-02 | high | flaw | partial | No Content-Security-Policy, no Trusted Types, and no SRI on five CDN scripts | web/index.html, .github/workflows/deploy_pages.yml |
| WSEC-03 | high | flaw | fixed | ReDoS: catastrophic backtracking in the in-browser method/field parser regexes freezes the tab | web/js/github-engine.js |
| WSEC-04 | medium | flaw | fixed | Loaded .openheart.json / sessionStorage state consumed with no schema validation (innerHTML and fetch sinks) | web/index.html, web/js/graph-canvas.js, web/js/editor.js |
| WSEC-05 | medium | flaw | fixed | Orphaned modules shipped to production include a latent raw-innerHTML of third-party Kroki SVG and code egress | web/app.js, web/js/viewer.js, web/js/api.js |
| WSEC-06 | low | flaw | partial | No clickjacking / framing protection on the studio page | web/index.html, .github/workflows/deploy_pages.yml |
| WSEC-07 | low | enhancement | fixed | Systemic unsafe innerHTML string-templating pattern across the studio | web/index.html, web/js/minimap-navigator.js |
| WSEC-08 | low | flaw | fixed | Ad-hoc syntax highlighter emits malformed nested HTML on annotations and strings | web/js/editor.js |

## verify:SRV:1 (3)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| SRV-V1-01 | high | flaw | fixed | Binary ignores HOST/PORT env and hardcodes 0.0.0.0 bind; no localhost-only option | src/adapters/server.rs:32, src/main.rs:532-538, Dockerfile:43-44 |
| SRV-V1-02 | medium | flaw | fixed | Failed git clone silently analyzes the first cached repo and returns success for the wrong repository | src/adapters/server.rs:418-459, src/adapters/server.rs:467-482 |
| SRV-V1-03 | medium | flaw | fixed | validate_url panics on unchecked parts[1] for URLs lacking a repo segment | src/adapters/web_repo.rs:111-112 |

## verify2:COMP:3 (2)

| ID | Severity | Kind | Status | Title | Files |
|---|---|---|---|---|---|
| COMP-R3-1 | critical | flaw | partial | /api/source reads any file the server can access: unauthenticated arbitrary file read on 0.0.0.0 with CORS * | src/adapters/server.rs:95-240, src/adapters/server.rs:32, src/adapters/server.rs:327 |
| COMP-R3-2 | high | flaw | partial | Studio silently replaces Rust-engine output with a 30-file regex sampler and labels it 'SCPG ... Verified' | web/index.html:862, web/index.html:881, web/index.html:912-935 |

