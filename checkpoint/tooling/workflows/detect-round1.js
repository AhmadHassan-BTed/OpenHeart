export const meta = {
  name: 'openheart-audit-find',
  description: 'Nine parallel read-only auditors sweep OpenHeart subsystems for flaws and benchmark-grade enhancements',
  phases: [{ title: 'Find', detail: '9 area auditors, read-only, evidence-backed findings' }],
}

const PREAMBLE = `CONTEXT: OpenHeart (repo root D:\\Repos\\OpenHeart on Windows; in Bash use /d/Repos/OpenHeart; Read/Grep/Glob tools also work) is a Rust (edition 2021, ~28.7K LOC in src/) static program-analysis engine. A 10-phase pipeline runs over Java/Kotlin sources: (1) tree-sitter lexical ingestion -> token corpus .tca, (2) succinct balanced-parentheses AST .bpa, (3) symbol table / scope graph .sta, (4) CFG in CSR + dominators .cfa, (5) SSA / def-use / CDG / IFDS .ssa, (6) call graph + Andersen points-to .cga, (7) traceability index .tra, (8) ROBDD path summaries .psa, (9) UML semantic metadata + GoF design-pattern detection .uma, (10) unified memory-mapped .scpg + query engine + diagram exporters (JSON / Mermaid / PlantUML / XMI). There is a hand-rolled HTTP server (src/adapters/server.rs), a CLI (src/main.rs), and a ~9K-line vanilla-JS web studio in web/ deployed to GitHub Pages. The README makes strong claims: 128x memory compression, O(1) bidirectional traceability, F1 = 1.0000 across Java/Kotlin/Python/Rust/TypeScript/C++, all 14 UML 2.5 diagrams plus 5 compiler IRs.

GOAL: The owner wants OpenHeart to become benchmark-grade, best-in-class software. You are one of nine auditors; each owns one area. Find (a) FLAWS that must be fixed: bugs, incorrect algorithms, security holes, crashes/panics on realistic input, silently wrong results, performance cliffs, claims the code does not back up; and (b) ENHANCEMENTS that would lift your area to the state of the art: algorithmic upgrades, missing capabilities, architecture/design improvements, test and benchmark coverage. Measure against the leading tools and literature named for your area.

RULES:
- Read the actual code. Every finding must cite repo-relative file:line references and state what the code actually does (short quotes are fine). Do not present speculation as fact; if a point is a hunch, set confidence to low.
- No Rust toolchain is installed: you cannot compile or run cargo. Reason statically. Node and Python are installed if a quick experiment helps (for example feeding crafted input to a JS function); put any scratch files under /c/Users/AHMAD_~1/AppData/Local/Temp/claude/D--Repos-OpenHeart/31873741-1e3a-46df-bb5d-a94a867cbe24/scratchpad/audit (create it if needed).
- The repository is strictly read-only for you: do not edit, create or delete any file inside D:\\Repos\\OpenHeart, run no git command that changes state, start no servers, clone nothing, fetch no URLs. An optional WebSearch to confirm a current library version or paper reference is fine.
- Prioritize by impact. Skip cosmetic nits unless systemic. Do not pad: 8 to 25 well-evidenced findings beat 50 shallow ones. Merge duplicates (one finding may list several locations).
- Severity: critical = exploitable security hole in the default deployment, data loss, or core output wrong for common input; high = wrong results or crash on realistic input, or a major gap blocking benchmark-grade status; medium = notable defect or gap with a workaround; low = minor.
- Recommendations must be concrete: name the algorithm, paper, crate, standard or tool to adopt, and what done looks like.
- Keep text fields tight: evidence and recommendation about 90 words max each, impact about 40 words.
- Return findings ordered most severe first. Finding ids use your area prefix, e.g. PREFIX-01, PREFIX-02.
- In summary, give a frank 4 to 8 sentence maturity assessment of your area against the state of the art, including what is genuinely good.`

const FINDING = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    kind: { type: 'string', enum: ['flaw', 'enhancement'] },
    category: { type: 'string', enum: ['security', 'correctness', 'robustness', 'performance', 'scalability', 'architecture', 'maintainability', 'testing', 'benchmarking', 'documentation', 'ux', 'accessibility', 'devops', 'dependency', 'feature-gap', 'claims-vs-reality'] },
    severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
    title: { type: 'string' },
    files: { type: 'array', items: { type: 'string' }, description: 'repo-relative path:line references' },
    evidence: { type: 'string' },
    impact: { type: 'string' },
    recommendation: { type: 'string' },
    effort: { type: 'string', enum: ['S', 'M', 'L', 'XL'] },
    confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
  },
  required: ['id', 'kind', 'category', 'severity', 'title', 'files', 'evidence', 'impact', 'recommendation', 'effort', 'confidence'],
}

const SCHEMA = {
  type: 'object',
  properties: {
    area: { type: 'string' },
    summary: { type: 'string' },
    strengths: { type: 'array', items: { type: 'string' }, description: 'Up to 5 genuinely good aspects worth keeping' },
    findings: { type: 'array', items: FINDING },
  },
  required: ['area', 'summary', 'strengths', 'findings'],
}

const AREAS = [
  {
    code: 'CORE',
    name: 'Core types, binary I/O, pipeline, Phase 1 ingestion, Phase 2 succinct AST',
    prompt: `SCOPE: src/core/** (types/*, io/binary.rs, io/mmap.rs, pipeline.rs, logger.rs), src/ingestion/** (parser/tree_sitter.rs, walker.rs, interner.rs, allocator.rs, manifest.rs, builder.rs, serializer.rs, adapter/{java,kotlin,generic,registry}.rs), src/ast/** (bp_encoder.rs, rank_select.rs, rmq.rs, jump_table.rs, preorder.rs, reducer.rs, builder.rs, serializer.rs, adapter/*), tests/ingestion_tests.rs, tests/ast_tests.rs, and docs/succinct_structures.md for the claimed design.
LOOK FOR:
- Succinct-structure correctness and real complexity: BP encoding; rank/select (Jacobson/Clark-style o(n) overhead with O(1) queries, or linear scans?); word-boundary and off-by-one bugs; select on absent ranks; find_close / enclose / LCA via RMQ; whether a sparse-table RMQ (O(n log n) words) defeats the succinct and 128x-compression claims (compare range min-max trees, Navarro and Sadakane 2014).
- Binary artifact I/O (io/binary.rs, io/mmap.rs, the serializers): endianness, alignment, versioning, bounds checks when loading corrupt or untrusted files (panics or UB), soundness of every unsafe block (grep for unsafe), mmap of files that may change underneath.
- Ingestion: tree-sitter usage (crate 0.20 is years old; ERROR/MISSING node handling; incremental re-parse), token record layout, string interner (FNV-1a: are collisions detected by comparing bytes?), allocator, non-UTF-8 / BOM / CRLF input, walker (symlink loops, huge files, .gitignore, generated code), ordering determinism, whether the Kotlin adapter is real given Cargo.toml has no tree-sitter-kotlin dependency, parallelism (no rayon) and throughput.
- pipeline.rs and logger.rs: stage abstraction, error propagation, global mutable state, logging cost on hot paths.
COMPARE AGAINST: sdsl-lite, the sucds / vers-vecs / succinct crates, Navarro "Compact Data Structures" (2016), tree-sitter best practice, rust-analyzer rowan syntax trees.`,
  },
  {
    code: 'SYM',
    name: 'Phase 3 symbol table, scope graph and name resolution',
    prompt: `SCOPE: src/symbol/** (passes/pass1_discovery.rs through pass5_hierarchy.rs, scope_graph/{mod,resolver}.rs, qual_name_table.rs, std_library/{mod,java_stubs}.rs, uml_meta/{associations,patterns}.rs, builder.rs, serializer.rs, adapter/java.rs), src/core/types/symbol.rs, tests/symbol_tests.rs.
LOOK FOR, judged against Java name-resolution rules (JLS chapter 6 scopes/shadowing/obscuring and 15.12 method invocation / overload resolution):
- imports: single-type, on-demand, static single and on-demand, implicit java.lang, same-package visibility, conflicts.
- nested, inner, local and anonymous classes; enums, records, interfaces with default/static methods, sealed types, annotations; generics (type parameters, bounds, wildcards, raw types, erasure); varargs; lambdas and method references; field vs local vs parameter shadowing; inherited member lookup through superclasses and interfaces; overload resolution (or matching by name/arity only).
- the hand-written JDK stubs: coverage vs the real JDK; better sources (ct.sym / jmods / classpath jars, Maven/Gradle classpath resolution).
- unresolved or ambiguous symbols (silent guesses vs explicit unknown), determinism, cross-file resolution, scalability (quadratic loops over all symbols?), memory layout.
- UML association inference in uml_meta (multiplicity from collections/arrays/Optional, aggregation vs composition heuristics, bidirectionality).
- pass1_discovery.rs is about 950 lines: look for string matching on raw source text instead of using the syntax tree, and hardcoded names.
COMPARE AGAINST: javac Attr/Resolve, Eclipse JDT, JavaParser SymbolSolver, Spoon, scope graphs (Neron et al. 2015, Statix), GitHub stack-graphs, scip-java.`,
  },
  {
    code: 'FLOW',
    name: 'Phases 4-6: CFG and dominators, SSA / CDG / IFDS, call graph and points-to',
    prompt: `SCOPE: src/cfg/** (builder/{mod,state}.rs, stmts/*.rs, analysis/{dominators,postdom,frontier,loops,rpo}.rs, serializer.rs), src/ssa/** (placement, renaming, version_stack, liveness, cdg, ifds, serializer), src/cg/** (call_sites, resolution, points_to, scc, serializer), src/core/types/{cfg,ssa,cg}.rs, tests/cfg_tests.rs, tests/ssa_tests.rs, tests/cg_tests.rs.
LOOK FOR:
- CFG completeness and correctness for Java: try/catch/finally (finally on every exit including return/break/continue/throw; exceptional edges from calls and throwing operations), try-with-resources, labeled break/continue, switch fall-through, arrow switch / switch expressions / pattern matching / yield (Java 14-21), enhanced for, do-while, short-circuit && || and the ternary operator, synchronized, lambdas and local classes, unreachable code, infinite loops.
- Dominators (Cooper-Harvey-Kennedy: unreachable nodes, fixpoint iteration, RPO numbering), post-dominators with multiple exits or no exit (virtual exit node), dominance frontiers, natural-loop detection and irreducible loops.
- SSA: phi placement (Cytron et al.; minimal vs pruned / semi-pruned via liveness), renaming with version stacks, fields, arrays, captured variables, exceptions; def-use chains; CDG via post-dominance frontiers; liveness fixpoint correctness.
- IFDS: a real Reps-Horwitz-Sagiv tabulation solver (path edges, summary edges, call/return flow functions) or a stub / intraprocedural approximation?
- Call graph: CHA vs RTA / VTA / points-to based; virtual and interface dispatch including default methods; constructors, static initializers, super calls, lambdas and method references, library calls; Andersen points-to (field sensitivity, worklist, cycle elimination, difference propagation, complexity); SCC (Tarjan correctness; recursion depth leading to stack overflow on large graphs).
- Stack-overflow risks from recursion and quadratic algorithms anywhere in these phases.
COMPARE AGAINST: Soot / SootUp, WALA, Doop, OPAL, Joern, PhASAR, Heros (IFDS), LLVM and Cranelift (dominators; SSA construction per Braun et al. 2013).`,
  },
  {
    code: 'BDD',
    name: 'Phase 7 traceability index and Phase 8 ROBDD path summaries',
    prompt: `SCOPE: src/psa/** (bdd/{mod,node,unique_table,apply,restrict,sat_count}.rs, construction/{mod,constraints,feasibility,recursive}.rs, ordering/{mod,force,rpo,sifting}.rs, builder.rs, metrics.rs, serializer.rs, types.rs), src/tra/** (forward/{cs_span,sym_span}.rs, backward/*.rs, delta/{mod,invalidation}.rs, uml_link/{mod,hash_chain}.rs, builder.rs, serializer.rs, types.rs), tests/psa_tests.rs, tests/tra_tests.rs, tests/research_paper_theorems_tests.rs, docs/traceability_and_incremental.md.
LOOK FOR:
- ROBDD canonicity and correctness: unique-table hashing and equality, the reduction rule (lo == hi), terminal handling, apply memoization (computed table) and its keys, restrict/cofactor correctness, complement edges, variable-order invariants; sat_count must account for skipped levels (multiply by 2^gap) and variables above the root; overflow (u64 vs big integers); node limits or timeouts against exponential blow-up; garbage collection / reference counting.
- Variable ordering: is sifting real Rudell sifting with in-place adjacent-level swaps, or a heuristic re-run? FORCE heuristic correctness.
- Path-summary construction soundness: how branch predicates become BDD variables; loops and recursion (unrolling? widening?); whether exact #SAT path counting really equals the feasible path count (in general it does not: correlated predicates, loops); what feasibility checks actually do (any constraint solving / SMT?).
- Traceability: is O(1) bidirectional lookup really O(1) (dense arrays / hash maps) or scans; forward/backward consistency (bijection claims); delta-invalidation soundness for incremental updates; hash_chain (which hash, collision handling, what integrity it really gives).
- What the research_paper_theorems tests actually assert: do they prove anything or are they tautologies?
COMPARE AGAINST: CUDD, Sylvan, BuDDy, oxidd and biodivine-lib-bdd (Rust), KLEE-style path counting, Z3-backed path feasibility, Kythe / SCIP cross-references, Salsa (rust-analyzer) incremental invalidation.`,
  },
  {
    code: 'UML',
    name: 'Phase 9 UML extraction (14 diagrams) and GoF design-pattern detection',
    prompt: `SCOPE: src/uma/** (structural/{class,component,composite,object,package}_diagram.rs; behavioral/{activity,communication,interaction_overview,sequence,state_machine,timing}_diagram.rs; patterns/*.rs (adapter, builder, chain, composite, decorator, facade, factory, observer, query, singleton, state, strategy, template_method); actor_identification.rs, label_extraction.rs, visitor.rs, builder.rs, serializer.rs, types.rs), docs/uml_derivation.md, tests/uma_tests.rs, tests/design_pattern_architecture_tests.rs, tests/strategy_diagram_tests.rs, tests/adversarial_trick_tests.rs, tests/pipeline_accuracy_tests.rs, examples/** (Java samples used as ground truth), sample_codebase/**.
LOOK FOR:
- Semantic grounding: for each of the 14 UML diagrams, is content derived from real program facts, or templated / fabricated? Deployment, timing, object, use-case and state-machine diagrams are largely not recoverable from static source: what does the code emit, and is it labeled heuristic? Fabricated or placeholder content presented as analysis is a flaw.
- Class-diagram fidelity: generics, inner classes, interfaces vs abstract classes, static members, multiplicities, dependency vs association vs aggregation vs composition, enum literals, records.
- Sequence / communication diagrams: derived from the call graph with correct message order, alt/opt and loop fragments, async, return messages, depth limits; activity diagrams from the CFG (decision/merge, fork/join).
- Design-pattern detection: name/keyword heuristics vs structural plus behavioral role matching; false positives (e.g. any getInstance method flagged Singleton) and false negatives (enum singleton, lazy holder, double-checked locking, generic observers); role assignment; confidence scores; overlap / conflict resolution.
- Test quality: do tests assert ground truth (expected roles and participants) or only non-emptiness? Any precision/recall measurement against a labeled corpus?
COMPARE AGAINST: Tsantalis et al. 2006 (similarity-scoring DPD), DeMIMA, PINOT, FINDER, DPDf, the P-MARt and DPB pattern benchmarks; reverse engineering in Visual Paradigm, Enterprise Architect, IntelliJ, ObjectAid; OMG UML 2.5.1 semantics.`,
  },
  {
    code: 'SCPG',
    name: 'Phase 10 unified SCPG binary, query engine, incremental deltas, diagram exporters',
    prompt: `SCOPE: src/scpg/**: api/{mod,builder}.rs, serializer/{mod,integrity,layout}.rs, mmap/mod.rs, incremental/{mod,delta}.rs, query/{mod,cache,cfl,impact,navigation,robdd,slice}.rs, diagram/{mod,common,factory,renderers}.rs, diagram/export/{json,mermaid,plantuml,plantuml_optimizer,xmi}.rs (about 5.8K of the 7.5K lines), types.rs; tests/scpg_tests.rs, tests/surgical_pipeline_dump_test.rs; the sample outputs web/diagrams/*.puml, *.mmd, *.json to judge output validity.
LOOK FOR:
- Integrity and format: which CRC-64 polynomial, is it verified on load and are mismatches fatal; versioning / backward compatibility; header and layout bounds checks against corrupt or hostile files; alignment / endianness; is zero-copy mmap real or does loading copy into Vecs; mmap soundness.
- Query engine: is CFL-reachability a real Dyck/CFL algorithm over an interprocedural graph (Reps 1998) or plain BFS; is slicing a real SDG two-phase slice (Horwitz-Reps-Binkley) or an intraprocedural approximation; impact analysis; cache invalidation correctness; complexity.
- Incremental / delta: soundness and granularity.
- Exporters: escaping and injection for identifiers, generics <>, quotes, newlines, @enduml / end keywords, Mermaid reserved words, XML special characters; syntax validity of each Mermaid and PlantUML diagram type (check the samples in web/diagrams); XMI conformance (OMG XMI 2.5.1 / UML 2.5.1 namespaces, xmi:id uniqueness, importability into Papyrus / Enterprise Architect); deterministic output (HashMap iteration order); duplication across the three exporters (extract a shared diagram IR plus visitors?); what plantuml_optimizer does; large-graph handling (size limits, clustering, filtering).
COMPARE AGAINST: Joern CPG / flatgraph, CodeQL databases, Kythe, SCIP / LSIF, Glean; the PlantUML and Mermaid grammars; Eclipse Papyrus XMI; Reps CFL-reachability; Horwitz-Reps-Binkley slicing.`,
  },
  {
    code: 'SRV',
    name: 'HTTP server, git-clone adapter, CLI and deployment surface (security and robustness)',
    prompt: `SCOPE: src/adapters/server.rs, src/adapters/web_repo.rs, src/adapters/mod.rs, src/main.rs, Dockerfile, run.sh, run.bat, scripts/restart_server.sh, .env.example, SECURITY.md, docs/security-architecture.md.
ALREADY OBSERVED BY THE LEAD (include each with exact line references and a concrete exploit or failure scenario, then go well beyond): /api/source reads arbitrary paths because the decoded file param is used directly as a candidate path; the server binds 0.0.0.0 with CORS *; the accept loop handles connections serially on one thread; each request is a single 8 KiB read with a 500 ms timeout; git clone of a user-supplied URL (SSRF, argument injection if the URL starts with '-', file:// and other transports, repo_name '..' traversal, unbounded disk use, no cleanup); WebRepoFetcher::validate_url indexes parts[1] (panic); silent fallback to the first directory in ./target_repos (returns the analysis of a different repository); hardcoded dev paths (FractalAndroid, examples).
ALSO LOOK FOR: HTTP parsing correctness (Content-Length, bodies larger than one read, chunked, keep-alive, HEAD responses with bodies, percent-decoding done by three replace calls); DoS (huge requests, slowloris, CPU-heavy analysis per request with no limits, queue or timeout); panics from unwrap killing the whole server; response-header injection; missing security headers (CSP, X-Content-Type-Options); static-file traversal; sensitive data in logs; Docker hardening (root user, no healthcheck, the '|| true' dependency-cache trick swallowing errors, rust:1.80 base vs the README's 1.75+ claim); CLI UX (hand-rolled argument parsing vs clap, exit codes, usage text vs real subcommands, e.g. 'server' in main.rs vs the Dockerfile's 'serve --port'); duplication of the pipeline orchestration between main.rs and server.rs.
COMPARE AGAINST: axum / hyper / tower-http best practice (timeouts, body limits, concurrency limits), OWASP ASVS, Rust CLI guidelines (clap), container hardening (distroless, non-root, read-only FS).`,
  },
  {
    code: 'WEB',
    name: 'Web studio frontend (vanilla JS canvas app on GitHub Pages)',
    prompt: `SCOPE: web/index.html, web/app.js, web/js/** (api, editor, file-tree, github-engine, graph-canvas (2.4K lines), graph-loader, icons, logger, minimap-navigator, orb, puml-parser, state, ui, uml-card-renderer, uml-layout, viewer, themes/*), web/style.css, web/themes/*.css, web/diagrams/manifest.json, .github/workflows/deploy_pages.yml.
LOOK FOR:
- XSS: untrusted content from analyzed GitHub repositories (class names, comments, string literals, file paths, branch names) and from loaded .openheart.json state files reaching innerHTML / insertAdjacentHTML / outerHTML / template strings / attribute interpolation. Trace a concrete source-to-sink path for each case.
- GitHub ingestion (github-engine.js): rate limits, token handling and storage (localStorage?), large repositories (tree API truncated flag, pagination), binary or huge files, error handling, whether it re-implements parsing in JS (a second, divergent analyzer from the Rust engine?).
- puml-parser.js: regex-based parsing, ReDoS-prone patterns, grammar coverage; consider a quick Node experiment.
- Rendering performance at scale (thousands of nodes and edges): layout algorithm complexity, spatial indexing for hit tests, culling, re-render frequency, requestAnimationFrame loops that never stop, memory leaks from listeners; compare to ELK / elkjs, dagre, Graphviz WASM, Sugiyama-style layered layout.
- State management, undo/redo correctness, error handling, module structure, no tests / lint / types / bundler; CSP and SRI for any CDN scripts; accessibility (keyboard navigation, ARIA, focus, contrast, screen readers); responsiveness; offline / PWA.
- You may run node --check on JS files and small Node experiments in the scratch directory.
COMPARE AGAINST: Excalidraw / tldraw-class canvas engineering, elkjs layered layouts, Mermaid Live Editor, PlantUML server, Sourcetrail, CodeSee, WCAG 2.2 AA.`,
  },
  {
    code: 'ENG',
    name: 'Engineering infrastructure, test strategy, benchmarking, dependencies, docs and claims, cross-cutting architecture',
    prompt: `SCOPE: tests/*.rs (the overall test strategy; other auditors check area-specific test content, you judge the suite as a whole), .github/workflows/{ci,release,deploy_pages}.yml, .github/dependabot.yml, Cargo.toml, Cargo.lock (dependency freshness), Makefile, scripts/*.py and scripts/*.sh, ruthless_verify.py, ruthless_config.json, README.md, ROADMAP.md, CHANGELOG.md, docs/** (architecture.md, overview.md, technical-decisions.md, succinct_structures.md, traceability_and_incremental.md, uml_derivation.md, plans/*.md, openheart_research_paper.tex, RESEARCH_PAPER_NEXT_PROMPT.txt), examples/README.md, .gitignore, .editorconfig.
LOOK FOR:
1. Test strategy: only 29 #[test] integration tests plus 16 inline test modules for about 28.7K LOC. Coverage gaps, assertion quality (tests that only print or check non-emptiness), and the absence of property-based tests (proptest), fuzzing (cargo-fuzz for parsers and binary loaders), snapshot tests (insta) for exporters, differential testing against reference tools, golden datasets.
2. CI: no clippy, no -D warnings, no MSRV job, no coverage (cargo-llvm-cov), no cargo-deny / cargo-audit, no benchmark regression tracking, unpinned actions (dtolnay/rust-toolchain@master; tags instead of SHAs), workflow permissions, cache keyed only on the lockfile, release-workflow quality, Linux-only matrix.
3. Cargo.toml: misleading description ('cardiac/code processing'), missing metadata (license, repository, rust-version, readme, keywords), [profile.dev] overflow-checks = false (hides overflow bugs in bit-twiddling code during tests), no release-profile tuning (lto, codegen-units), tree-sitter 0.20 outdated, possibly unused dependencies (tree-sitter-javascript?), no features or workspace split.
4. Benchmark readiness: any reproducible performance benchmark (criterion / divan / iai-callgrind)? Accuracy evaluation with ground truth (precision/recall per pattern and diagram)? Pinned public datasets (scripts depend on target_repos/ which is not in the repo, so not reproducible)? Comparison against competing tools? Does any evidence support the README claims (128x compression, O(1) traceability, F1 = 1.0000 across Java/Kotlin/Python/Rust/TypeScript/C++; check which languages the code can actually ingest)? Are 'Phase X COMPLETED' statements consistent with the specs in docs/plans?
5. Repo hygiene and docs: stray root files, docs accuracy, research-paper claims vs implementation, rustdoc coverage, CONTRIBUTING accuracy, versioning (0.1.0 in Cargo.toml vs 'v2.0.0' in docs), CHANGELOG, an LLM prompt file committed under docs.
6. Cross-cutting architecture: error-handling strategy (String errors vs typed thiserror errors), about 291 .unwrap() calls in src, public API surface (everything pub, no semver discipline), pipeline orchestration, no parallelism (no rayon), configuration, custom logger vs tracing.
COMPARE AGAINST: rust-analyzer, ruff, Biome, tree-sitter, Joern and Semgrep engineering practices; the Rust API Guidelines; OpenSSF Scorecard.`,
  },
]

phase('Find')
const results = await parallel(AREAS.map(a => () =>
  agent(`${PREAMBLE}\n\nYOUR AREA: ${a.name}. Finding id prefix: ${a.code}.\n\n${a.prompt}`, {
    label: `find:${a.code}`,
    phase: 'Find',
    schema: SCHEMA,
  }).then(r => (r ? { ...r, code: a.code } : null))
))

const ok = results.filter(Boolean)
const missing = AREAS.filter(a => !ok.find(r => r.code === a.code)).map(a => a.code)
if (missing.length) log(`No result from: ${missing.join(', ')}`)
log(`${ok.reduce((n, r) => n + r.findings.length, 0)} findings across ${ok.length} areas`)
return ok
