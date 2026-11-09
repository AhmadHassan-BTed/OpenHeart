export const meta = {
  name: 'openheart-audit-lenses-b',
  description: 'Cross-cutting OpenHeart auditors: web security second pass, deep test-suite review, language support vs claims',
  phases: [{ title: 'Lenses', detail: '3 cross-cutting auditors over the whole repo' }],
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
    code: 'WSEC',
    prompt: `LENS: Web studio security, an independent second pass. Finding id prefix WSEC.
Scope: web/index.html, web/app.js, web/js/**, web/docs/*.html, web/diagrams/manifest.json, .github/workflows/deploy_pages.yml. Trace every DOM sink (innerHTML, outerHTML, insertAdjacentHTML, document.write, srcdoc, setAttribute on href/src/on* attributes, new Function / eval, template-literal HTML) back to its sources: GitHub API responses (file paths, file contents, repository and branch names), parsed PlantUML / Mermaid text, analysis JSON, .openheart.json state files the user loads, URL hash and query params, localStorage. For each reachable sink give the exact payload that would execute (for example a Java identifier or comment carrying markup, a file path containing quotes) and the file:line chain from source to sink.
Also check: GitHub token handling (where it is entered, stored and sent; leakage via URLs, logs or error messages), missing CSP, third-party scripts without SRI, postMessage handlers without origin checks, open redirects, clickjacking, prototype pollution when merging JSON state, ReDoS in regexes (try suspicious ones in Node in the scratch directory).
COMPARE AGAINST: the OWASP DOM-based XSS prevention cheat sheet, Trusted Types, Mermaid securityLevel strict mode, DOMPurify.`,
  },
  {
    code: 'TST',
    prompt: `LENS: Deep test-suite review. Finding id prefix TST.
Read every file in tests/ and every #[cfg(test)] module in src/ (grep -rn 'cfg(test)' src). For each test: what behavior it pins down; whether its assertions would catch a realistic bug (mutation-testing mindset: name two or three plausible bugs in the code under test that the test would NOT catch); and whether it depends on external state (target_repos/, network, current directory, timing).
Produce a coverage map: which modules and functions have meaningful tests and which have none, especially rank/select, RMQ, dominators, post-dominators, SSA renaming, IFDS, points-to, SCC, BDD apply / sat_count, sifting, CRC / integrity, every exporter, server routes, and the web JS.
Then specify, concretely for this codebase, the test architecture a benchmark-grade project needs: property tests (proptest) with named invariants per data structure and algorithm; differential tests (dominators vs a naive oracle, BDDs vs truth-table enumeration, rank/select vs naive counting); golden / snapshot tests (insta) for exporters; fuzz targets (cargo-fuzz) for parsers and artifact loaders; end-to-end tests on pinned real repositories; and mutation testing (cargo-mutants) as a quality gate.`,
  },
  {
    code: 'LANG',
    prompt: `LENS: Language-agnosticism and extensibility. Finding id prefix LANG.
The README claims support for Java, Kotlin, Python, Rust, TypeScript and C++, and the server's /api/supported-languages lists python, typescript, javascript, cpp, rust and go. Determine what is supported end to end: Cargo.toml has only the tree-sitter-java and tree-sitter-javascript grammars. Inspect src/ingestion/adapter/*, src/ast/adapter/*, src/symbol/adapter/* and every later phase for Java-specific assumptions (node-kind strings such as class_declaration or method_invocation, keywords, java.lang stubs, file extensions, package/import semantics) hardcoded outside the adapter boundaries. Evaluate the adapter / registry architecture: what adding Python or TypeScript would require (list the files that must change), whether a language-neutral IR exists between the CST and later phases, and how this compares with Joern frontends, CodeQL extractors, Semgrep's generic AST, and tree-sitter-graph / stack-graphs. Report every claim of language support the code does not back up, and design the target architecture (a language-neutral IR plus per-language lowering) with a migration path.`,
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
