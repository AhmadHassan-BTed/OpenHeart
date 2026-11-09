export const meta = {
  name: 'openheart-audit-context',
  description: 'Benchmark-standard research, competitor capability matrix, and automated local checks for OpenHeart (independent of the area audit)',
  phases: [{ title: 'Context', detail: 'benchmark research, competitor matrix, automated checks' }],
}

const CONTEXT = `CONTEXT: OpenHeart (repo root D:\\Repos\\OpenHeart on Windows; in Bash use /d/Repos/OpenHeart) is a Rust (~28.7K LOC in src/) static program-analysis engine with a 10-phase pipeline over Java/Kotlin sources (tree-sitter ingestion, succinct BP AST, symbol table, CFG + dominators, SSA / CDG / IFDS, call graph + Andersen points-to, traceability index, ROBDD path summaries, UML extraction + GoF design-pattern detection, unified memory-mapped .scpg + query engine + JSON / Mermaid / PlantUML / XMI exporters), a hand-rolled HTTP server, a CLI, and a ~9K-line vanilla-JS web studio on GitHub Pages. The README claims 128x memory compression, O(1) bidirectional traceability, F1 = 1.0000 across six languages, and all 14 UML 2.5 diagrams. The owner wants it to become benchmark-grade, best-in-class software. Other auditors are separately reviewing the code area by area; your job is the specific task below.
RULES: The repository is read-only for you: never edit, create or delete files inside D:\\Repos\\OpenHeart and run no state-changing git commands. Scratch files go under /c/Users/AHMAD_~1/AppData/Local/Temp/claude/D--Repos-OpenHeart/31873741-1e3a-46df-bb5d-a94a867cbe24/scratchpad/<your-subfolder>. Run Python with -I. Cite repo-relative file:line evidence for any claim about OpenHeart. Keep text fields tight (about 90 words max each).`

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

const BENCH_SCHEMA = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    findings: { type: 'array', items: FINDING },
    suite: {
      type: 'array',
      description: 'The proposed OpenHeart Bench, one entry per evaluated output',
      items: {
        type: 'object',
        properties: {
          output: { type: 'string', description: 'e.g. call graph, symbol resolution, pattern detection, performance' },
          corpus: { type: 'string', description: 'datasets / repos with pinned versions' },
          oracle: { type: 'string', description: 'ground truth source or differential oracle' },
          metrics: { type: 'string' },
          competitors: { type: 'string', description: 'tools to compare against and any published numbers' },
        },
        required: ['output', 'corpus', 'oracle', 'metrics', 'competitors'],
      },
    },
    harness: { type: 'string', description: 'repo layout, tooling, CI integration, regression thresholds, publishing' },
    sources: { type: 'array', items: { type: 'string' }, description: 'URLs or citations used' },
  },
  required: ['summary', 'findings', 'suite', 'harness', 'sources'],
}

const COMP_SCHEMA = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    matrix: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          dimension: { type: 'string' },
          openheart: { type: 'string', description: 'what OpenHeart actually does today, with file evidence' },
          best_in_class: { type: 'string', description: 'which tools lead and what they do' },
          gap: { type: 'string', enum: ['none', 'small', 'medium', 'large'] },
        },
        required: ['dimension', 'openheart', 'best_in_class', 'gap'],
      },
    },
    differentiators: { type: 'array', items: { type: 'string' } },
    findings: { type: 'array', items: FINDING },
    sources: { type: 'array', items: { type: 'string' } },
  },
  required: ['summary', 'matrix', 'differentiators', 'findings', 'sources'],
}

const AUTO_SCHEMA = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    metrics: {
      type: 'array',
      items: {
        type: 'object',
        properties: { name: { type: 'string' }, value: { type: 'string' }, detail: { type: 'string' } },
        required: ['name', 'value', 'detail'],
      },
    },
    findings: { type: 'array', items: FINDING },
  },
  required: ['summary', 'metrics', 'findings'],
}

const TASKS = [
  {
    code: 'BENCH',
    schema: BENCH_SCHEMA,
    prompt: `TASK (finding id prefix BENCH): Define what benchmark-grade evaluation looks like for OpenHeart and design a concrete, reproducible benchmark suite ("OpenHeart Bench") it should adopt.
First read what evaluation exists today and what is claimed: README.md, docs/overview.md, docs/architecture.md, docs/technical-decisions.md, scripts/*.py, ruthless_verify.py, ruthless_config.json, tests/pipeline_accuracy_tests.rs, tests/research_paper_theorems_tests.rs. Note every claim with no reproducible evidence behind it.
Then, using WebSearch (standard mode) where you are unsure, identify for each OpenHeart output the accepted public benchmarks, ground-truth corpora, oracles and metrics:
- Java call graphs and points-to: JCG / Judge (Reif et al., ISSTA 2019), XCorpus, DaCapo, Qualitas Corpus, call-graph soundness studies (e.g. Sui et al.), SootUp / OPAL / WALA / Doop comparisons.
- Symbol resolution and cross-references: javac as a differential oracle, scip-java / Kythe / stack-graphs evaluations.
- CFG / SSA / dominators: differential testing against bytecode CFGs (ASM, SootUp), Lengauer-Tarjan as an oracle, property-based tests.
- Design-pattern detection: P-MARt, DPB (Arcelli Fontana et al.), Percerons, DPDf; precision / recall / F1 per pattern and per role.
- UML reverse engineering: the Lindholmen dataset (Hebig et al., MSR 2016), ModelSet; layout-quality metrics (edge crossings, area, aspect ratio).
- ROBDD: path-count oracles via explicit path enumeration on small programs; standard BDD sanity benchmarks.
- Performance and scale: repository size tiers (e.g. commons-lang, guava, spring-framework, elasticsearch, intellij-community) at pinned commits; metrics (LOC/s, peak RSS, artifact bytes per LOC, query latency p50/p99); tooling (criterion or divan, iai-callgrind, hyperfine, dhat / heaptrack, bencher.dev or github-action-benchmark).
Return: findings for the gaps between claims and measurement (with file:line evidence), the suite design (one entry per output), the harness design, and sources. Where competitor tools have published numbers, include them with the source so OpenHeart can be compared honestly.`,
  },
  {
    code: 'COMP',
    schema: COMP_SCHEMA,
    prompt: `TASK (finding id prefix COMP): Build a capability matrix comparing OpenHeart as actually implemented (read src/main.rs, src/adapters/server.rs, src/ingestion/adapter/*, src/ingestion/parser/*, src/scpg/query/*, src/scpg/diagram/export/*, README.md, ROADMAP.md) against best-in-class tools in its two categories:
(a) static-analysis / code-property-graph engines: Joern, CodeQL, SootUp, WALA, OPAL, Doop, Semgrep, Infer, PhASAR;
(b) code-to-UML and code-visualization tools: IntelliJ IDEA diagrams, Visual Paradigm and Enterprise Architect reverse engineering, PlantUML generators, Sourcetrail, CodeSee, Mermaid generators.
Dimensions: languages and language versions; frontend (source vs bytecode, build-system / classpath awareness); analyses (CHA, RTA, points-to, IFDS, taint); query language or API; incremental analysis; largest codebases handled; outputs and interop (SARIF, SCIP / LSIF, GraphML, DOT, XMI); IDE / LSP integration; CI integration; extensibility (plugins, language adapters); diagram UX (navigation, filtering, diffing between commits); docs, licensing and community. Use WebSearch (standard mode) to confirm competitor facts you are unsure about.
Return the matrix, the most important missing capabilities as findings (kind enhancement, severity by importance for best-in-class status), the 3 to 5 things OpenHeart could credibly be best at once its flaws are fixed, and sources.`,
  },
  {
    code: 'AUTO',
    schema: AUTO_SCHEMA,
    prompt: `TASK (finding id prefix AUTO): Run every automated check possible on this machine without a Rust toolchain (cargo is NOT installed; Node and Python are). Report quantitative metrics plus findings. Use scratch subfolder audit-auto.
1. Syntax-check every .js file under web/ with Node (they are ES modules: copy to .mjs in scratch or use node --check --input-type=module via stdin).
2. Parse every .py file (scripts/*.py, ruthless_verify.py) with python -I -c "import ast,sys; ast.parse(open(sys.argv[1],encoding='utf-8').read())".
3. JSON validity of every .json file in the repo.
4. web/diagrams/manifest.json vs files on disk: referenced-but-missing, present-but-unreferenced, duplicates or near-duplicates (state.json vs statemachine.json, use_case.json vs usecase.json), and counts vs the README's 19 diagrams.
5. Rust module tree: every mod declaration resolves to a file; every .rs under src/ is reachable from src/lib.rs or src/main.rs (list orphans).
6. Cargo.toml dependencies actually referenced in src/ or tests/ (for example, is tree_sitter_javascript used anywhere?).
7. Relative links in README.md and docs/**/*.md that point to missing files.
8. Code duplication: write a quick token-shingle duplicate detector (for example 40-token windows) and run it over src/scpg/diagram/export/*.rs, over all of src/, and over web/js/*.js; report duplicated-line percentages and the top duplicated regions with file:line.
9. Per-file Rust hotspots: counts of .unwrap(), .expect(, panic!, unreachable!, slice/array indexing, numeric 'as' casts; longest functions (approximate by brace matching); every unsafe block with location.
10. Rustdoc coverage proxy: share of pub fn / struct / enum / trait items preceded by a /// comment, per top-level module.
11. Hardcoded dev-specific paths and URLs in src/, web/js, scripts (target_repos, FractalAndroid, localhost ports, personal GitHub URLs).
12. Secret-looking strings (API keys, tokens): report file:line and type only, never the value.
Return metrics as {name, value, detail} entries and the significant problems as findings.`,
  },
]

phase('Context')
const results = await parallel(TASKS.map(t => () =>
  agent(`${CONTEXT}\n\n${t.prompt}`, { label: `context:${t.code}`, phase: 'Context', schema: t.schema })
    .then(r => (r ? { ...r, code: t.code } : null))
))
const ok = results.filter(Boolean)
const missing = TASKS.filter(t => !ok.find(r => r.code === t.code)).map(t => t.code)
if (missing.length) log(`No result from: ${missing.join(', ')}`)
return ok
