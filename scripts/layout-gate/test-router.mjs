import path from 'node:path';
// Router tests: correctness checks on every route plus timings.
import { pathToFileURL, fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

const REPO = process.env.OH_REPO || path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const { routeOrthogonal } = await import(pathToFileURL(`${REPO}/web/js/layout/router.js`).href);
const G = await import(pathToFileURL(`${REPO}/web/js/layout/geometry.js`).href);

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function field(seed, cols, rows, cellW = 260, cellH = 220) {
  const r = rng(seed);
  const obs = [];
  for (let j = 0; j < rows; j++) for (let i = 0; i < cols; i++) {
    const w = 100 + Math.floor(r() * 100);
    const h = 60 + Math.floor(r() * 100);
    const fx = (cellW - w - 40) * r();
    const fy = (cellH - h - 40) * r();
    obs.push({ id: `n${j}_${i}`, cx: i * cellW + 20 + fx + w / 2, cy: j * cellH + 20 + fy + h / 2, w, h, ancestors: [], gi: i, gj: j });
  }
  return obs;
}

function randomEdges(seed, obs, count, cols, rows, longShare = 0.15, reach = 3) {
  const r = rng(seed);
  const byCell = new Map(obs.map(o => [`${o.gi},${o.gj}`, o]));
  const edges = [];
  for (let k = 0; k < count; k++) {
    const a = obs[Math.floor(r() * obs.length)];
    let b;
    if (r() < longShare) b = obs[Math.floor(r() * obs.length)];
    else {
      const di = Math.floor(r() * (2 * reach + 1)) - reach;
      const dj = Math.floor(r() * (2 * reach + 1)) - reach;
      b = byCell.get(`${Math.min(cols - 1, Math.max(0, a.gi + di))},${Math.min(rows - 1, Math.max(0, a.gj + dj))}`);
    }
    if (!b || a === b) continue;
    edges.push({ id: `e${k}`, source: a.id, target: b.id });
  }
  return edges;
}

function check(name, input, routes, opts = {}) {
  const byId = new Map(input.obstacles.map(o => [o.id, o]));
  const viol = { offBorder: 0, diagonal: 0, throughObstacle: 0, notSimplified: 0 };
  const examples = [];
  const segs = [];
  for (const e of input.edges) {
    const r = routes.get(e.id);
    if (!r) continue;
    const p = r.points;
    const S = byId.get(e.source);
    const T = byId.get(e.target);
    const onBorder = (pt, n) => G.distanceToRectBorder(pt, n) <= 0.5 &&
      !(pt.x > n.cx - n.w / 2 + 0.5 && pt.x < n.cx + n.w / 2 - 0.5 && pt.y > n.cy - n.h / 2 + 0.5 && pt.y < n.cy + n.h / 2 - 0.5);
    if (!onBorder(p[0], S) || !onBorder(p[p.length - 1], T)) { viol.offBorder++; if (examples.length < 5) examples.push(`${e.id} off border`); }
    const containers = new Set([...(S.ancestors || []), ...(T.ancestors || [])]);
    for (let i = 1; i < p.length; i++) {
      const a = p[i - 1];
      const b = p[i];
      if (Math.abs(a.x - b.x) > 1e-6 && Math.abs(a.y - b.y) > 1e-6) { viol.diagonal++; if (examples.length < 5) examples.push(`${e.id} diagonal`); }
      if (Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.y - b.y) < 1e-9) viol.notSimplified++;
      for (const o of input.obstacles) {
        if (containers.has(o.id)) continue;
        if (G.segmentIntersectsRectInterior(a, b, o, 1)) { viol.throughObstacle++; if (examples.length < 5) examples.push(`${e.id} through ${o.id}`); }
      }
      segs.push({ e: e.id, a, b });
      if (i >= 2) {
        const c = p[i - 2];
        const cross = (a.x - c.x) * (b.y - a.y) - (a.y - c.y) * (b.x - a.x);
        if (Math.abs(cross) < 1e-9) viol.notSimplified++;
      }
    }
  }
  // Shared collinear overlaps left after nudging (different edges, same line, overlapping > 1px).
  let shared = 0;
  const groups = new Map();
  for (const s of segs) {
    const h = Math.abs(s.a.y - s.b.y) < 1e-6;
    const key = h ? `h${s.a.y.toFixed(2)}` : `v${s.a.x.toFixed(2)}`;
    const lo = h ? Math.min(s.a.x, s.b.x) : Math.min(s.a.y, s.b.y);
    const hi = h ? Math.max(s.a.x, s.b.x) : Math.max(s.a.y, s.b.y);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push({ e: s.e, lo, hi });
  }
  groups.forEach(list => {
    list.sort((x, y) => x.lo - y.lo);
    for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length && list[j].lo < list[i].hi - 1; j++) {
      if (list[i].e !== list[j].e && Math.min(list[i].hi, list[j].hi) - list[j].lo > 1) shared++;
    }
  });
  const routed = input.edges.filter(e => routes.has(e.id)).length;
  const bends = [...routes.values()].reduce((a, r) => a + Math.max(0, r.points.length - 2), 0);
  const total = Object.values(viol).reduce((a, b) => a + b, 0);
  console.log(`${total ? 'FAIL' : 'PASS'} ${name.padEnd(34)} routed ${routed}/${input.edges.length}  bends ${bends}  sharedOverlaps ${shared}  ${opts.ms !== undefined ? `${opts.ms.toFixed(0)}ms` : ''}  ${total ? JSON.stringify(viol) : ''}`);
  if (examples.length) console.log('     ', examples.join('; '));
  return { routed, total, shared };
}

function run(name, input, options = {}) {
  const t0 = performance.now();
  const routes = routeOrthogonal(input, options);
  const ms = performance.now() - t0;
  return { routes, res: check(name, input, routes, { ms }) };
}

// 1. Random field, 50 obstacles, 300 edges.
{
  const obs = field(1, 10, 5);
  const edges = randomEdges(2, obs, 300, 10, 5, 0.3, 3);
  run('field-50 / 300 edges', { obstacles: obs, edges });
}

// 2. Nested packages: A and B with C in between.
{
  const pkg = (id, l, t, r, b) => ({ id, cx: (l + r) / 2, cy: (t + b) / 2, w: r - l, h: b - t, ancestors: [] });
  const card = (id, cx, cy, parent) => ({ id, cx, cy, w: 120, h: 80, ancestors: parent ? [parent] : [] });
  const obstacles = [
    pkg('A', 0, 0, 400, 300), card('a1', 120, 120, 'A'), card('a2', 280, 200, 'A'),
    pkg('C', 500, -100, 900, 400), card('c1', 700, 150, 'C'),
    pkg('B', 1000, 0, 1400, 300), card('b1', 1120, 120, 'B'), card('b2', 1280, 200, 'B'),
    card('free', 700, 600, null)
  ];
  const edges = [
    { id: 'a1-b1', source: 'a1', target: 'b1' }, { id: 'a2-b2', source: 'a2', target: 'b2' },
    { id: 'a1-c1', source: 'a1', target: 'c1' }, { id: 'A-B', source: 'A', target: 'B' },
    { id: 'a1-free', source: 'a1', target: 'free' }, { id: 'nested', source: 'a1', target: 'A' },
    { id: 'self', source: 'b1', target: 'b1' }, { id: 'missing', source: 'b1', target: 'nope' }
  ];
  const { routes } = run('nested packages', { obstacles, edges });
  const passesC = (id) => {
    const r = routes.get(id);
    if (!r) return 'unrouted';
    for (let i = 1; i < r.points.length; i++) if (G.segmentIntersectsRectInterior(r.points[i - 1], r.points[i], obstacles[3], 1)) return 'THROUGH C';
    return 'around C';
  };
  console.log(`      a1-b1 ${passesC('a1-b1')}, a2-b2 ${passesC('a2-b2')}, A-B ${passesC('A-B')}; omitted as required: nested=${!routes.has('nested')} self=${!routes.has('self')} missing=${!routes.has('missing')}; a1-c1 routed=${routes.has('a1-c1')}`);
}

// 3. Parallel edges.
{
  const obstacles = [
    { id: 's', cx: 0, cy: 0, w: 160, h: 100, ancestors: [] },
    { id: 't', cx: 600, cy: 300, w: 160, h: 100, ancestors: [] },
    { id: 'mid', cx: 300, cy: 150, w: 120, h: 80, ancestors: [] }
  ];
  const edges = [];
  for (let k = 0; k < 5; k++) edges.push({ id: `p${k}`, source: k % 2 ? 't' : 's', target: k % 2 ? 's' : 't' });
  const { routes } = run('parallel x5', { obstacles, edges });
  const firsts = edges.map(e => routes.get(e.id)).filter(Boolean).map(r => r.points);
  // Distances between corresponding middle segments.
  const mids = firsts.map(p => p[Math.floor(p.length / 2)]);
  console.log('      middle points:', mids.map(m => `(${m.x.toFixed(0)},${m.y.toFixed(0)})`).join(' '));
}

// 4. Side costs: only the bottom side may be used at the source, only the left side at the target.
{
  const obstacles = [
    { id: 's', cx: 0, cy: 0, w: 160, h: 100, ancestors: [] },
    { id: 't', cx: 500, cy: -200, w: 160, h: 100, ancestors: [] }
  ];
  const edges = [{ id: 'sc', source: 's', target: 't', sourceSides: { top: Infinity, left: Infinity, right: Infinity }, targetSides: { top: Infinity, right: Infinity, bottom: Infinity } }];
  const { routes } = run('side costs', { obstacles, edges });
  const p = routes.get('sc').points;
  console.log(`      starts at (${p[0].x},${p[0].y}) [bottom y=50], ends at (${p[p.length - 1].x},${p[p.length - 1].y}) [left x=420]`);
}

// 5. Packages collapsing many cards: 8 packages x 30 cards, 400 edges between packages.
{
  const obstacles = [];
  const cards = [];
  for (let p = 0; p < 8; p++) {
    const px = (p % 4) * 1500;
    const py = Math.floor(p / 4) * 1500;
    obstacles.push({ id: `P${p}`, cx: px + 600, cy: py + 600, w: 1200, h: 1200, ancestors: [] });
    for (let k = 0; k < 30; k++) {
      const c = { id: `P${p}c${k}`, cx: px + 100 + (k % 6) * 200, cy: py + 120 + Math.floor(k / 6) * 220, w: 140, h: 100, ancestors: [`P${p}`] };
      obstacles.push(c);
      cards.push(c);
    }
  }
  const r = rng(5);
  const edges = [];
  for (let k = 0; k < 400; k++) {
    const a = cards[Math.floor(r() * cards.length)];
    const b = cards[Math.floor(r() * cards.length)];
    if (a !== b) edges.push({ id: `x${k}`, source: a.id, target: b.id });
  }
  run('8 packages x 30 cards / 400 edges', { obstacles, edges });
}

// 6. Performance: 1,700 obstacles, 1,500 edges (mostly local, 15% long).
{
  const obs = field(7, 50, 34);
  const edges = randomEdges(8, obs, 1500, 50, 34, 0.15, 3);
  run('field-1700 / 1500 edges', { obstacles: obs, edges });
}

// 7. Performance: 50 obstacles, 300 local edges.
{
  const obs = field(9, 10, 5);
  const edges = randomEdges(10, obs, 300, 10, 5, 0.0, 2);
  run('field-50 / 300 local edges', { obstacles: obs, edges });
}

// 9. Nested packages at scale: 60 packages x 30 cards (1,860 obstacles), 1,700 edges (60% inside a package).
function nestedScale(seed) {
  const r = rng(seed);
  const obstacles = [];
  const byPkg = [];
  for (let p = 0; p < 60; p++) {
    const px = (p % 10) * 1450;
    const py = Math.floor(p / 10) * 1450;
    obstacles.push({ id: `Q${p}`, cx: px + 600, cy: py + 620, w: 1200, h: 1240, ancestors: [] });
    const cards = [];
    for (let k = 0; k < 30; k++) {
      const w = 120 + Math.floor(r() * 50);
      const h = 80 + Math.floor(r() * 50);
      const c = { id: `Q${p}c${k}`, cx: px + 110 + (k % 6) * 195, cy: py + 150 + Math.floor(k / 6) * 220, w, h, ancestors: [`Q${p}`] };
      obstacles.push(c);
      cards.push(c);
    }
    byPkg.push(cards);
  }
  const edges = [];
  for (let k = 0; edges.length < 1700; k++) {
    const pa = Math.floor(r() * 60);
    const a = byPkg[pa][Math.floor(r() * 30)];
    let b;
    if (r() < 0.6) b = byPkg[pa][Math.floor(r() * 30)];
    else {
      // Mostly neighbouring packages, some far.
      const pb = r() < 0.7 ? Math.min(59, Math.max(0, pa + [1, -1, 10, -10][Math.floor(r() * 4)])) : Math.floor(r() * 60);
      b = byPkg[pb][Math.floor(r() * 30)];
    }
    if (a !== b) edges.push({ id: `q${k}`, source: a.id, target: b.id });
  }
  return { obstacles, edges };
}
for (const budget of [3000, 400]) {
  const input = nestedScale(31);
  const opts = { margin: 14, bendPenalty: 40, spacing: 10, timeBudgetMs: budget };
  run(`nested 1860 obst / 1700 e (budget ${budget})`, input, opts);
  const a = JSON.stringify([...routeOrthogonal(input, opts).entries()]);
  const b = JSON.stringify([...routeOrthogonal(input, opts).entries()]);
  console.log(`${a === b ? 'PASS' : 'FAIL'} determinism (nested scale, budget ${budget})`);
}
{
  const obs = field(7, 50, 34);
  const edges = randomEdges(8, obs, 1500, 50, 34, 0.15, 3);
  const opts = { margin: 14, timeBudgetMs: 400 };
  run('field-1700 / 1500 edges (budget 400)', { obstacles: obs, edges }, opts);
  const a = JSON.stringify([...routeOrthogonal({ obstacles: obs, edges }, opts).entries()]);
  const b = JSON.stringify([...routeOrthogonal({ obstacles: obs, edges }, opts).entries()]);
  console.log(`${a === b ? 'PASS' : 'FAIL'} determinism (field-1700, budget 400)`);
}

// 8. Determinism.
{
  const obs = field(11, 12, 8);
  const edges = randomEdges(12, obs, 200, 12, 8, 0.2, 3);
  const a = JSON.stringify([...routeOrthogonal({ obstacles: obs, edges }).entries()]);
  const b = JSON.stringify([...routeOrthogonal({ obstacles: obs, edges }).entries()]);
  console.log(`${a === b ? 'PASS' : 'FAIL'} determinism`);
}
