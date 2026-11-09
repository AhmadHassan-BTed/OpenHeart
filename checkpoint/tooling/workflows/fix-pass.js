export const meta = {
  name: 'openheart-fix',
  description: 'Pure-coding fix pass on OpenHeart main: one engineer per disjoint path set implements the detected fixes',
  phases: [{ title: 'Fix', detail: 'implement fixes in owned paths, no builds, no tests' }],
}

const FIX = 'C:\\Users\\AHMAD_~1\\AppData\\Local\\Temp\\claude\\D--Repos-OpenHeart\\31873741-1e3a-46df-bb5d-a94a867cbe24\\scratchpad\\fix\\'

const OWNERS = {
  CORE: 'src/core/**, src/ingestion/**, src/ast/**',
  SYM: 'src/symbol/**',
  FLOW: 'src/cfg/**, src/ssa/**, src/cg/**',
  BDD: 'src/psa/**, src/tra/**',
  UML: 'src/uma/**',
  SCPGX: 'src/scpg/diagram/** (all exporters and diagram factory/renderers)',
  SCPG: 'src/scpg/** EXCEPT src/scpg/diagram/** (api, serializer, mmap, incremental, query, mod.rs, types.rs)',
  SRV: 'src/adapters/**, src/main.rs, src/lib.rs, Dockerfile, run.sh, run.bat, scripts/restart_server.sh, .env.example. Also unify the pipeline orchestration duplicated between src/main.rs and src/adapters/server.rs into one shared runner inside your paths.',
  WEB: 'web/** (except web/diagrams/* generated samples unless a finding is about them)',
  ENG: (args && args.pass >= 2) ? 'Cargo.toml (all sections, including aligning [dependencies] versions; other engineers may add dependency lines at the same time, so Read Cargo.toml immediately before each Edit and change only the lines you mean to), Makefile, .gitignore, .editorconfig, .gitattributes' : 'Cargo.toml [package] and [profile.*] sections, Makefile, .gitignore, .editorconfig, .gitattributes',
  DOCS: 'README.md, ROADMAP.md, CHANGELOG.md, SECURITY.md, SUPPORT.md, CODE_OF_CONDUCT.md, docs/**, examples/README.md',
}

const RESULT = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          status: { type: 'string', enum: ['fixed', 'partial', 'skipped'] },
          note: { type: 'string', description: 'one line' },
        },
        required: ['id', 'status', 'note'],
      },
    },
    handoffs: {
      type: 'array',
      items: {
        type: 'object',
        properties: { file: { type: 'string' }, change: { type: 'string' }, reason: { type: 'string' } },
        required: ['file', 'change', 'reason'],
      },
    },
    files_changed: { type: 'array', items: { type: 'string' } },
  },
  required: ['results', 'handoffs', 'files_changed'],
}

const prompt = (owner, pass) => `You are a fix engineer on OpenHeart, a Rust static program-analysis engine (10-phase pipeline from tree-sitter ingestion of Java/Kotlin to UML diagrams) with a hand-rolled HTTP server, a CLI and a vanilla-JS web studio. Repository: D:\\Repos\\OpenHeart (in Bash: /d/Repos/OpenHeart), branch main. Edit the files in place there, directly on main.

YOUR FINDINGS: ${FIX}${owner}-${pass}.json (an audit's detected flaws and enhancements; each has files, evidence, impact, recommendation). Implement them in the code.

YOUR PATHS (you may create and edit files only here): ${OWNERS[owner]}
Other engineers are editing other paths at the same time, and auditors are reading the repo. Never edit outside your paths. If a fix needs a change elsewhere, do the part inside your paths and record the rest in handoffs (file, exact change, reason). Exception: if a fix needs a new crate, you may add or change one dependency line under [dependencies] in Cargo.toml; Read Cargo.toml immediately before that Edit and change nothing else.

PHASE RULES (from the owner, non-negotiable): this is the pure coding phase. Do not compile, build, lint, run or write tests, benchmark, or verify anything; do not write reports or docs; do not ask questions. Do not run any git command that changes state (no commit, branch, stash, checkout, reset). Write real, complete, production-quality code: no TODOs, stubs, placeholders or fabricated output. Where a value cannot be computed from the available data, make the code say so explicitly (an error, an Option, an explicit unknown marker) instead of inventing it.

ORDER: critical first, then high, medium, low; flaws before enhancements. Fix root causes as recommended, adapting where the recommendation does not fit the code. For large enhancements, implement as much as fits cleanly in your paths, foundations first. When you change a signature, update every call site inside your paths and list outside call sites in handoffs. Skip findings that are only about tests or benchmarks. Match the surrounding code's naming, comment density and idioms.

RETURN: per finding id: fixed | partial | skipped with a one-line note; handoffs; the list of files you changed or created.`

const owners = args.owners
const pass = args.pass || 1
phase('Fix')
const out = await parallel(owners.map(o => () =>
  agent(prompt(o, pass), { label: `fix:${o}:${pass}`, phase: 'Fix', schema: RESULT }).then(r => (r ? { owner: o, ...r } : null))
))
const ok = out.filter(Boolean)
const missing = owners.filter(o => !ok.find(r => r.owner === o))
if (missing.length) log(`No result from: ${missing.join(', ')}`)
return ok.map(r => ({
  owner: r.owner,
  fixed: r.results.filter(x => x.status === 'fixed').length,
  partial: r.results.filter(x => x.status === 'partial').length,
  skipped: r.results.filter(x => x.status === 'skipped').length,
  handoffs: r.handoffs,
  files_changed: r.files_changed,
}))
