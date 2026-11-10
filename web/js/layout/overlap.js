/**
 * OpenHeart layout: overlap removal and compaction by separation-constraint projection.
 *
 * Algorithm: VPSC (Variable Placement with Separation Constraints) from
 *   T. Dwyer, K. Marriott and P. J. Stuckey, "Fast Node Overlap Removal",
 *   Graph Drawing 2005, LNCS 3843, pp. 153-164.
 *
 * solveSeparation is the paper's quadratic-programming solver. `satisfy` builds blocks of
 * variables joined by tight (active) constraints: variables are processed in a total order and
 * each block is merged with the block across its most violated incoming constraint. `refine`
 * then splits blocks on active constraints whose Lagrange multiplier is negative and
 * re-satisfies, which moves the solution to the weighted least-squares optimum.
 *
 * removeOverlaps follows the paper's two-pass scheme: x constraints from a scanline over y (a
 * pair is separated horizontally when that is the cheaper axis), then y constraints between
 * scanline neighbours over x, which separates every pair still overlapping in x.
 *
 * Public gap options are the clearance between borders, as in geometry.rectsOverlap. Internally
 * the module works with g = 2 * clearance: two rectangles are clear of each other when their
 * centres are at least (a + b + g) / 2 apart on some axis, where a and b are their sizes on that
 * axis.
 */
import { SPACING } from './geometry.js';

const LAGRANGIAN_TOLERANCE = -1e-4;
const MAX_REFINE_PASSES = 100;
const SLACK_EPS = 1e-10;
const SEPARATION_EPS = 1e-6;
const FIXED_WEIGHT = 1e6;
const PUSH_APART_ITERATIONS = 24;

/** Kahn's algorithm over the constraint graph; ties by variable index. Cyclic leftovers go last. */
function topologicalOrder(n, inC, outC) {
  const indeg = new Int32Array(n);
  for (let v = 0; v < n; v++) indeg[v] = inC[v].length;
  const order = [];
  for (let v = 0; v < n; v++) if (indeg[v] === 0) order.push(v);
  for (let head = 0; head < order.length; head++) {
    const v = order[head];
    for (const c of outC[v]) if (--indeg[c.right] === 0) order.push(c.right);
  }
  if (order.length < n) {
    const seen = new Uint8Array(n);
    for (const v of order) seen[v] = 1;
    for (let v = 0; v < n; v++) if (!seen[v]) order.push(v);
  }
  return order;
}

/**
 * Solves min sum w_i (pos_i - desired_i)^2 subject to pos[right] - pos[left] >= gap (VPSC).
 * @param {Array<{ desired: number, weight?: number }>} vars weight defaults to 1; fixed
 *   variables use a large weight such as 1e6.
 * @param {Array<{ left: number, right: number, gap: number }>} constraints indices into vars;
 *   the constraint graph must be acyclic. Invalid entries are ignored.
 * @returns {Float64Array} positions, always feasible for an acyclic constraint set.
 */
export function solveSeparation(vars, constraints) {
  const n = Array.isArray(vars) ? vars.length : 0;
  const result = new Float64Array(n);
  if (n === 0) return result;

  const desired = new Float64Array(n);
  const weight = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const v = vars[i];
    desired[i] = v && Number.isFinite(v.desired) ? v.desired : 0;
    weight[i] = v && Number.isFinite(v.weight) && v.weight > 0 ? v.weight : 1;
  }

  const inC = Array.from({ length: n }, () => []);
  const outC = Array.from({ length: n }, () => []);
  let m = 0;
  if (Array.isArray(constraints)) {
    for (const raw of constraints) {
      if (!raw) continue;
      const l = raw.left;
      const r = raw.right;
      if (!Number.isInteger(l) || !Number.isInteger(r) || l < 0 || r < 0 || l >= n || r >= n || l === r) continue;
      const c = { left: l, right: r, gap: Number.isFinite(raw.gap) ? raw.gap : 0, active: false, lm: 0, id: m++ };
      outC[l].push(c);
      inC[r].push(c);
    }
  }
  if (m === 0) {
    result.set(desired);
    return result;
  }
  const order = topologicalOrder(n, inC, outC);

  // ── Blocks ── position(v) = block.posn + offset[v]; block.posn = AB / B is the weighted optimum.
  const offset = new Float64Array(n);
  const blockOf = new Array(n);
  const blocks = [];
  const makeBlock = () => {
    const b = { vars: [], AB: 0, B: 0, posn: 0, inList: null, outList: null, deleted: false };
    blocks.push(b);
    return b;
  };
  const addVar = (b, v) => {
    blockOf[v] = b;
    b.vars.push(v);
    b.AB += weight[v] * (desired[v] - offset[v]);
    b.B += weight[v];
  };
  const updatePosn = (b) => { b.posn = b.B > 0 ? b.AB / b.B : 0; };
  const pos = (v) => blockOf[v].posn + offset[v];
  const slack = (c) => pos(c.right) - c.gap - pos(c.left);

  for (let v = 0; v < n; v++) {
    const b = makeBlock();
    addVar(b, v);
    updatePosn(b);
  }

  const setUpIn = (b) => {
    const list = [];
    for (const v of b.vars) for (const c of inC[v]) if (blockOf[c.left] !== b) list.push(c);
    b.inList = list;
  };
  const setUpOut = (b) => {
    const list = [];
    for (const v of b.vars) for (const c of outC[v]) if (blockOf[c.right] !== b) list.push(c);
    b.outList = list;
  };
  // Most violated constraint crossing into (or out of) the block; internal ones are dropped.
  const findMinIn = (b) => {
    if (!b.inList) setUpIn(b);
    const list = b.inList;
    let best = null;
    let bestSlack = Infinity;
    let w = 0;
    for (let i = 0; i < list.length; i++) {
      const c = list[i];
      if (blockOf[c.left] === b) continue;
      list[w++] = c;
      const s = slack(c);
      if (s < bestSlack || (s === bestSlack && best && c.id < best.id)) {
        best = c;
        bestSlack = s;
      }
    }
    list.length = w;
    return best;
  };
  const findMinOut = (b) => {
    if (!b.outList) setUpOut(b);
    const list = b.outList;
    let best = null;
    let bestSlack = Infinity;
    let w = 0;
    for (let i = 0; i < list.length; i++) {
      const c = list[i];
      if (blockOf[c.right] === b) continue;
      list[w++] = c;
      const s = slack(c);
      if (s < bestSlack || (s === bestSlack && best && c.id < best.id)) {
        best = c;
        bestSlack = s;
      }
    }
    list.length = w;
    return best;
  };

  // Moves absorb's variables into keep, shifting their offsets by dist, and makes c active.
  const mergeInto = (keep, absorb, c, dist) => {
    c.active = true;
    for (const v of absorb.vars) {
      offset[v] += dist;
      addVar(keep, v);
    }
    updatePosn(keep);
    keep.inList = keep.inList && absorb.inList ? keep.inList.concat(absorb.inList) : null;
    keep.outList = keep.outList && absorb.outList ? keep.outList.concat(absorb.outList) : null;
    absorb.deleted = true;
    absorb.vars = [];
    absorb.inList = null;
    absorb.outList = null;
    return keep;
  };

  const mergeLeft = (start) => {
    let r = start;
    let c = findMinIn(r);
    while (c && slack(c) < -SLACK_EPS) {
      const l = blockOf[c.left];
      if (!l.inList) setUpIn(l);
      if (r.vars.length >= l.vars.length) {
        r = mergeInto(r, l, c, offset[c.right] - offset[c.left] - c.gap);
      } else {
        r = mergeInto(l, r, c, offset[c.left] + c.gap - offset[c.right]);
      }
      c = findMinIn(r);
    }
    return r;
  };

  const mergeRight = (start) => {
    let l = start;
    let c = findMinOut(l);
    while (c && slack(c) < -SLACK_EPS) {
      const r = blockOf[c.right];
      if (!r.outList) setUpOut(r);
      if (l.vars.length >= r.vars.length) {
        l = mergeInto(l, r, c, offset[c.left] + c.gap - offset[c.right]);
      } else {
        l = mergeInto(r, l, c, offset[c.right] - offset[c.left] - c.gap);
      }
      c = findMinOut(l);
    }
    return l;
  };

  // ── satisfy ──
  for (const v of order) mergeLeft(blockOf[v]);

  // ── refine ──
  const mark = new Int32Array(n);
  let stamp = 0;
  const parentCon = new Array(n);
  const dfdv = new Float64Array(n);

  // Lagrange multipliers over the block's spanning tree of active constraints; returns the
  // active constraint with the smallest multiplier.
  const minLagrangian = (b) => {
    if (b.vars.length < 2) return null;
    stamp++;
    const root = b.vars[0];
    const visit = [root];
    mark[root] = stamp;
    parentCon[root] = null;
    for (let i = 0; i < visit.length; i++) {
      const v = visit[i];
      dfdv[v] = 2 * weight[v] * (pos(v) - desired[v]);
      for (const c of outC[v]) {
        if (c.active && mark[c.right] !== stamp) {
          mark[c.right] = stamp;
          parentCon[c.right] = c;
          visit.push(c.right);
        }
      }
      for (const c of inC[v]) {
        if (c.active && mark[c.left] !== stamp) {
          mark[c.left] = stamp;
          parentCon[c.left] = c;
          visit.push(c.left);
        }
      }
    }
    let best = null;
    for (let i = visit.length - 1; i > 0; i--) {
      const v = visit[i];
      const c = parentCon[v];
      if (c.right === v) {
        c.lm = dfdv[v];
        dfdv[c.left] += c.lm;
      } else {
        c.lm = -dfdv[v];
        dfdv[c.right] -= c.lm;
      }
      if (!best || c.lm < best.lm || (c.lm === best.lm && c.id < best.id)) best = c;
    }
    return best;
  };

  const populate = (b, start) => {
    stamp++;
    mark[start] = stamp;
    const stack = [start];
    while (stack.length) {
      const v = stack.pop();
      addVar(b, v);
      for (const c of outC[v]) {
        if (c.active && mark[c.right] !== stamp) {
          mark[c.right] = stamp;
          stack.push(c.right);
        }
      }
      for (const c of inC[v]) {
        if (c.active && mark[c.left] !== stamp) {
          mark[c.left] = stamp;
          stack.push(c.left);
        }
      }
    }
    updatePosn(b);
  };

  const splitBlock = (b, c) => {
    c.active = false;
    const oldPosn = b.posn;
    b.deleted = true;
    b.vars = [];
    b.inList = null;
    b.outList = null;
    const l = makeBlock();
    populate(l, c.left);
    const r = makeBlock();
    populate(r, c.right);
    // The right part stays put until the left part has settled.
    r.posn = oldPosn;
    mergeLeft(l);
    const rr = blockOf[c.right];
    updatePosn(rr);
    mergeRight(rr);
  };

  for (let pass = 0; pass < MAX_REFINE_PASSES; pass++) {
    let changed = false;
    const snapshot = blocks.filter(b => !b.deleted);
    for (const b of snapshot) {
      if (b.deleted) continue;
      const c = minLagrangian(b);
      if (c && c.lm < LAGRANGIAN_TOLERANCE) {
        splitBlock(b, c);
        changed = true;
      }
    }
    if (!changed) break;
  }

  for (let v = 0; v < n; v++) {
    const p = pos(v);
    result[v] = Number.isFinite(p) ? p : desired[v];
  }
  // Feasibility guarantee: push right-hand variables in topological order.
  for (const v of order) {
    for (const c of inC[v]) {
      const need = result[c.left] + c.gap;
      if (result[v] < need) result[v] = need;
    }
  }
  return result;
}

// ── Rectangle helpers ──

function compareKeys(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Wraps valid rectangles with sanitised sizes and an id rank for stable tie-breaks. */
function prepare(rects) {
  const items = [];
  if (!Array.isArray(rects)) return items;
  for (const r of rects) {
    if (!r || !Number.isFinite(r.cx) || !Number.isFinite(r.cy)) continue;
    items.push({
      r,
      w: Number.isFinite(r.w) && r.w > 0 ? r.w : 0,
      h: Number.isFinite(r.h) && r.h > 0 ? r.h : 0,
      fixed: Boolean(r.fixed),
      key: r.id === undefined || r.id === null ? '' : String(r.id),
      rank: 0
    });
  }
  const byKey = items.map((_, i) => i).sort((a, b) => compareKeys(items[a].key, items[b].key) || a - b);
  byKey.forEach((idx, k) => { items[idx].rank = k; });
  return items;
}

const centerOf = (it, axis) => (axis === 'x' ? it.r.cx : it.r.cy);
const sizeOf = (it, axis) => (axis === 'x' ? it.w : it.h);
const setCenter = (it, axis, value) => {
  if (axis === 'x') it.r.cx = value;
  else it.r.cy = value;
};

/** Positive extent of the overlap of a and b on each axis, with the internal g = 2 * clearance. */
function extent(a, b, gap) {
  return {
    x: (a.w + b.w + gap) / 2 - Math.abs(a.r.cx - b.r.cx),
    y: (a.h + b.h + gap) / 2 - Math.abs(a.r.cy - b.r.cy)
  };
}

function itemsOverlap(a, b, gap) {
  return Math.abs(a.r.cx - b.r.cx) * 2 < a.w + b.w + gap && Math.abs(a.r.cy - b.r.cy) * 2 < a.h + b.h + gap;
}

/** Overlapping pairs [i, j] by a sweep over left edges; stops after `limit` pairs. */
function overlappingPairs(items, gap, limit = Infinity) {
  const pad = gap / 4;
  const left = (i) => items[i].r.cx - items[i].w / 2 - pad;
  const right = (i) => items[i].r.cx + items[i].w / 2 + pad;
  const order = items.map((_, i) => i).sort((a, b) => (left(a) - left(b)) || (items[a].rank - items[b].rank));
  const pairs = [];
  let active = [];
  for (const i of order) {
    const li = left(i);
    if (active.length) active = active.filter(j => right(j) > li);
    for (const j of active) {
      if (itemsOverlap(items[i], items[j], gap)) {
        pairs.push([j, i]);
        if (pairs.length >= limit) return pairs;
      }
    }
    active.push(i);
  }
  return pairs;
}

/** Binary search for the insertion index of v in scanline S ordered by `before`. */
function lowerBound(S, v, before) {
  let lo = 0;
  let hi = S.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (before(S[mid], v) < 0) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Sweep events along `axis` (open/close of each inflated rectangle), closes first on ties. */
function sweepEvents(items, gap, axis) {
  const pad = gap / 4;
  const events = [];
  for (let i = 0; i < items.length; i++) {
    const half = sizeOf(items[i], axis) / 2 + pad;
    const c = centerOf(items[i], axis);
    events.push({ pos: c - half, open: true, i }, { pos: c + half, open: false, i });
  }
  events.sort((a, b) => (a.pos - b.pos) ||
    (a.open === b.open ? items[a.i].rank - items[b.i].rank : (a.open ? 1 : -1)));
  return events;
}

/**
 * The paper's x-constraint generation: a scanline over y; when a rectangle opens, walk its
 * scanline neighbours outward, keeping those where horizontal separation is the cheaper fix and
 * stopping at the first neighbour already clear in x. Constraints are emitted on close.
 */
function paperXConstraints(items, gap) {
  const n = items.length;
  const before = (a, b) => (items[a].r.cx - items[b].r.cx) || (items[a].rank - items[b].rank);
  const leftN = Array.from({ length: n }, () => new Set());
  const rightN = Array.from({ length: n }, () => new Set());
  const cons = [];
  const seen = new Set();
  const add = (u, v) => {
    const key = u * n + v;
    if (seen.has(key)) return;
    seen.add(key);
    cons.push({ left: u, right: v, gap: (items[u].w + items[v].w + gap) / 2 + SEPARATION_EPS });
  };
  const S = [];
  for (const ev of sweepEvents(items, gap, 'y')) {
    const v = ev.i;
    if (ev.open) {
      const k = lowerBound(S, v, before);
      S.splice(k, 0, v);
      for (let j = k - 1; j >= 0; j--) {
        const u = S[j];
        const e = extent(items[u], items[v], gap);
        if (e.x <= 0) {
          leftN[v].add(u);
          rightN[u].add(v);
          break;
        }
        if (e.x <= e.y) {
          leftN[v].add(u);
          rightN[u].add(v);
        }
      }
      for (let j = k + 1; j < S.length; j++) {
        const u = S[j];
        const e = extent(items[v], items[u], gap);
        if (e.x <= 0) {
          rightN[v].add(u);
          leftN[u].add(v);
          break;
        }
        if (e.x <= e.y) {
          rightN[v].add(u);
          leftN[u].add(v);
        }
      }
    } else {
      for (const u of leftN[v]) {
        add(u, v);
        rightN[u].delete(v);
      }
      for (const u of rightN[v]) {
        add(v, u);
        leftN[u].delete(v);
      }
      leftN[v].clear();
      rightN[v].clear();
      const k = lowerBound(S, v, before);
      if (S[k] === v) S.splice(k, 1);
      else {
        const idx = S.indexOf(v);
        if (idx >= 0) S.splice(idx, 1);
      }
    }
  }
  return cons;
}

/**
 * Constraints along `axis` between neighbours of a scanline swept along the other axis: a pair
 * is constrained whenever it becomes adjacent while both are open. Every pair whose projections
 * on the other axis overlap is then separated through a chain of constraints, in its current
 * order along `axis`.
 */
function adjacencyConstraints(items, gap, axis) {
  const n = items.length;
  const other = axis === 'x' ? 'y' : 'x';
  const before = (a, b) => (centerOf(items[a], axis) - centerOf(items[b], axis)) || (items[a].rank - items[b].rank);
  const cons = [];
  const seen = new Set();
  const add = (u, v) => {
    const key = u * n + v;
    if (seen.has(key)) return;
    seen.add(key);
    cons.push({ left: u, right: v, gap: (sizeOf(items[u], axis) + sizeOf(items[v], axis) + gap) / 2 + SEPARATION_EPS });
  };
  const S = [];
  for (const ev of sweepEvents(items, gap, other)) {
    const v = ev.i;
    if (ev.open) {
      const k = lowerBound(S, v, before);
      S.splice(k, 0, v);
      if (k > 0) add(S[k - 1], v);
      if (k + 1 < S.length) add(v, S[k + 1]);
    } else {
      let k = lowerBound(S, v, before);
      if (S[k] !== v) k = S.indexOf(v);
      if (k < 0) continue;
      if (k > 0 && k + 1 < S.length) add(S[k - 1], S[k + 1]);
      S.splice(k, 1);
    }
  }
  return cons;
}

function solveAxis(items, cons, axis, desiredOf) {
  const vars = items.map((it, i) => ({ desired: desiredOf(it, i), weight: it.fixed ? FIXED_WEIGHT : 1 }));
  const before = items.map(it => centerOf(it, axis));
  const pos = solveSeparation(vars, cons);
  items.forEach((it, i) => {
    let p = pos[i];
    // A fixed rectangle that the solver only nudged by rounding goes back exactly.
    if (it.fixed && Math.abs(p - before[i]) < 0.01) p = before[i];
    setCenter(it, axis, p);
  });
}

function pushApart(items, gap) {
  for (let iter = 0; iter < PUSH_APART_ITERATIONS; iter++) {
    const pairs = overlappingPairs(items, gap);
    if (pairs.length === 0) return;
    for (const [i, j] of pairs) {
      const A = items[i];
      const B = items[j];
      const e = extent(A, B, gap);
      if (e.x <= 0 || e.y <= 0) continue;
      const axis = e.x <= e.y ? 'x' : 'y';
      const amount = (axis === 'x' ? e.x : e.y) + 1e-3;
      const ca = centerOf(A, axis);
      const cb = centerOf(B, axis);
      const aFirst = ca < cb || (ca === cb && A.rank < B.rank);
      const lo = aFirst ? A : B;
      const hi = aFirst ? B : A;
      if (hi.fixed && !lo.fixed) {
        setCenter(lo, axis, centerOf(lo, axis) - amount);
      } else if (lo.fixed || hi.fixed) {
        setCenter(hi, axis, centerOf(hi, axis) + amount);
      } else {
        setCenter(lo, axis, centerOf(lo, axis) - amount / 2);
        setCenter(hi, axis, centerOf(hi, axis) + amount / 2);
      }
    }
  }
}

/**
 * Counts overlapping pairs, using the same test as geometry.rectsOverlap(a, b, gap).
 * @param {Array<{ id?: string, cx: number, cy: number, w: number, h: number }>} rects
 * @param {number} [gap]
 * @returns {number}
 */
export function countOverlaps(rects, gap = 0) {
  const items = prepare(rects);
  return items.length < 2 ? 0 : overlappingPairs(items, 2 * (Number.isFinite(gap) ? gap : 0)).length;
}

/**
 * Removes every overlap between rectangles while moving them as little as possible (VPSC):
 * x pass from the paper's scanline, then a y pass over every pair still overlapping in x,
 * repeated until clean. If anything is left after maxPasses, pairs are pushed apart along their
 * cheaper axis, and as a last resort every pair overlapping in y is separated in x.
 * @param {Array<{ id: string, cx: number, cy: number, w: number, h: number, fixed?: boolean }>} rects mutated in place
 * @param {{ gap?: number, maxPasses?: number }} [options] gap defaults to SPACING.overlapGap
 * @returns {number} overlapping pairs remaining (0 on success)
 */
export function removeOverlaps(rects, options = {}) {
  const opts = options || {};
  const gap = 2 * (Number.isFinite(opts.gap) ? Math.max(0, opts.gap) : SPACING.overlapGap);
  const maxPasses = Number.isInteger(opts.maxPasses) && opts.maxPasses > 0 ? opts.maxPasses : 8;
  const items = prepare(rects);
  if (items.length < 2) return 0;
  for (let pass = 0; pass < maxPasses; pass++) {
    if (overlappingPairs(items, gap, 1).length === 0) return 0;
    solveAxis(items, paperXConstraints(items, gap), 'x', it => it.r.cx);
    solveAxis(items, adjacencyConstraints(items, gap, 'y'), 'y', it => it.r.cy);
  }
  if (overlappingPairs(items, gap, 1).length === 0) return 0;
  pushApart(items, gap);
  if (overlappingPairs(items, gap, 1).length > 0) {
    solveAxis(items, adjacencyConstraints(items, gap, 'x'), 'x', it => it.r.cx);
  }
  return overlappingPairs(items, gap).length;
}

/**
 * Pulls rectangles together, x then y, without creating overlap and without changing the
 * left/right order of pairs that share a row or the above/below order of pairs that share a
 * column. Desired positions are the current ones moved toward the centroid by `strength`.
 * Fixed rectangles keep their position.
 * @param {Array<{ id: string, cx: number, cy: number, w: number, h: number, fixed?: boolean }>} rects mutated in place
 * @param {{ gap?: number, strength?: number }} [options] strength in [0, 1], default 0.5
 * @returns {Array<object>} the same array
 */
export function compact(rects, options = {}) {
  const opts = options || {};
  const gap = 2 * (Number.isFinite(opts.gap) ? Math.max(0, opts.gap) : SPACING.overlapGap);
  const strength = Number.isFinite(opts.strength) ? Math.min(1, Math.max(0, opts.strength)) : 0.5;
  const items = prepare(rects);
  if (items.length < 2 || strength === 0) return rects;
  for (const axis of ['x', 'y']) {
    let sum = 0;
    for (const it of items) sum += centerOf(it, axis);
    const centroid = sum / items.length;
    const cons = adjacencyConstraints(items, gap, axis);
    solveAxis(items, cons, axis, (it) => {
      const c = centerOf(it, axis);
      return it.fixed ? c : c + strength * (centroid - c);
    });
  }
  return rects;
}
