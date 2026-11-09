// Assemble the OpenHeart audit report from workflow journals.
//   node build.js digest <workflowsDir>                       -> compact listing for curation
//   node build.js build  <workflowsDir> <curation.json> <template.html> <out.html> [data.json]
const fs = require('fs'), path = require('path');
const [, , mode, wfDir, curPath, tplPath, outPath, dumpPath] = process.argv;
const SEV = ['critical', 'high', 'medium', 'low'];
const sevIdx = s => { const i = SEV.indexOf(s); return i < 0 ? 9 : i; };

function deepFind(o, pred, depth) {
  depth = depth || 0;
  if (!o || typeof o !== 'object' || depth > 5) return null;
  if (!Array.isArray(o) && pred(o)) return o;
  for (const v of Object.values(o)) {
    if (typeof v === 'string' && v.length > 20 && v[0] === '{') {
      try { const r = deepFind(JSON.parse(v), pred, depth + 1); if (r) return r; } catch (e) {}
    }
    const r = deepFind(v, pred, depth + 1);
    if (r) return r;
  }
  return null;
}
const isResult = o => ['findings', 'verdicts', 'suite', 'matrix', 'metrics'].some(k => Array.isArray(o[k]));

// label -> latest result, scanning workflow runs oldest first
function collect(dir) {
  const runs = fs.readdirSync(dir)
    .map(d => path.join(dir, d, 'journal.jsonl'))
    .filter(j => fs.existsSync(j))
    .sort((a, b) => fs.statSync(a).mtimeMs - fs.statSync(b).mtimeMs);
  const out = {};
  for (const j of runs) {
    const labels = {};
    for (const l of fs.readFileSync(j, 'utf8').split('\n')) {
      if (!l.trim()) continue;
      let o; try { o = JSON.parse(l); } catch (e) { continue; }
      if (o.type === 'started') { labels[o.agentId] = o.label; labels[o.key] = o.label; continue; }
      const label = o.label || labels[o.agentId] || labels[o.key];
      if (!label) continue;
      const r = deepFind(o, isResult);
      if (r) out[label] = r;
    }
  }
  return out;
}

// Combine refute-lens and reproduce-lens verdicts for one finding.
function combine(vs) {
  const notes = vs.map(v => (vs.length > 1 ? (v.lens === 'repro' ? 'Reproduce pass: ' : 'Refute pass: ') : '') + v.notes).join(' ');
  const files = Array.from(new Set([].concat(...vs.map(v => v.corrected_files || []))));
  const lowest = vs.map(v => v.corrected_severity).sort((a, b) => sevIdx(b) - sevIdx(a))[0];
  if (vs.length === 1) return { verdict: vs[0].verdict, corrected_severity: vs[0].corrected_severity, notes, corrected_files: files };
  const kept = vs.filter(v => v.verdict !== 'refuted');
  if (!kept.length) return { verdict: 'refuted', corrected_severity: lowest, notes, corrected_files: files };
  if (kept.length < vs.length) return { verdict: 'partially-correct', corrected_severity: lowest, notes, corrected_files: files, split: true };
  const reproOk = vs.find(v => v.lens === 'repro' && (v.verdict === 'confirmed' || v.verdict === 'partially-correct'));
  const verdict = kept.every(v => v.verdict === 'confirmed') ? 'confirmed'
    : kept.some(v => v.verdict === 'partially-correct') ? 'partially-correct'
    : kept.some(v => v.verdict === 'confirmed') ? 'confirmed' : 'unverifiable';
  return { verdict, corrected_severity: reproOk ? reproOk.corrected_severity : lowest, notes, corrected_files: files };
}

function gather(results, cur) {
  const ctxArea = (cur && cur.contextArea) || {};
  const areaRes = {}, findings = [], vlist = {}, missed = [];
  for (const [label, r] of Object.entries(results)) {
    let m;
    if ((m = label.match(/^(find|find2|lens|context):(\w+)$/))) {
      const code = m[2];
      if (m[1] !== 'find2') areaRes[code] = r;
      const area = m[1] === 'find' ? code : (ctxArea[code] || code);
      (r.findings || []).forEach(f => findings.push(Object.assign({}, f, { area, source: m[1] === 'find2' ? code + '2' : code })));
    } else if ((m = label.match(/^(verify2?):(\w+):\d+$/))) {
      const lens = m[1] === 'verify2' ? 'repro' : 'refute';
      const code = m[2];
      (r.verdicts || []).forEach(v => { (vlist[v.id] = vlist[v.id] || []).push(Object.assign({}, v, { lens })); });
      (r.missed || []).forEach(f => missed.push(Object.assign({}, f, { area: ctxArea[code] || code, source: code, verdict: 'added' })));
    }
  }
  const verdicts = {};
  for (const [id, vs] of Object.entries(vlist)) verdicts[id] = combine(vs);
  return { areaRes, findings, verdicts, missed, vlist };
}

if (mode === 'digest') {
  const results = collect(wfDir);
  const { findings, verdicts, missed, vlist } = gather(results, null);
  console.log('labels:', Object.keys(results).sort().join(' '));
  const all = findings.concat(missed);
  const ids = new Set(all.map(f => f.id));
  for (const f of all.sort((a, b) => a.source.localeCompare(b.source) || sevIdx(a.severity) - sevIdx(b.severity))) {
    const v = verdicts[f.id];
    const n = vlist[f.id] ? vlist[f.id].length : 0;
    const ver = f.verdict === 'added' ? 'ADDED' : v ? `${v.verdict}${v.corrected_severity !== f.severity ? '->' + v.corrected_severity : ''}${n > 1 ? ' x' + n : ''}` : '-';
    console.log(`${f.source}\t${f.id}\t${f.severity}\t${f.kind}\t${f.category}\t${ver}\t${f.title}`);
  }
  const orphan = Object.keys(verdicts).filter(id => !ids.has(id));
  if (orphan.length) console.log('\nverdicts with no matching finding:', orphan.join(' '));
  const tally = {}; Object.values(verdicts).forEach(v => { tally[v.verdict] = (tally[v.verdict] || 0) + 1; });
  console.log(`\nfindings=${findings.length} missed=${missed.length} verdicts=${Object.keys(verdicts).length} ${JSON.stringify(tally)}`);
  process.exit(0);
}

// ---- build ----
const cur = JSON.parse(fs.readFileSync(curPath, 'utf8'));
const results = collect(wfDir);
const { areaRes, findings: raw, verdicts, missed } = gather(results, cur);
const warn = m => console.error('WARN', m);

const dropped = [];
let findings = [];
for (const f of raw) {
  const v = verdicts[f.id];
  if (cur.raw) {
    // Raw mode: keep every finding as detected; notes from the partial review are annotations only.
    f.verdict = v && v.verdict === 'refuted' ? 'disputed' : null;
    if (v) f.verifyNote = `Partial review (${v.verdict}${v.corrected_severity && v.corrected_severity !== f.severity ? ', suggested ' + v.corrected_severity : ''}): ${v.notes}`;
    findings.push(f);
    continue;
  }
  if (v && v.verdict === 'refuted') { dropped.push({ id: f.id, title: f.title, reason: v.notes }); continue; }
  if (v) {
    f.verdict = v.verdict;
    if (v.corrected_severity) f.severity = v.corrected_severity;
    f.verifyNote = v.notes;
    if (v.corrected_files && v.corrected_files.length) f.files = v.corrected_files;
  } else {
    f.verdict = null;
  }
  findings.push(f);
}
findings = findings.concat(missed);

// Curation: exclusions, duplicate merges, field overrides
const byId = {};
findings.forEach(f => {
  if (byId[f.id]) { warn('duplicate id ' + f.id); f.id = f.id + '-b'; }
  byId[f.id] = f;
});
(cur.exclude || []).forEach(x => {
  const id = typeof x === 'string' ? x : x.id;
  if (!byId[id]) return warn('exclude: unknown ' + id);
  byId[id]._gone = true;
  if (x.reason) dropped.push({ id, title: byId[id].title, reason: x.reason });
});
Object.entries(cur.merge || {}).forEach(([dup, keep]) => {
  const d = byId[dup], k = byId[keep];
  if (!d || !k) return warn(`merge: unknown ${dup} -> ${keep}`);
  d._gone = true;
  k.files = Array.from(new Set((k.files || []).concat(d.files || [])));
  k.alsoRaisedAs = (k.alsoRaisedAs || []).concat([dup]);
  if (sevIdx(d.severity) < sevIdx(k.severity)) k.severity = d.severity;
});
Object.entries(cur.overrides || {}).forEach(([id, o]) => {
  if (!byId[id]) return warn('override: unknown ' + id);
  Object.assign(byId[id], o);
});
findings = findings.filter(f => !f._gone);
findings.forEach(f => {
  if (f.alsoRaisedAs) f.verifyNote = (f.verifyNote ? f.verifyNote + ' ' : '') + `Also raised independently as ${f.alsoRaisedAs.join(', ')}.`;
  delete f.alsoRaisedAs; delete f._gone; delete f.source;
});

const areaOrder = cur.areas.map(a => a.code);
const ai = c => { const i = areaOrder.indexOf(c); return i < 0 ? 99 : i; };
findings.sort((a, b) => sevIdx(a.severity) - sevIdx(b.severity)
  || (a.kind === b.kind ? 0 : a.kind === 'flaw' ? -1 : 1)
  || ai(a.area) - ai(b.area) || a.id.localeCompare(b.id));
findings.forEach(f => { if (ai(f.area) === 99) warn(`finding ${f.id} has area ${f.area} not in curation.areas`); });

const live = new Set(findings.map(f => f.id));
(cur.top || []).forEach(t => { if (!live.has(t.id)) warn('top: missing ' + t.id); });
(cur.roadmap || []).forEach(s => s.items.forEach(it => { if (!live.has(it.id)) warn(`roadmap ${s.stage}: missing ${it.id}`); }));

const areas = cur.areas.map(a => {
  const r = areaRes[a.code] || {};
  return { code: a.code, name: a.name, artifacts: a.artifacts, summary: a.summary || r.summary || '', strengths: a.strengths || r.strengths || [] };
});

const B = areaRes.BENCH, C = areaRes.COMP, M = areaRes.AUTO;
const paras = s => (Array.isArray(s) ? s : String(s || '').split(/\n\s*\n|\n(?=\d+[.)] )/)).map(x => x.trim()).filter(Boolean);
const data = {
  meta: cur.meta,
  areas,
  findings,
  top: cur.top || [],
  roadmap: cur.roadmap || [],
  dropped: dropped.concat(cur.droppedExtra || []),
  bench: B ? { lede: cur.benchLede || B.summary, suite: B.suite, harness: paras(cur.benchHarness || B.harness), sources: B.sources } : null,
  compare: C ? { lede: cur.compareLede || C.summary, matrix: C.matrix, differentiators: C.differentiators, sources: C.sources } : null,
  measured: M ? (cur.measured || M.metrics) : null,
};

const LS = String.fromCharCode(0x2028), PS = String.fromCharCode(0x2029);
const json = JSON.stringify(data).replace(/</g, '\\u003c').split(LS).join('\\u2028').split(PS).join('\\u2029');
const tpl = fs.readFileSync(tplPath, 'utf8');
if (!tpl.includes('__AUDIT_DATA__')) throw new Error('template has no __AUDIT_DATA__ marker');
fs.writeFileSync(outPath, tpl.split('__AUDIT_DATA__').join(json));
if (dumpPath) fs.writeFileSync(dumpPath, JSON.stringify(data, null, 1));
const sev = {}; findings.forEach(f => { sev[f.severity] = (sev[f.severity] || 0) + 1; });
console.log(`wrote ${outPath}: ${findings.length} findings ${JSON.stringify(sev)}, dropped ${data.dropped.length}, ${(fs.statSync(outPath).size / 1024).toFixed(0)} KB`);
