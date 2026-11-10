// Property tests for overlap.js (VPSC) and pack.js on random inputs.
import { pathToFileURL, fileURLToPath } from 'node:url';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
const REPO = process.env.OH_REPO || path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const mod = p => import(pathToFileURL(path.join(REPO, p)).href);
const { removeOverlaps, compact, solveSeparation } = await mod('web/js/layout/overlap.js');
const { packBoxes } = await mod('web/js/layout/pack.js');
const G = await mod('web/js/layout/geometry.js');

function rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6D2B79F5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
const fails = [];
const check = (cond, msg) => { if (!cond) fails.push(msg); };

function clearance(rects) {
  let worst = Infinity;
  for (let i = 0; i < rects.length; i++) for (let j = i + 1; j < rects.length; j++) {
    const a = rects[i], b = rects[j];
    const dx = Math.abs(a.cx - b.cx) - (a.w + b.w) / 2;
    const dy = Math.abs(a.cy - b.cy) - (a.h + b.h) / 2;
    worst = Math.min(worst, Math.max(dx, dy));
  }
  return worst;
}

// 1. solveSeparation: feasibility and optimality on a chain.
{
  const vars = [{ desired: 0 }, { desired: 0 }, { desired: 0 }];
  const cons = [{ left: 0, right: 1, gap: 10 }, { left: 1, right: 2, gap: 10 }];
  const p = solveSeparation(vars, cons);
  check(Math.abs(p[1] - p[0] - 10) < 1e-6 && Math.abs(p[2] - p[1] - 10) < 1e-6, `chain gaps ${[...p]}`);
  check(Math.abs(p[0] + p[1] + p[2]) < 1e-6, `chain centred (least squares) ${[...p]}`);
}

// 2. removeOverlaps on random piles: zero overlaps with the requested clearance.
for (const [seed, n, spread] of [[1, 30, 400], [2, 200, 1500], [3, 800, 3000], [4, 3000, 8000]]) {
  const r = rng(seed);
  const rects = Array.from({ length: n }, (_, i) => ({ id: `r${i}`, cx: r() * spread, cy: r() * spread * 0.6, w: 60 + r() * 240, h: 40 + r() * 200 }));
  const before = rects.map(x => ({ ...x }));
  const t0 = performance.now();
  const left = removeOverlaps(rects, { gap: 20 });
  const ms = performance.now() - t0;
  const c = n <= 800 ? clearance(rects) : null;
  let moved = 0;
  rects.forEach((x, i) => { moved += Math.hypot(x.cx - before[i].cx, x.cy - before[i].cy); });
  console.log(`removeOverlaps n=${n}: remaining=${left} minClearance=${c === null ? 'skipped' : c.toFixed(2)} meanMove=${(moved / n).toFixed(1)}px ${ms.toFixed(0)}ms`);
  check(left === 0, `removeOverlaps n=${n} left ${left}`);
  if (c !== null) check(c >= 20 - 1e-6, `removeOverlaps n=${n} clearance ${c}`);
  check(ms < 2000, `removeOverlaps n=${n} slow ${ms}`);
  // determinism
  const again = before.map(x => ({ ...x }));
  removeOverlaps(again, { gap: 20 });
  check(again.every((x, i) => x.cx === rects[i].cx && x.cy === rects[i].cy), `removeOverlaps n=${n} non-deterministic`);
}

// 3. Already-clean input must not move.
{
  const rects = [{ id: 'a', cx: 0, cy: 0, w: 100, h: 100 }, { id: 'b', cx: 300, cy: 0, w: 100, h: 100 }];
  removeOverlaps(rects, { gap: 20 });
  check(rects[0].cx === 0 && rects[1].cx === 300, 'clean input moved');
}

// 4. compact keeps zero overlaps and shrinks the spread.
{
  const r = rng(9);
  const rects = Array.from({ length: 60 }, (_, i) => ({ id: `c${i}`, cx: (i % 8) * 900 + r() * 50, cy: Math.floor(i / 8) * 700, w: 200, h: 150 }));
  const b0 = G.boundsOf(rects);
  compact(rects, { gap: 30, strength: 0.6 });
  const b1 = G.boundsOf(rects);
  const c = clearance(rects);
  console.log(`compact: area ${Math.round(b0.w * b0.h)} -> ${Math.round(b1.w * b1.h)}, minClearance=${c.toFixed(2)}`);
  check(c >= 30 - 1e-6, `compact clearance ${c}`);
  check(b1.w * b1.h < b0.w * b0.h, 'compact did not shrink');
}

// 5. packBoxes: no overlaps, gaps respected, aspect near target, starts at 0,0.
for (const [seed, n, uniform] of [[5, 12, true], [6, 120, true], [7, 40, false], [8, 400, false]]) {
  const r = rng(seed);
  const boxes = Array.from({ length: n }, (_, i) => ({ id: `b${i}`, w: uniform ? 280 + r() * 40 : 80 + r() * 1400, h: uniform ? 180 + r() * 60 : 60 + r() * 900 }));
  const res = packBoxes(boxes, { gapX: 40, gapY: 40, aspectRatio: 1.6 });
  const rects = boxes.map(b => ({ ...res.positions.get(b.id), w: b.w, h: b.h }));
  const c = clearance(rects);
  const bb = G.boundsOf(rects);
  const used = boxes.reduce((s, b) => s + b.w * b.h, 0) / (res.width * res.height);
  console.log(`packBoxes n=${n} ${uniform ? 'uniform' : 'mixed'}: ${Math.round(res.width)}x${Math.round(res.height)} aspect=${(res.width / res.height).toFixed(2)} areaUse=${used.toFixed(2)} minGap=${c.toFixed(1)}`);
  check(c >= 40 - 1e-6, `packBoxes n=${n} gap ${c}`);
  check(Math.abs(bb.minX) < 1e-6 && Math.abs(bb.minY) < 1e-6, `packBoxes n=${n} origin ${bb.minX},${bb.minY}`);
  check(Math.abs(bb.w - res.width) < 1e-6 && Math.abs(bb.h - res.height) < 1e-6, `packBoxes n=${n} size mismatch`);
}

console.log(fails.length ? `FAILURES:\n- ${fails.join('\n- ')}` : 'ALL UNIT CHECKS PASSED');
