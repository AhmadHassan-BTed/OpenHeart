export const meta = {
  name: 'openheart-audit-lenses-a',
  description: 'Cross-cutting OpenHeart auditors: end-to-end data flow, benchmark integrity / overfitting, scalability, panic and overflow robustness',
  phases: [{ title: 'Lenses', detail: '4 cross-cutting auditors over the whole repo' }],
}

const PREAMBLE = `CONTEXT: OpenHeart (repo root D:\\Repos\\OpenHeart on Windows; in Bash use /d/Repos/OpenHeart; Read/Grep/Glob tools also work) is a Rust (edition 2021, ~28.7K LOC in src/) static program-analysis engine. A 10-phase pipeline runs over Java/Kotlin sources: (1) tree-sitter lexical ingestion -> token corpus .tca, (2) succinct balanced-parentheses AST .bpa, (3) symbol table / scope graph .sta, (4) CFG in CSR + dominators .cfa, (5) SSA / def-use / CDG / IFDS .ssa, (6) call graph + Andersen points-to .cga, (7) traceability index .tra, (8) ROBDD path summaries .psa, (9) UML semantic metadata + GoF design-pattern detection .uma, (10) unified memory-mapped .scpg + query engine + diagram exporters (JSON / Mermaid / PlantUML / XMI). There is a hand-rolled HTTP server (src/adapters/server.rs), a CLI (src/main.rs), and a ~9K-line vanilla-JS web studio in web/ deployed to GitHub Pages. The README makes strong claims: 128x memory compression, O(1) bidirectional traceability, F1 = 1.0000 across Java/Kotlin/Python/Rust/TypeScript/C++, all 14 UML 2.5 diagrams plus 5 compiler IRs.

GOAL: The owner wants OpenHeart to become benchmark-grade, best-in-class software. Nine other auditors each review one subsystem; you apply one CROSS-CUTTING lens to the whole repository, looking for what falls between subsystem reviews. Find (a) FLAWS that must be fixed and (b) ENHANCEMENTS that would lift the project to the state of the art, as seen through your lens.

RULES:
- Read the actual code. Every finding must cite repo-relative file:line references and state what the code actually does (short quotes are fine). Do not present speculation as fact; if a point is a hunch, set confidence to low.
- No Rust toolchain is installed: you cannot compile or run cargo. Reason statically. Node and Python are installed if a quick experiment helps; put scratch files under /c/Users/AHMAD_~1/AppData/Local/Temp/claude/D--Repos-OpenHeart/31873741-1e3a-46df-bb5d-a94a867cbe24/scratchpad/lens (create it if needed); run Python with -I.
- The repository is strictly read-only for you: do not edit, create or delete any file inside D:\\Repos\\OpenHeart, run no git command that changes state, start no servers, clone nothing, fetch no URLs.
- Prioritize by impact. Do not pad: 8 to 25 well-evidenced findings beat 50 shallow ones. Merge duplicates (one finding may list several locations).
- Severity: critical = exploitable security hole in the default deployment, data loss, or core output wrong for common input; high = wrong results or crash on realistic input, or a major gap blocking benchmark-grade status; medium = notable defect or gap with a workaround; low = minor.
- Recommendations must be concrete: name the algorithm, paper, crate, standard or tool to adopt, and what done looks like.
- Keep text fields tight: evidence and recommendation about 90 words max each, impact about 40 words.
- Return findings ordered most severe first. Finding ids use your prefix, e.g. PREFIX-01.
- In summary, give a frank 4 to 8 sentence assessment through your lens, including what is genuinely good.`

const FINDING = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    kind: { type: 'string', enum: ['flaw', 'enhancement'] },
    category: { type: 'string', enum: ['security', 'correctness', 'robustness', 'performance', 'scalability', 'architecture', 'maintainability', 'testing', 'benchmarking', 'documentation', 'ux', 'accessibility', 'devops', 'dependency', 'feature-gap', 'claims-vs-reality'] },
    severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
    title: { type: 'string' },
    files: { type: 'array', items: { type: 'string' } },
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
    strengths: { type: 'array', items: { type: 'string' } },
    findings: { type: 'array', items: FINDING },
  },
  required: ['area', 'summary', 'strengths', 'findings'],
}

const LENSES = [
  {
    code: 'E2E',
    prompt: `LENS: End-to-end integration and data flow across all 10 phases. Finding id prefix E2E.
src/main.rs (cmd_analyze) and src/adapters/server.rs orchestrate the phases; src/core/pipeline.rs defines stages. For each phase boundary (1 to 2 to ... to 10) determine: what the phase consumes (in-memory structs or artifact files); whether it uses the upstream results or silently ignores or recomputes them (for example re-scanning raw source text instead of using the AST or symbol table); whether its outputs are consumed downstream at all (dead artifacts); where data is dropped (only the first file, class or method processed; truncation; filters by name); where ids (token_id, node ids, symbol ids, block ids) cross phases and whether they stay consistent (per-file vs global id spaces, off-by-one); which phases are stubs or emit placeholder or default data.
Check determinism end to end: HashMap/HashSet iteration order leaking into artifacts or diagrams, timestamps, absolute paths in outputs (non-reproducible results). Check error propagation: when phase k fails, does the run abort, skip, or continue silently with empty data? Check that the CLI analyze path and the server path run the same pipeline (divergence between the two). Cite file:line for every seam.
COMPARE AGAINST: how Joern, CodeQL extractors and rust-analyzer / Salsa structure multi-stage pipelines with stable ids and incremental recomputation.`,
  },
  {
    code: 'FIT',
    prompt: `LENS: Benchmark integrity: overfitting to the examples, hardcoded special cases, fabricated outputs, tautological tests. Finding id prefix FIT.
A benchmark-grade tool must generalize. Hunt for code that only works because it recognizes specific inputs:
- Grep src/ and web/js/ for identifiers from examples/, sample_codebase/ and the datasets named in scripts/ (java-design-patterns, spring-petclinic, FractalAndroid), for example NewsAgency, NewsChannel, CreditCardStrategy, PaypalStrategy, ShoppingCart, CsvDataMiner, PdfDataMiner, DataMiner, ComputerBuilder, Director, RoadLogistics, SeaLogistics, Truck, Ship, DatabaseConnectionPool, MediaAdapter, AudioPlayer, VlcPlayer, Mp4Player, Beverage, Espresso, Mocha, Whip, CondimentDecorator, VideoConversionFacade, BitrateReader, AudioMixer, Order, OrderFactory, OrderService, SingletonConfig, TransactionProcessor, TransferService, LedgerTransaction, CheckingAccount, petclinic, Owner, Vet, Pet. Any detector, diagram builder or exporter keyed on such names, or on name substrings like Strategy / Factory / Observer / Singleton / Adapter / Decorator / Facade / Builder instead of structure, is overfitting; report each with file:line.
- Hardcoded outputs: constant diagrams, canned JSON, fixed actor names, default lifelines, sample data embedded in Rust or JS and shown as analysis results.
- Tautological tests: asserting what the code trivially returns, counts >= 0, non-emptiness, output compared with itself, current output frozen as ground truth without independent labels, expectations tuned to the examples.
- The evaluation scripts (scripts/*.py, ruthless_verify.py, ruthless_config.json): how are precision, recall and F1 computed? Is the ground truth independent of the tool (or derived from its own output or from class names)? Could F1 = 1.0000 be an artifact of how the metric is defined?
For each case, say how the result would change on an unseen codebase (renamed classes, a different domain).`,
  },
  {
    code: 'SCALE',
    prompt: `LENS: Complexity, scalability and throughput across the whole pipeline. Finding id prefix SCALE.
Estimate how OpenHeart behaves on a 1M-LOC Java codebase (for example elasticsearch or intellij-community). For each phase identify the dominant algorithm and its practical complexity; quadratic-or-worse loops (nested scans over all symbols, nodes or edges, Vec::contains inside loops, linear searches where maps belong, repeated cloning or string formatting); unbounded recursion (stack overflow on deep ASTs, long else-if chains, large SCCs); guards against exponential blow-up (BDDs, path enumeration, call strings); whole-repository-in-memory designs; artifact sizes; I/O patterns (unbuffered small writes); logging overhead on hot paths; and missing parallelism (rayon over files or methods), caching or incremental reuse. Check the web studio's in-browser limits too (web/js/github-engine.js, graph-canvas.js, uml-layout.js, minimap-navigator.js). Rank by expected wall-clock and memory impact and give concrete fixes (data structure, algorithm, crate) with the expected complexity after the fix.
COMPARE AGAINST: rust-analyzer, ruff and Biome throughput engineering; Joern and CodeQL scale numbers; SootUp / OPAL call-graph construction on large corpora.`,
  },
  {
    code: 'ROB',
    prompt: `LENS: Panic safety, integer overflow and untrusted-input robustness across all of src/. Finding id prefix ROB.
Inputs OpenHeart does not control: source files (any bytes, any size, malformed or adversarial Java/Kotlin), CLI arguments and paths, artifact files passed to 'inspect' or loaded from disk, HTTP requests, cloned repositories (symlinks, huge files, deep directories). For each input class find the panic sites reachable from it: unwrap/expect on fallible operations, slice and array indexing with unchecked indices, integer arithmetic that can overflow (Cargo.toml sets overflow-checks = false for dev, so tests will not catch overflow, and release builds wrap silently), 'as' casts that truncate (usize to u32, i64 to u32), division by zero, unbounded allocations driven by length fields read from files (Vec::with_capacity(len) from untrusted headers), unsafe blocks and their invariants, and infinite loops on malformed input. Prioritize by reachability from the default deployment (the server) and from realistic source code (generated files, huge methods, unicode identifiers, non-UTF-8 bytes). For each finding give the concrete triggering input and the failing line.
COMPARE AGAINST: how rust-analyzer, tree-sitter and ruff harden against arbitrary input (cargo-fuzz / libFuzzer, checked arithmetic, error recovery, no panics on user input).`,
  },
]

phase('Lenses')
const results = await parallel(LENSES.map(l => () =>
  agent(`${PREAMBLE}\n\n${l.prompt}`, { label: `lens:${l.code}`, phase: 'Lenses', schema: SCHEMA })
    .then(r => (r ? { ...r, code: l.code } : null))
))
const ok = results.filter(Boolean)
const missing = LENSES.filter(l => !ok.find(r => r.code === l.code)).map(l => l.code)
if (missing.length) log(`No result from: ${missing.join(', ')}`)
return ok
