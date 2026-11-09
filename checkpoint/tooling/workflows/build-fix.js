export const meta = {
  name: 'openheart-build-fix',
  description: 'Fix the current compile errors in OpenHeart in four parallel groups with disjoint files (compile errors only)',
  phases: [{ title: 'Fix', detail: 'minimal compile fixes per file group' }],
}

const LOG = 'C:\\Users\\AHMAD_~1\\AppData\\Local\\Temp\\claude\\D--Repos-OpenHeart\\31873741-1e3a-46df-bb5d-a94a867cbe24\\scratchpad\\build2.log'

const GROUPS = {
  TS: { files: 'src/ast/adapter/java.rs, src/ast/adapter/registry.rs, src/ast/adapter/generic.rs, src/ingestion/adapter/java.rs, src/ingestion/adapter/javascript.rs, src/ingestion/adapter/generic.rs, src/ingestion/parser/tree_sitter.rs, src/cg/serializer.rs', hint: 'tree-sitter is now 0.25 and the grammar crates 0.23: use tree_sitter_java::LANGUAGE.into() / tree_sitter_javascript::LANGUAGE.into() (LanguageFn -> Language); Parser::set_language takes &Language; Language is Clone but not Copy, so return self.ts_lang.clone(). For src/cg/serializer.rs:94 fix the u16/u32 mismatch by widening or converting consistently with the record type it writes.' },
  TRA: { files: 'src/tra/** (mod.rs, entity.rs, builder.rs, forward/cs_span.rs, forward/sym_span.rs, uml_link/mod.rs, uml_link/digest.rs, and a position module if needed)', hint: 'crate::tra::position is imported by four files but not declared: if src/tra/position.rs exists, declare pub mod position; in src/tra/mod.rs; if it does not exist, create it with exactly the items those imports use, inferred from their use sites. Fix builder.rs call/return mismatches against the current function signatures (a function now returns (Vec<BICsEntry>, CallSiteIndexReport) and takes 2 arguments). Fix digest.rs u8/u16 vs u32 mismatches consistently with the record field types.' },
  SCPG: { files: 'src/scpg/diagram/export/json.rs, src/scpg/diagram/export/mermaid.rs, src/scpg/diagram/export/plantuml.rs, src/scpg/query/lang.rs', hint: 'Handle the new enum variants (InteractionEvent::FragmentElse, FragmentKind::Critical) in each match with real rendering (else-branch separator of the enclosing fragment; critical region), json.rs:357 expects u32 but gets Option<u32> (handle the None case explicitly, e.g. emit null or skip, consistent with the surrounding output), and fix the two E0503 borrow errors in query/lang.rs (copy the index before the mutable borrow or restructure the loop).' },
  SRV: { files: 'src/adapters/server.rs, src/adapters/runner.rs', hint: 'server.rs imports ANALYSIS_GRAPH_TYPES, SOURCE_LANGUAGES, UML_DIAGRAM_TYPES from crate::adapters::runner, which do not exist: define them in runner.rs from the data the runner actually supports (or import from where they really live). FetchOptions has private fields: use its constructor/builder. DiscoveryOptions now has a languages field: fill it from the runner/discovery defaults. Handle DiagramFormat::DOT and DiagramFormat::GraphML in the match at server.rs:1537 using the exporters that exist (or return an explicit unsupported-format error response). runner.rs:2257 calls Phase8Stage::run_with, which does not exist: read src/psa/mod.rs and call the existing Phase8Stage API with the same inputs.' },
}

const RULES = `You are fixing compile errors in OpenHeart (Rust, repo D:\\Repos\\OpenHeart, branch main; in Bash: /d/Repos/OpenHeart). The full compiler output is in ${LOG}; read the errors for your files there (they are short-format lines: path:line:col: error...). Fix ONLY compile errors in your files, with minimal, correct changes that keep the intended behavior; read surrounding code and the APIs you call before editing. Do not edit files outside your group. Do not run cargo, tests, linters or any checks (another process builds afterwards and other engineers are editing other files right now). No git commands. Match the surrounding code style. Return the list of errors you fixed and any error you could not fix with the reason.`

const RESULT = {
  type: 'object',
  properties: {
    fixed: { type: 'array', items: { type: 'string' } },
    not_fixed: { type: 'array', items: { type: 'string' } },
    files_changed: { type: 'array', items: { type: 'string' } },
  },
  required: ['fixed', 'not_fixed', 'files_changed'],
}

phase('Fix')
const out = await parallel(Object.entries(GROUPS).map(([code, g]) => () =>
  agent(`${RULES}\n\nYOUR FILES: ${g.files}\nHINTS: ${g.hint}`, { label: `buildfix:${code}`, phase: 'Fix', schema: RESULT })
    .then(r => (r ? { code, ...r } : null))
))
return out.filter(Boolean).map(r => ({ code: r.code, fixed: r.fixed.length, not_fixed: r.not_fixed, files: r.files_changed }))
