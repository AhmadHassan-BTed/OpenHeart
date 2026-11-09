// Prints one line whenever a fix engineer or a new-module engineer finishes.
const fs = require('fs'), path = require('path');
const [, , wfDir, skip] = process.argv;
const state = {}, done = new Set(String(skip || '').split(',').filter(Boolean));
function find(o, d) {
  d = d || 0;
  if (!o || typeof o !== 'object' || d > 5) return null;
  if (!Array.isArray(o) && Array.isArray(o.handoffs)) return o;
  for (const v of Object.values(o)) { const r = find(v, d + 1); if (r) return r; }
  return null;
}
function tick() {
  let dirs = [];
  try { dirs = fs.readdirSync(wfDir); } catch (e) { return; }
  for (const d of dirs) {
    const j = path.join(wfDir, d, 'journal.jsonl');
    let txt;
    try { txt = fs.readFileSync(j, 'utf8'); } catch (e) { continue; }
    const st = state[j] || (state[j] = { n: 0, labels: {} });
    const lines = txt.split('\n');
    const c = lines.length - 1;
    for (let i = st.n; i < c; i++) {
      let o;
      try { o = JSON.parse(lines[i]); } catch (e) { continue; }
      if (o.type === 'started') { st.labels[o.agentId] = o.label; st.labels[o.key] = o.label; continue; }
      const label = o.label || st.labels[o.agentId] || st.labels[o.key] || '';
      if (!/^(fix|cfam):/.test(label) || done.has(label)) continue;
      const r = find(o);
      if (!r) continue;
      done.add(label);
      const res = r.results || [];
      const n = s => res.filter(x => x.status === s).length;
      const what = r.results ? 'fixed=' + n('fixed') + ' partial=' + n('partial') + ' skipped=' + n('skipped') : 'files=' + (r.files_created || []).length;
      console.log(label + ' done: ' + what + ' handoffs=' + r.handoffs.length);
    }
    st.n = c;
  }
}
setInterval(tick, 3000);
tick();
