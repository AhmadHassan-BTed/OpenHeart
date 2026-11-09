// node dispatch.js <workflowsDir> <outDir> <pass> <OWNER> [<OWNER> ...]
// Routes every detected finding to the fix owners of the files it cites; never sends a finding to the same owner twice.
const fs = require('fs'), path = require('path');
const [, , wfDir, outDir, pass, ...owners] = process.argv;
fs.mkdirSync(outDir, { recursive: true });
const RULES = [
  ['src/scpg/diagram/', 'SCPGX'], ['src/scpg/', 'SCPG'],
  ['src/core/', 'CORE'], ['src/ingestion/', 'CORE'], ['src/ast/', 'CORE'],
  ['src/symbol/', 'SYM'], ['src/cfg/', 'FLOW'], ['src/ssa/', 'FLOW'], ['src/cg/', 'FLOW'],
  ['src/psa/', 'BDD'], ['src/tra/', 'BDD'], ['src/uma/', 'UML'],
  ['src/adapters/', 'SRV'], ['src/main.rs', 'SRV'], ['src/lib.rs', 'SRV'], ['Dockerfile', 'SRV'], ['run.sh', 'SRV'], ['run.bat', 'SRV'], ['scripts/restart_server.sh', 'SRV'], ['.env.example', 'SRV'],
  ['web/', 'WEB'],
  ['README.md', 'DOCS'], ['ROADMAP.md', 'DOCS'], ['CHANGELOG.md', 'DOCS'], ['SECURITY.md', 'DOCS'], ['SUPPORT.md', 'DOCS'], ['CODE_OF_CONDUCT.md', 'DOCS'], ['docs/', 'DOCS'], ['examples/README.md', 'DOCS'],
  ['Cargo.toml', 'ENG'], ['Cargo.lock', 'ENG'], ['.github/', 'ENG'], ['Makefile', 'ENG'], ['scripts/', 'ENG'], ['ruthless_', 'ENG'], ['.gitignore', 'ENG'], ['.editorconfig', 'ENG'], ['.gitattributes', 'ENG'],
  ['tests/', 'TESTS'],
];
const DEFER = new Set(['testing', 'benchmarking']);
const SEV = ['critical', 'high', 'medium', 'low'];
function ownerOf(p) {
  let s = String(p).trim().split(String.fromCharCode(92)).join('/').replace(/^[.][/]/, '').replace(/^[/]?d:?[/]Repos[/]OpenHeart[/]/i, '');
  s = s.replace(/[:#(].*$/, '').replace(/\s.*$/, '');
  for (const [pre, o] of RULES) if (s === pre || s.startsWith(pre)) return o;
  return null;
}
function deepFind(o, pred, d) {
  d = d || 0; if (!o || typeof o !== 'object' || d > 5) return null;
  if (!Array.isArray(o) && pred(o)) return o;
  for (const v of Object.values(o)) {
    if (typeof v === 'string' && v.length > 20 && v[0] === '{') { try { const r = deepFind(JSON.parse(v), pred, d + 1); if (r) return r; } catch (e) {} }
    const r = deepFind(v, pred, d + 1); if (r) return r;
  }
  return null;
}
const all = [];
for (const d of fs.readdirSync(wfDir)) {
  const j = path.join(wfDir, d, 'journal.jsonl'); if (!fs.existsSync(j)) continue;
  const labels = {};
  for (const l of fs.readFileSync(j, 'utf8').split('\n')) {
    if (!l.trim()) continue; let o; try { o = JSON.parse(l); } catch (e) { continue; }
    if (o.type === 'started') { labels[o.agentId] = o.label; labels[o.key] = o.label; continue; }
    const label = o.label || labels[o.agentId] || labels[o.key] || '';
    if (/^(find|find2|lens|context):/.test(label)) {
      const r = deepFind(o, x => Array.isArray(x.findings)); if (r) r.findings.forEach(f => all.push(f));
    } else if (/^(fix|cfam):/.test(label)) {
      const r = deepFind(o, x => Array.isArray(x.handoffs));
      if (r) r.handoffs.forEach((h, i) => all.push({
        id: 'HO-' + label.replace(/[^A-Za-z0-9]+/g, '_') + '-' + (i + 1), kind: 'flaw', category: 'architecture', severity: 'high',
        title: 'Handoff from ' + label + ': ' + String(h.change).slice(0, 100),
        files: String(h.file).split(/[,;]/).map(x => x.trim()).filter(Boolean),
        evidence: h.change, impact: h.reason, recommendation: h.change, effort: 'M', confidence: 'high',
      }));
    } else if (/^verify2?:/.test(label)) {
      const r = deepFind(o, x => Array.isArray(x.verdicts)); if (r && Array.isArray(r.missed)) r.missed.forEach(f => all.push(f));
    }
  }
}
if (Number(pass) >= 2) {
  const BASE = [
    'OpenHeart must support every language with language-agnostic algorithms, and must never hard-code, template or guess: it works the way compilers do, statically.',
    'New code is being written now (read what exists): src/lang (neutral.rs universal vocabulary, frontend.rs LanguageFrontend trait, registry.rs single list of languages, profile.rs, build/ for compile_commands, CMake and MSBuild, preprocess.rs, project_model.rs, uml_semantics.rs, idioms.rs, frontends java, kotlin, c, cpp, csharp); src/ir (typed three-address SSA IR with explicit memory operations, lowering from the AST, dominators); src/memory (abstract heap, field-sensitive points-to with on-the-fly call graph, escape, ownership, mod/ref); src/analysis (lattices, worklist solvers, SCCP, intervals, nullness, typestate, control dependence, IFDS/IDE, contexts, canonical branch conditions); src/derive (every diagram derived from those facts with provenance and a Derivation tag, NotDerivable instead of invention); src/symex (bounded static symbolic execution with a stack, heap and path-condition model producing execution traces); src/abi (C/C++ Itanium and MSVC, JVM and CLR memory layouts, lifetimes, calling conventions); src/arch (architecture recovery: dependency graph, DSM, layers, clustering, metrics, boundaries); src/memview (lifetime, ownership, heap-shape and stack-frame views plus memory-safety and data-race facts). Everything from the architectural flow down to memory-level execution must come from these facts, for every language, with semantics exactly per each language specification.',
    'Contract names: LangId::C, LangId::Cpp, LangId::CSharp; ASTNodeType additions NN_NAMESPACE_DECL, NN_STRUCT_DECL, NN_UNION_DECL, NN_TYPEDEF_DECL, NN_FUNCTION_DECL, NN_DESTRUCTOR_DECL, NN_OPERATOR_DECL, NN_TEMPLATE_DECL, NN_PROPERTY_DECL, NN_INDEXER_DECL, NN_EVENT_DECL, NN_DELEGATE_DECL, NN_USING_DIRECTIVE, NN_INCLUDE_DIRECTIVE, NN_GOTO_STMT, NN_LABELED_STMT, NN_FOREACH_STMT, NN_USING_STMT, NN_LOCK_STMT, NN_YIELD_STMT.',
  ].join(' ');
  const SPEC = {
    CORE: 'Add the LangId and ASTNodeType contract variants; resolve Phase 1 and Phase 2 adapters through src/lang/registry.rs (java, kotlin, c, cpp, csharp) and delete the JavaScript-grammar fallback; upgrade tree-sitter so every grammar crate uses one compatible core version; remove Java-only assumptions from tokenization, reduction and the pipeline; make the token corpus lossless (comments, whitespace and preprocessor trivia kept with exact spans) and generate the per-grammar node-kind classification tables from the node-types.json of each grammar so no construct is ever dropped silently; make the pipeline run lowering (src/ir), memory, abi, analysis, symex, arch, memview and derive stages.',
    SYM: 'Select semantic adapters and standard-library models through the frontend registry; use the neutral visibility and modifier supersets; make scope, import, inheritance and overload resolution generic over frontends (namespaces, using directives, includes, multiple inheritance, partial types, extension functions); no Java-specific names in the passes; replace token-window heuristics with resolution from the syntax tree, exactly per each language specification (JLS, ISO C and C++, ECMA-334 C#, the Kotlin specification).',
    FLOW: 'Rebuild CFG, SSA, CDG and call graph on the new core: CFG and SSA from src/ir (dominators from src/ir/dom.rs), def-use and control dependence from src/analysis, call graph and points-to from src/memory, IFDS from src/analysis/ifds.rs; keep the existing artifact formats as serializations of those results; no Java-specific node names in algorithms.',
    BDD: 'Build path conditions from src/analysis/conditions.rs canonical atoms over the IR, count paths on the IR CFG, and make traceability consume neutral spans from IR provenance, so every frontend is handled the same way.',
    UML: 'Produce UML records from src/derive (class, object, package, component, deployment, sequence, communication, activity, state, use case, interaction, timing) instead of heuristics and templates; use frontend ownership and multiplicity hooks and idioms; make pattern detection structural over symbol, IR and memory facts, language-neutral.',
    SCPGX: 'Make every exporter render src/derive/model.rs DiagramModel only: delete all hard-coded templates and sample content; render NotDerivable and Partial explicitly; language-neutral naming (., ::), templates and generics, properties, events, operators, destructors, structs, unions and namespaces.',
    SCPG: 'Make the unified graph, queries, slicing (on IR def-use and control dependence) and impact analysis neutral over all frontends, keyed by stable language-independent ids, and store the real facts, not counts.',
    SRV: 'Add pub mod lang; pub mod ir; pub mod memory; pub mod analysis; pub mod derive; pub mod symex; pub mod abi; pub mod arch; pub mod memview; to src/lib.rs plus every mod declaration listed in handoffs; make CLI and server discovery, supported-language lists and API responses come from src/lang/registry.rs and profile.rs; accept C, C++, C#, Java and Kotlin projects including compile_commands.json, CMake and .sln/.csproj inputs; run the new pipeline stages.',
    WEB: 'Wire web/js/lang/c-family.js into the in-browser engine for C, C++ and C#, keep Java and Kotlin, take the extension list from one place, make rendering language-neutral, and never show hard-coded sample diagrams as analysis results.',
    ENG: 'Keep Cargo.toml dependencies consistent: one tree-sitter core version that all grammar crates (java, kotlin, c, cpp, c-sharp) support, rust-version raised if a dependency requires it, and every crate the new modules added kept.',
    DOCS: 'Document the language-neutral, compiler-grade architecture truthfully: which languages and phases each frontend supports, and that diagrams are derived from facts with explicit NotDerivable markers.',
  };
  const PATHS = { CORE: 'src/core/', SYM: 'src/symbol/', FLOW: 'src/cfg/', BDD: 'src/psa/', UML: 'src/uma/', SCPGX: 'src/scpg/diagram/', SCPG: 'src/scpg/', SRV: 'src/lib.rs', WEB: 'web/', ENG: 'Cargo.toml', DOCS: 'README.md' };
  for (const o of owners) if (SPEC[o]) all.unshift({ id: 'MANDATE-CORE-ARCH-' + o, kind: 'enhancement', category: 'architecture', severity: 'critical', title: 'Language-neutral, compiler-grade rewiring of your paths (no hard-coding, no templates)', files: [PATHS[o]], evidence: BASE, impact: 'Without this, OpenHeart stays Java-specific and heuristic instead of a universal, compiler-grade engine.', recommendation: SPEC[o], effort: 'XL', confidence: 'high' });
}
const logPath = path.join(outDir, 'dispatch-log.json');
const log = fs.existsSync(logPath) ? JSON.parse(fs.readFileSync(logPath, 'utf8')) : {};
const seenIds = new Set(); const uniq = all.filter(f => !seenIds.has(f.id) && seenIds.add(f.id));
const batch = {}; owners.forEach(o => { batch[o] = []; });
let deferred = 0; const unowned = [];
for (const f of uniq) {
  const os = new Set((f.files || []).map(ownerOf).filter(Boolean));
  if (!os.size) { unowned.push(f.id); continue; }
  if (DEFER.has(f.category)) { deferred++; continue; }
  for (const o of owners) {
    if (!os.has(o) || (log[f.id] || []).includes(o)) continue;
    batch[o].push(f);
    (log[f.id] = log[f.id] || []).push(o);
  }
}
for (const o of owners) {
  batch[o].sort((a, b) => SEV.indexOf(a.severity) - SEV.indexOf(b.severity) || (a.kind === b.kind ? 0 : a.kind === 'flaw' ? -1 : 1));
  fs.writeFileSync(path.join(outDir, `${o}-${pass}.json`), JSON.stringify(batch[o], null, 1));
  const sev = {}; batch[o].forEach(f => { sev[f.severity] = (sev[f.severity] || 0) + 1; });
  console.log(`${o}-${pass}.json: ${batch[o].length} findings ${JSON.stringify(sev)}`);
}
fs.writeFileSync(logPath, JSON.stringify(log, null, 1));
console.log(`total unique findings=${uniq.length} deferred(testing/benchmarking)=${deferred} unowned=${unowned.length}${unowned.length ? ' [' + unowned.join(' ') + ']' : ''}`);
