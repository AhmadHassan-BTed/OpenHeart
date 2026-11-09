export const meta = {
  name: 'openheart-detect-round2',
  description: 'Second detection round for OpenHeart: exhaustive, no-cap sweep per area that skips everything already reported',
  phases: [{ title: 'Detect', detail: 'complete raw list of remaining flaws and enhancements per area' }],
}

const SEEN = 'C:\\Users\\AHMAD_~1\\AppData\\Local\\Temp\\claude\\D--Repos-OpenHeart\\31873741-1e3a-46df-bb5d-a94a867cbe24\\scratchpad\\seen\\'

const PREAMBLE = `CONTEXT: OpenHeart (repo root D:\\Repos\\OpenHeart on Windows; in Bash use /d/Repos/OpenHeart; Read/Grep/Glob tools also work) is a Rust (edition 2021, ~28.7K LOC in src/) static program-analysis engine. A 10-phase pipeline runs over Java/Kotlin sources: (1) tree-sitter lexical ingestion -> token corpus .tca, (2) succinct balanced-parentheses AST .bpa, (3) symbol table / scope graph .sta, (4) CFG in CSR + dominators .cfa, (5) SSA / def-use / CDG / IFDS .ssa, (6) call graph + Andersen points-to .cga, (7) traceability index .tra, (8) ROBDD path summaries .psa, (9) UML semantic metadata + GoF design-pattern detection .uma, (10) unified memory-mapped .scpg + query engine + diagram exporters (JSON / Mermaid / PlantUML / XMI). There is a hand-rolled HTTP server (src/adapters/server.rs), a CLI (src/main.rs), and a vanilla-JS web studio in web/.

GOAL: The owner will fix every problem in OpenHeart, so we need the COMPLETE raw list for your area, not a top-N. A first-round auditor already reported the items in the file named below. Read that file first and do not repeat its items. Find EVERYTHING ELSE in your scope:
- every remaining flaw: bugs, wrong results, edge cases, unwrap/expect or indexing panics on fallible input, integer overflow or truncating casts, security issues, performance problems, unbounded resource use, dead or unreachable code, stubs, misleading names, comments, docs or log messages, missing validation or error handling, inconsistent behavior between similar functions, hardcoded values, nondeterminism;
- every remaining enhancement: missing features, algorithm or data-structure upgrades, API and design improvements, and missing tests (name the function and the test to add).
Include medium and low severity items: nothing is too small if it should be fixed. If you find a materially different aspect of an already-listed item, report it as a new finding and say how it differs.
METHOD: go through every file in your scope from top to bottom and record each issue as you meet it. There is no upper limit on the number of findings; completeness is the goal. Keep text fields short (evidence and recommendation about 60 words max each, impact about 25 words) so the list stays scannable.

RULES:
- Cite repo-relative file:line for every finding and state what the code actually does. Do not present speculation as fact; mark hunches with confidence low.
- No Rust toolchain is installed; reason statically. Node and Python are available for quick experiments in /c/Users/AHMAD_~1/AppData/Local/Temp/claude/D--Repos-OpenHeart/31873741-1e3a-46df-bb5d-a94a867cbe24/scratchpad/round2 (run Python with -I).
- The repository is read-only for you: no edits, creations or deletions inside D:\\Repos\\OpenHeart, no state-changing git commands, no servers, no clones, no fetches.
- Severity: critical = exploitable security hole in the default deployment, data loss, or core output wrong for common input; high = wrong results or crash on realistic input, or a major gap blocking benchmark-grade status; medium = notable defect or gap with a workaround; low = minor.
- Finding ids use your prefix, e.g. PREFIX-01, PREFIX-02.
- In summary, write 2 to 4 sentences on what this pass covered and name any file in scope you could not review fully.`

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

const AREAS = {
  CORE: { seen: 'seen-CORE.txt', scope: 'src/core/** (types/*, io/binary.rs, io/mmap.rs, pipeline.rs, logger.rs, mod.rs), src/ingestion/** (every file, including parser/*, adapter/*, walker.rs, interner.rs, allocator.rs, manifest.rs, builder.rs, serializer.rs), src/ast/** (every file, including bp_encoder.rs, rank_select.rs, rmq.rs, jump_table.rs, preorder.rs, reducer.rs, builder.rs, serializer.rs, adapter/*), tests/ingestion_tests.rs, tests/ast_tests.rs.' },
  SYM: { seen: 'seen-SYM.txt', scope: 'src/symbol/** (every file: passes/pass1_discovery.rs through pass5_hierarchy.rs, scope_graph/*, qual_name_table.rs, std_library/*, uml_meta/*, adapter/*, builder.rs, serializer.rs, mod.rs), src/core/types/symbol.rs, tests/symbol_tests.rs.' },
  FLOW: { seen: 'seen-FLOW.txt', scope: 'src/cfg/** (builder/*, stmts/*, analysis/*, serializer.rs, mod.rs), src/ssa/** (every file), src/cg/** (every file), src/core/types/{cfg,ssa,cg}.rs, tests/cfg_tests.rs, tests/ssa_tests.rs, tests/cg_tests.rs.' },
  BDD: { seen: 'seen-BDD.txt', scope: 'src/psa/** (bdd/*, construction/*, ordering/*, builder.rs, metrics.rs, serializer.rs, types.rs, mod.rs), src/tra/** (every file: forward/*, backward/*, delta/*, uml_link/*, builder.rs, serializer.rs, types.rs, mod.rs), tests/psa_tests.rs, tests/tra_tests.rs, tests/research_paper_theorems_tests.rs.' },
  UML: { seen: 'seen-UML.txt', scope: 'src/uma/** (structural/*, behavioral/*, patterns/*, actor_identification.rs, label_extraction.rs, visitor.rs, builder.rs, serializer.rs, types.rs, mod.rs), tests/uma_tests.rs, tests/design_pattern_architecture_tests.rs, tests/strategy_diagram_tests.rs, tests/adversarial_trick_tests.rs, tests/pipeline_accuracy_tests.rs.' },
  SCPGX: { seen: 'seen-SCPG.txt', scope: 'src/scpg/diagram/** only: mod.rs, common.rs, factory.rs, renderers.rs, export/{mod,json,mermaid,plantuml,plantuml_optimizer,xmi}.rs (about 5.8K lines), plus the generated samples web/diagrams/*.puml, *.mmd and *.json to check output validity. Another auditor covers the rest of src/scpg.' },
  SCPG: { seen: 'seen-SCPG.txt', scope: 'src/scpg/** EXCEPT src/scpg/diagram/: api/*, serializer/{mod,integrity,layout}.rs, mmap/*, incremental/*, query/{mod,cache,cfl,impact,navigation,robdd,slice}.rs, mod.rs, types.rs; tests/scpg_tests.rs, tests/surgical_pipeline_dump_test.rs. Another auditor covers src/scpg/diagram.' },
  WEB: { seen: 'seen-WEB.txt', scope: 'web/index.html, web/app.js, web/js/** (every file, including themes/*), web/style.css, web/themes/*.css, web/diagrams/manifest.json, web/docs/*.html, .github/workflows/deploy_pages.yml. Also read seen-WSEC.txt in the same folder: those web security items are already reported too.' },
  ENG: { seen: 'seen-ENG.txt', scope: 'Cargo.toml, Cargo.lock, Makefile, .github/**, scripts/**, ruthless_verify.py, ruthless_config.json, .gitignore, .editorconfig, .gitattributes, README.md, ROADMAP.md, CHANGELOG.md, docs/**, examples/README.md. Also read seen-BENCH.txt, seen-AUTO.txt and seen-COMP.txt in the same folder: those items are already reported too.' },
  SRV: { seen: 'seen-SRV.txt', scope: 'src/adapters/** (server.rs, web_repo.rs, mod.rs), src/main.rs, Dockerfile, run.sh, run.bat, scripts/restart_server.sh, Makefile, .env.example, SECURITY.md, docs/security-architecture.md.' },
}

const codes = args && args.codes ? args.codes : Object.keys(AREAS)
phase('Detect')
const results = await parallel(codes.map(code => () => {
  const a = AREAS[code]
  return agent(`${PREAMBLE}\n\nYOUR AREA: ${code}. Finding id prefix: ${code}2.\nALREADY REPORTED (read first, do not repeat): ${SEEN}${a.seen}\nSCOPE: ${a.scope}`, {
    label: `find2:${code}`,
    phase: 'Detect',
    schema: SCHEMA,
  }).then(r => (r ? { ...r, code } : null))
}))
const ok = results.filter(Boolean)
const missing = codes.filter(c => !ok.find(r => r.code === c))
if (missing.length) log(`No result from: ${missing.join(', ')}`)
log(`${ok.reduce((n, r) => n + r.findings.length, 0)} new findings across ${ok.length} areas`)
return ok.map(r => ({ code: r.code, count: r.findings.length, summary: r.summary }))
