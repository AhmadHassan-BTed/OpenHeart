// node watch-seen.js <workflowsDir> <outDir> [--once]
// Writes seen-<CODE>.txt (already-reported findings) whenever a detection agent finishes.
const fs = require('fs'), path = require('path');
const [, , wfDir, outDir, flag] = process.argv;
fs.mkdirSync(outDir, { recursive: true });
const done = new Set(fs.readdirSync(outDir).map(f => (f.match(/^seen-(\w+)\.txt$/) || [])[1]).filter(Boolean));
const state = {};
function findResult(o, d) {
  d = d || 0; if (!o || typeof o !== 'object' || d > 4) return null;
  if (!Array.isArray(o) && Array.isArray(o.findings)) return o;
  for (const v of Object.values(o)) {
    if (typeof v === 'string' && v.length > 20 && v[0] === '{') { try { const r = findResult(JSON.parse(v), d + 1); if (r) return r; } catch (e) {} }
    const r = findResult(v, d + 1); if (r) return r;
  }
  return null;
}
function tick() {
  let dirs = []; try { dirs = fs.readdirSync(wfDir); } catch (e) { return; }
  for (const d of dirs) {
    const j = path.join(wfDir, d, 'journal.jsonl');
    let txt; try { txt = fs.readFileSync(j, 'utf8'); } catch (e) { continue; }
    const st = state[j] || (state[j] = { processed: 0, labels: {} });
    const lines = txt.split('\n'); const complete = lines.length - 1;
    for (let i = st.processed; i < complete; i++) {
      const l = lines[i]; if (!l.trim()) continue;
      let o; try { o = JSON.parse(l); } catch (e) { continue; }
      if (o.type === 'started') { st.labels[o.agentId] = o.label; st.labels[o.key] = o.label; continue; }
      const label = o.label || st.labels[o.agentId] || st.labels[o.key] || '';
      const m = label.match(/^(find|find2|lens|context):(\w+)$/);
      if (!m) continue;
      const code = (m[1] === 'find2' ? 'R2_' : '') + m[2];
      if (done.has(code)) continue;
      const res = findResult(o);
      if (!res) continue;
      done.add(code);
      const rows = res.findings.map(f => `${f.id} | ${f.severity} | ${f.kind} | ${f.title} | ${(f.files || []).join(', ')}`);
      fs.writeFileSync(path.join(outDir, `seen-${code}.txt`), `Already reported (${res.findings.length}):\n${rows.join('\n')}\n`);
      const sev = {}; res.findings.forEach(f => { sev[f.severity] = (sev[f.severity] || 0) + 1; });
      console.log(`${label} done: findings=${res.findings.length} ${JSON.stringify(sev)} -> seen-${code}.txt`);
    }
    st.processed = complete;
  }
}
if (flag === '--once') tick(); else { setInterval(tick, 2000); tick(); }
