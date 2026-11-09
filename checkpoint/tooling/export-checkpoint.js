// Exports the full audit-and-rewrite state into <repo>/checkpoint so work can resume on any machine.
//   node export-checkpoint.js <workflowsDir> <fixDir> <repoDir> <toolingSrcDir...>
const fs = require('fs'), path = require('path');
const [, , wfDir, fixDir, repoDir, ...toolSrcs] = process.argv;
const OUT = path.join(repoDir, 'checkpoint');
const SEV = ['critical', 'high', 'medium', 'low'];
const DEFER = new Set(['testing', 'benchmarking']);
fs.mkdirSync(path.join(OUT, 'queues'), { recursive: true });
fs.mkdirSync(path.join(OUT, 'tooling'), { recursive: true });

function deepFind(o, pred, d) {
  d = d || 0;
  if (!o || typeof o !== 'object' || d > 5) return null;
  if (!Array.isArray(o) && pred(o)) return o;
  for (const v of Object.values(o)) {
    if (typeof v === 'string' && v.length > 20 && v[0] === '{') { try { const r = deepFind(JSON.parse(v), pred, d + 1); if (r) return r; } catch (e) {} }
    const r = deepFind(v, pred, d + 1);
    if (r) return r;
  }
  return null;
}

// Collect every agent result by label (latest run wins).
const runs = fs.readdirSync(wfDir).map(d => path.join(wfDir, d, 'journal.jsonl')).filter(j => fs.existsSync(j))
  .sort((a, b) => fs.statSync(a).mtimeMs - fs.statSync(b).mtimeMs);
const byLabel = {};
for (const j of runs) {
  const labels = {};
  for (const l of fs.readFileSync(j, 'utf8').split('\n')) {
    if (!l.trim()) continue;
    let o; try { o = JSON.parse(l); } catch (e) { continue; }
    if (o.type === 'started') { labels[o.agentId] = o.label; labels[o.key] = o.label; continue; }
    const label = o.label || labels[o.agentId] || labels[o.key];
    if (!label) continue;
    const r = deepFind(o, x => ['findings', 'verdicts', 'handoffs', 'suite', 'matrix', 'metrics'].some(k => Array.isArray(x[k])));
    if (r) byLabel[label] = r;
  }
}

// Findings
const findings = {}; const areaSummaries = {};
for (const [label, r] of Object.entries(byLabel)) {
  const m = label.match(/^(find|find2|lens|context):(\w+)$/);
  if (m) {
    areaSummaries[label] = { summary: r.summary || '', strengths: r.strengths || [] };
    (r.findings || []).forEach(f => { if (!findings[f.id]) findings[f.id] = Object.assign({ source: label }, f); });
  }
  const v = label.match(/^verify2?:(\w+):\d+$/);
  if (v) (r.missed || []).forEach(f => { if (!findings[f.id]) findings[f.id] = Object.assign({ source: label }, f); });
}

// Fix results and handoffs
const fixResults = {}; const handoffs = []; const newModules = {};
for (const [label, r] of Object.entries(byLabel)) {
  const m = label.match(/^fix:(\w+):(\d+)$/);
  if (m) {
    fixResults[label] = { owner: m[1], pass: Number(m[2]), results: r.results || [], files_changed: r.files_changed || [] };
    (r.handoffs || []).forEach((h, i) => handoffs.push(Object.assign({ id: `HO-${label.replace(/[^A-Za-z0-9]+/g, '_')}-${i + 1}`, from: label }, h)));
  }
  const c = label.match(/^cfam:(\w+)$/);
  if (c) {
    newModules[c[1]] = { files_created: r.files_created || [], note: r.note || '' };
    (r.handoffs || []).forEach((h, i) => handoffs.push(Object.assign({ id: `HO-${label.replace(/[^A-Za-z0-9]+/g, '_')}-${i + 1}`, from: label }, h)));
  }
}

// Dispatched batches: <OWNER>-<pass>.json
const dispatched = {}; // id -> [{owner, pass}]
for (const f of fs.readdirSync(fixDir)) {
  const m = f.match(/^(\w+)-(\d+)\.json$/);
  if (!m) continue;
  const batch = JSON.parse(fs.readFileSync(path.join(fixDir, f), 'utf8'));
  fs.copyFileSync(path.join(fixDir, f), path.join(OUT, 'queues', f));
  batch.forEach(x => { (dispatched[x.id] = dispatched[x.id] || []).push({ owner: m[1], pass: Number(m[2]) }); });
}
const statusOf = {}; // id -> owner -> status
for (const fr of Object.values(fixResults)) fr.results.forEach(x => { (statusOf[x.id] = statusOf[x.id] || {})[fr.owner] = { status: x.status, note: x.note, pass: fr.pass }; });
const reported = new Set(Object.keys(fixResults));

// Per-finding status
const rows = Object.values(findings).map(f => {
  const d = dispatched[f.id] || [];
  const st = statusOf[f.id] || {};
  let status;
  if (DEFER.has(f.category)) status = 'deferred-testing-phase';
  else if (!d.length) status = 'not-yet-dispatched';
  else {
    const per = d.map(x => (st[x.owner] ? st[x.owner].status : (reported.has(`fix:${x.owner}:${x.pass}`) ? 'not-addressed' : 'in-queue')));
    if (per.every(s => s === 'fixed')) status = 'fixed';
    else if (per.some(s => s === 'fixed' || s === 'partial')) status = 'partial';
    else if (per.every(s => s === 'skipped' || s === 'not-addressed')) status = 'skipped';
    else status = 'in-queue';
  }
  return Object.assign({}, f, { status, owners: d, fix: st });
}).sort((a, b) => a.source.localeCompare(b.source) || SEV.indexOf(a.severity) - SEV.indexOf(b.severity) || a.id.localeCompare(b.id));

fs.writeFileSync(path.join(OUT, 'findings.json'), JSON.stringify(rows, null, 1));
fs.writeFileSync(path.join(OUT, 'fix-results.json'), JSON.stringify(fixResults, null, 1));
fs.writeFileSync(path.join(OUT, 'handoffs.json'), JSON.stringify(handoffs, null, 1));
fs.writeFileSync(path.join(OUT, 'new-modules.json'), JSON.stringify(newModules, null, 1));
fs.writeFileSync(path.join(OUT, 'area-summaries.json'), JSON.stringify(areaSummaries, null, 1));
const research = {};
['context:BENCH', 'context:COMP', 'context:AUTO'].forEach(k => { if (byLabel[k]) research[k] = byLabel[k]; });
fs.writeFileSync(path.join(OUT, 'research.json'), JSON.stringify(research, null, 1));

// Human-readable index
const esc = s => String(s || '').replace(/\|/g, '\\|').replace(/\r?\n/g, ' ');
const tally = {}; rows.forEach(r => { tally[r.status] = (tally[r.status] || 0) + 1; });
let md = '# Findings index\n\nGenerated from `findings.json`. Status is per finding across the owners it was routed to.\n\n';
md += '| Status | Count |\n|---|---|\n' + Object.entries(tally).sort().map(([k, v]) => `| ${k} | ${v} |`).join('\n') + '\n\n';
const groups = {}; rows.forEach(r => { (groups[r.source] = groups[r.source] || []).push(r); });
for (const [src, list] of Object.entries(groups)) {
  md += `## ${src} (${list.length})\n\n| ID | Severity | Kind | Status | Title | Files |\n|---|---|---|---|---|---|\n`;
  md += list.map(r => `| ${r.id} | ${r.severity} | ${r.kind} | ${r.status} | ${esc(r.title)} | ${esc((r.files || []).slice(0, 3).join(', '))} |`).join('\n') + '\n\n';
}
fs.writeFileSync(path.join(OUT, 'FINDINGS.md'), md);

// Tooling copies
for (const src of toolSrcs) {
  if (!fs.existsSync(src)) continue;
  if (fs.statSync(src).isDirectory()) {
    for (const f of fs.readdirSync(src)) if (/\.(js|json|html)$/.test(f) && !/^preview|_inline|curation-preview/.test(f)) fs.copyFileSync(path.join(src, f), path.join(OUT, 'tooling', f));
  } else fs.copyFileSync(src, path.join(OUT, 'tooling', path.basename(src)));
}

const owners = {};
Object.values(fixResults).forEach(fr => {
  const o = owners[fr.owner] = owners[fr.owner] || { passes: [] };
  const n = s => fr.results.filter(x => x.status === s).length;
  o.passes.push({ pass: fr.pass, fixed: n('fixed'), partial: n('partial'), skipped: n('skipped'), files: fr.files_changed.length });
});
console.log(JSON.stringify({ findings: rows.length, status: tally, owners, newModules: Object.fromEntries(Object.entries(newModules).map(([k, v]) => [k, v.files_created.length])), handoffs: handoffs.length }, null, 1));
