/**
 * OpenHeart orthogonal edge router.
 *
 * Routes edges as axis-parallel polylines around obstacles (cards and package footprints) and
 * spreads apart edges that share a corridor. Method after M. Wybrow, K. Marriott and P. J.
 * Stuckey, "Orthogonal Connector Routing", Graph Drawing 2009 (the libavoid approach), made
 * local per edge and coarse-to-fine for long edges:
 *
 *  1. A uniform-grid spatial index over every obstacle.
 *  2. Blockers. An edge may not enter any obstacle except the packages that contain either end
 *     (it must pass through those). An obstacle inside a package that is itself a blocker is
 *     covered by that package, so whole unrelated packages collapse into one blocker. The end
 *     nodes are blockers too, so the route touches them only through its end stubs. Blockers are
 *     inflated by the margin.
 *  3. Search region. Short edges search a window: the bounding box of their end nodes plus a
 *     padding, doubled while no path is found. Edges whose window would hold too many blockers
 *     are routed coarse-to-fine (global then detailed routing): an A* over a coarse cell grid,
 *     where cells fully inside a blocker are impassable and partly covered cells are expensive,
 *     picks a corridor; the corridor dilated by a few cells is the only region the detailed search
 *     may use.
 *  4. Detailed search on a sparse orthogonal visibility grid of the region: the x and y
 *     coordinates of every inflated blocker border, the end nodes' centre lines, the region border
 *     and the midlines of wide gaps. Because every blocker border is a grid line, a grid segment is
 *     blocked exactly when its midpoint lies strictly inside a blocker, and a vertex when it lies
 *     strictly inside. Validity is evaluated lazily, only where the search goes; state storage is
 *     dense (typed arrays) for small grids and sparse (maps) for large corridor grids.
 *  5. Ports at the four side midpoints of each end node, moved out by the margin; the stub from the
 *     node border to the port is part of the route. Per-side costs bias or forbid sides.
 *  6. A* over (vertex, incoming direction): cost = length + bend penalty per bend + side costs,
 *     Manhattan heuristic, binary heap, deterministic tie-breaks. As a last resort the margin is
 *     reduced.
 *  7. Parallel edges (same pair of end nodes) are routed once and share the path.
 *  8. Nudging (libavoid's orthogonal segment nudging): collinear overlapping segments of different
 *     edges are ordered so that no new crossings arise where they turn away, then spread a
 *     spacing apart around the shared line. Each segment is clamped so it stays clear of every
 *     obstacle and of the other parallel segments near it, never reverses an adjacent segment,
 *     and keeps its end stub on its node side away from the corners.
 *  9. Every final route is validated; a route that nudging pushed into an obstacle reverts to its
 *     un-nudged path, and a path that is still invalid is dropped. No returned route ever crosses
 *     an obstacle it may not enter.
 */

import { segmentIntersectsRectInterior, simplifyPolyline } from './geometry.js';

// Directions: 0 right (+x), 1 down (+y), 2 left (-x), 3 up (-y).
const DX = [1, 0, -1, 0];
const DY = [0, 1, 0, -1];
/** Node sides in a fixed order, with the direction leaving and entering through each. */
const SIDES = ['top', 'bottom', 'left', 'right'];
const OUTWARD = { top: 3, bottom: 1, left: 2, right: 0 };
const INWARD = { top: 1, bottom: 3, left: 0, right: 2 };

const EPS = 1e-7;
/** Largest grid searched with dense typed-array state storage. */
const MAX_DENSE_VERTICES = 1000000;
/** Largest virtual grid a corridor search may span (storage is sparse). */
const MAX_SPARSE_VERTICES = 400000000;
const MAX_EXPANSIONS = 1500000;
/**
 * A* heuristic weight. 1 keeps routes optimal. Above 1 (weighted A*) routes may cost up to that
 * factor more for far fewer expansions; measured on a 1,700-card field, 1.15 saved 3x the work
 * but added 27% more bends, so quality wins.
 */
const HEURISTIC_WEIGHT = 1;
/** Windows with more blockers than this are routed coarse-to-fine instead. */
const WINDOW_BLOCKER_LIMIT = 220;
const CORRIDOR_DILATIONS = [2, 4];
/** Coarse grid size target (cells over the whole drawing). */
const COARSE_TARGET_CELLS = 60000;
/** Edges whose ends are farther apart than this (px, Manhattan, and in coarse cells) route corridor-first. */
const LONG_EDGE_PX = 2500;
const LONG_EDGE_CELLS = 12;
/** Work budget per millisecond of timeBudgetMs (search expansions; calibrated in Node). */
const WORK_PER_MS = 2000;
/** Fallback after the work budget: a wide corridor with a fixed allowance per edge. */
const FALLBACK_DILATION = 6;
const FALLBACK_EXPANSIONS = 40000;
const STUB_CORNER_INSET = 6;
const NUDGE_CLEARANCE = 2;

const now = () => (typeof performance !== 'undefined' && typeof performance.now === 'function' ? performance.now() : Date.now());

function finiteRect(o) {
  return o && Number.isFinite(o.cx) && Number.isFinite(o.cy) && Number.isFinite(o.w) && Number.isFinite(o.h) && o.w > 0 && o.h > 0;
}

// ───────────────────────── spatial index ─────────────────────────

/** Uniform-grid bucket index over boxes with l/t/r/b fields. */
class GridIndex {
  constructor(items, cellHint) {
    this.items = items;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const it of items) {
      if (it.l < minX) minX = it.l;
      if (it.t < minY) minY = it.t;
      if (it.r > maxX) maxX = it.r;
      if (it.b > maxY) maxY = it.b;
    }
    if (minX === Infinity) {
      minX = 0;
      minY = 0;
      maxX = 1;
      maxY = 1;
    }
    const W = Math.max(1, maxX - minX);
    const H = Math.max(1, maxY - minY);
    let cell = Math.max(16, cellHint || Math.sqrt((W * H) / Math.max(1, items.length)));
    cell = Math.max(cell, W / 512, H / 512);
    this.minX = minX;
    this.minY = minY;
    this.cell = cell;
    this.cols = Math.max(1, Math.ceil(W / cell) + 1);
    this.rows = Math.max(1, Math.ceil(H / cell) + 1);
    this.cells = new Array(this.cols * this.rows);
    this.stamp = new Uint32Array(items.length);
    this.tick = 0;
    items.forEach((it, idx) => {
      const c0 = this.col(it.l);
      const c1 = this.col(it.r);
      const r0 = this.row(it.t);
      const r1 = this.row(it.b);
      for (let r = r0; r <= r1; r++) {
        for (let c = c0; c <= c1; c++) {
          const k = r * this.cols + c;
          (this.cells[k] || (this.cells[k] = [])).push(idx);
        }
      }
    });
  }

  col(x) {
    return Math.min(this.cols - 1, Math.max(0, Math.floor((x - this.minX) / this.cell)));
  }

  row(y) {
    return Math.min(this.rows - 1, Math.max(0, Math.floor((y - this.minY) / this.cell)));
  }

  /** Calls fn(item) once for every item whose box intersects [l, r] x [t, b] (closed). */
  query(l, t, r, b, fn) {
    this.tick = (this.tick + 1) >>> 0;
    if (this.tick === 0) {
      this.stamp.fill(0);
      this.tick = 1;
    }
    const tick = this.tick;
    const c0 = this.col(l);
    const c1 = this.col(r);
    const r0 = this.row(t);
    const r1 = this.row(b);
    for (let rr = r0; rr <= r1; rr++) {
      for (let cc = c0; cc <= c1; cc++) {
        const list = this.cells[rr * this.cols + cc];
        if (!list) continue;
        for (const idx of list) {
          if (this.stamp[idx] === tick) continue;
          this.stamp[idx] = tick;
          const it = this.items[idx];
          if (it.l <= r && it.r >= l && it.t <= b && it.b >= t) fn(it);
        }
      }
    }
  }
}

// ───────────────────────── binary heap ─────────────────────────

/** Min-heap of states keyed by f, ties broken by larger g, then smaller state id. */
class StateHeap {
  constructor() {
    this.f = [];
    this.g = [];
    this.s = [];
  }

  get size() {
    return this.s.length;
  }

  less(i, j) {
    const f = this.f;
    if (f[i] !== f[j]) return f[i] < f[j];
    if (this.g[i] !== this.g[j]) return this.g[i] > this.g[j];
    return this.s[i] < this.s[j];
  }

  swap(i, j) {
    const f = this.f;
    const g = this.g;
    const s = this.s;
    let t = f[i]; f[i] = f[j]; f[j] = t;
    t = g[i]; g[i] = g[j]; g[j] = t;
    t = s[i]; s[i] = s[j]; s[j] = t;
  }

  push(fv, gv, sv) {
    this.f.push(fv);
    this.g.push(gv);
    this.s.push(sv);
    let i = this.s.length - 1;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (!this.less(i, p)) break;
      this.swap(i, p);
      i = p;
    }
  }

  /** Removes the minimum and writes it to out. */
  pop(out) {
    out.f = this.f[0];
    out.g = this.g[0];
    out.s = this.s[0];
    const last = this.s.length - 1;
    if (last > 0) {
      this.f[0] = this.f[last];
      this.g[0] = this.g[last];
      this.s[0] = this.s[last];
    }
    this.f.pop();
    this.g.pop();
    this.s.pop();
    const n = this.s.length;
    let i = 0;
    for (;;) {
      const l = 2 * i + 1;
      const r = l + 1;
      let m = i;
      if (l < n && this.less(l, m)) m = l;
      if (r < n && this.less(r, m)) m = r;
      if (m === i) break;
      this.swap(i, m);
      i = m;
    }
  }
}

// Dense search buffers reused across edges (stamped, so they never need clearing).
let bufCap = 0;
let gBuf = null;
let parentBuf = null;
let stampBuf = null;
let stampTick = 0;

function ensureBuffers(states) {
  if (states <= bufCap) return;
  bufCap = Math.max(states, Math.ceil(bufCap * 1.5), 1024);
  gBuf = new Float64Array(bufCap);
  parentBuf = new Int32Array(bufCap);
  stampBuf = new Uint32Array(bufCap);
  stampTick = 0;
}

function nextStamp() {
  stampTick = (stampTick + 1) >>> 0;
  if (stampTick === 0) {
    stampBuf.fill(0);
    stampTick = 1;
  }
  return stampTick;
}

/** State storage: g score and parent per state, dense or sparse. */
function makeStore(states) {
  if (states <= 4 * MAX_DENSE_VERTICES) {
    ensureBuffers(states);
    const tick = nextStamp();
    return {
      g: (s) => (stampBuf[s] === tick ? gBuf[s] : Infinity),
      set: (s, g, parent) => {
        stampBuf[s] = tick;
        gBuf[s] = g;
        parentBuf[s] = parent;
      },
      parent: (s) => parentBuf[s]
    };
  }
  const gMap = new Map();
  const pMap = new Map();
  return {
    g: (s) => {
      const v = gMap.get(s);
      return v === undefined ? Infinity : v;
    },
    set: (s, g, parent) => {
      gMap.set(s, g);
      pMap.set(s, parent);
    },
    parent: (s) => {
      const v = pMap.get(s);
      return v === undefined ? -1 : v;
    }
  };
}

// ───────────────────────── helpers ─────────────────────────

function sortedUnique(values, lo, hi) {
  const v = values.filter(x => Number.isFinite(x) && x >= lo - EPS && x <= hi + EPS).sort((a, b) => a - b);
  const out = [];
  for (const x of v) {
    if (!out.length || x - out[out.length - 1] > 1e-6) out.push(x);
  }
  return out;
}

function exactIndex(arr, x) {
  let lo = 0;
  let hi = arr.length - 1;
  while (lo <= hi) {
    const m = (lo + hi) >> 1;
    const d = arr[m] - x;
    if (Math.abs(d) <= 1e-6) return m;
    if (d < 0) lo = m + 1;
    else hi = m - 1;
  }
  return -1;
}

function sideCost(costs, side) {
  if (!costs || typeof costs !== 'object') return 0;
  const v = costs[side];
  if (v === undefined || v === null) return 0;
  if (v === Infinity) return Infinity;
  return Number.isFinite(v) ? Math.max(0, v) : 0;
}

function borderPoint(node, side) {
  switch (side) {
    case 'top': return { x: node.cx, y: node.t };
    case 'bottom': return { x: node.cx, y: node.b };
    case 'left': return { x: node.l, y: node.cy };
    default: return { x: node.r, y: node.cy };
  }
}

function portPoint(node, side, m) {
  switch (side) {
    case 'top': return { x: node.cx, y: node.t - m };
    case 'bottom': return { x: node.cx, y: node.b + m };
    case 'left': return { x: node.l - m, y: node.cy };
    default: return { x: node.r + m, y: node.cy };
  }
}

function realRect(o) {
  return { cx: o.cx, cy: o.cy, w: o.w, h: o.h };
}

/**
 * Lower bound on the bends needed to travel from a point moving in direction d to a point offset
 * by (dx, dy) and arrive moving in direction da.
 */
function minBends(d, da, dx, dy) {
  const forward = dx * DX[d] + dy * DY[d];
  const lateral = Math.abs(dx * DY[d] - dy * DX[d]);
  if (d === da) return lateral < 1e-6 && forward >= -1e-6 ? 0 : 2;
  if (d === ((da + 2) & 3)) return 2;
  const aheadInArrival = dx * DX[da] + dy * DY[da];
  return forward >= -1e-6 && aheadInArrival >= -1e-6 ? 1 : 3;
}

// ───────────────────────── detailed search ─────────────────────────

/**
 * A* on the sparse visibility grid of one region (a window, optionally restricted to a corridor).
 * Returns { pts } with the polyline from the source border to the target border, { pts: null }
 * when there is no path, or { pts: null, tooBig: true } when the grid exceeds the vertex limit.
 */
function searchRegion(win, blockers, src, tgt, srcCosts, tgtCosts, margin, bendPenalty, corridor, work) {
  const inflated = blockers.map(o => ({ l: o.l - margin, t: o.t - margin, r: o.r + margin, b: o.b + margin, o }));

  const xsRaw = [win.l, win.r, src.cx, tgt.cx];
  const ysRaw = [win.t, win.b, src.cy, tgt.cy];
  for (const B of inflated) {
    xsRaw.push(B.l, B.r);
    ysRaw.push(B.t, B.b);
  }
  let xs = sortedUnique(xsRaw, win.l, win.r);
  let ys = sortedUnique(ysRaw, win.t, win.b);
  const vertexLimit = corridor ? MAX_SPARSE_VERTICES : MAX_DENSE_VERTICES;
  const addMidlines = (arr) => {
    const out = [arr[0]];
    for (let i = 1; i < arr.length; i++) {
      if (arr[i] - arr[i - 1] > 3 * margin + 1) out.push((arr[i] + arr[i - 1]) / 2);
      out.push(arr[i]);
    }
    return out;
  };
  if ((2 * xs.length) * (2 * ys.length) <= vertexLimit) {
    xs = addMidlines(xs);
    ys = addMidlines(ys);
  }
  const nx = xs.length;
  const ny = ys.length;
  if (nx * ny > vertexLimit) return { pts: null, tooBig: true };
  if (nx < 2 || ny < 2) return { pts: null };

  const local = new GridIndex(inflated);
  const strictlyInside = (x, y) => {
    const list = local.cells[local.row(y) * local.cols + local.col(x)];
    if (!list) return false;
    for (const idx of list) {
      const B = inflated[idx];
      if (x > B.l + EPS && x < B.r - EPS && y > B.t + EPS && y < B.b - EPS) return true;
    }
    return false;
  };
  const inRegion = corridor ? (x, y) => corridor.contains(x, y) : () => true;

  const vCacheDense = nx * ny <= MAX_DENSE_VERTICES ? new Int8Array(nx * ny) : null;
  const vCacheSparse = vCacheDense ? null : new Map();
  const vertexFree = (vid) => {
    let s = vCacheDense ? vCacheDense[vid] : (vCacheSparse.get(vid) || 0);
    if (s === 0) {
      const i = vid % nx;
      const j = (vid - i) / nx;
      const x = xs[i];
      const y = ys[j];
      s = !inRegion(x, y) || strictlyInside(x, y) ? 2 : 1;
      if (vCacheDense) vCacheDense[vid] = s;
      else vCacheSparse.set(vid, s);
    }
    return s === 1;
  };

  // Grid segment validity, cached per segment: the segment from a vertex to its right (+x) or
  // lower (+y) neighbour is stored at that vertex (bit 0/1 known, bit 2/3 free).
  const segCacheDense = vCacheDense ? new Uint8Array(nx * ny) : null;
  const segCacheSparse = segCacheDense ? null : new Map();
  const segmentFree = (vid, nvid, d) => {
    const base = d === 0 || d === 1 ? vid : nvid;
    const horizontalStep = d === 0 || d === 2;
    const knownBit = horizontalStep ? 1 : 2;
    const freeBit = horizontalStep ? 4 : 8;
    let bits = segCacheDense ? segCacheDense[base] : (segCacheSparse.get(base) || 0);
    if (!(bits & knownBit)) {
      const bi = base % nx;
      const bj = (base - bi) / nx;
      const mx = horizontalStep ? (xs[bi] + xs[bi + 1]) / 2 : xs[bi];
      const my = horizontalStep ? ys[bj] : (ys[bj] + ys[bj + 1]) / 2;
      const ok = !strictlyInside(mx, my) && inRegion(mx, my);
      bits |= knownBit | (ok ? freeBit : 0);
      if (segCacheDense) segCacheDense[base] = bits;
      else segCacheSparse.set(base, bits);
    }
    return (bits & freeBit) !== 0;
  };

  // The stub from a node border to its port may not enter any blocker but its own node.
  const stubClear = (node, from, to) => {
    for (const B of inflated) {
      if (B.o === node) continue;
      if (segmentIntersectsRectInterior(from, to, realRect(B.o), 0.5)) return false;
    }
    return true;
  };

  const srcPorts = [];
  for (const side of SIDES) {
    const cost = sideCost(srcCosts, side);
    if (cost === Infinity) continue;
    const p = portPoint(src, side, margin);
    const i = exactIndex(xs, p.x);
    const j = exactIndex(ys, p.y);
    if (i < 0 || j < 0) continue;
    const vid = j * nx + i;
    if (!vertexFree(vid)) continue;
    if (!stubClear(src, borderPoint(src, side), p)) continue;
    srcPorts.push({ side, vid, dir: OUTWARD[side], cost });
  }
  const tgtPorts = new Map();
  const tgtPortList = [];
  for (const side of SIDES) {
    const cost = sideCost(tgtCosts, side);
    if (cost === Infinity) continue;
    const p = portPoint(tgt, side, margin);
    const i = exactIndex(xs, p.x);
    const j = exactIndex(ys, p.y);
    if (i < 0 || j < 0) continue;
    const vid = j * nx + i;
    if (!vertexFree(vid)) continue;
    if (!stubClear(tgt, borderPoint(tgt, side), p)) continue;
    const port = { side, vid, dir: INWARD[side], cost, x: p.x, y: p.y };
    tgtPortList.push(port);
    if (!tgtPorts.has(vid)) tgtPorts.set(vid, []);
    tgtPorts.get(vid).push(port);
  }
  if (!srcPorts.length || !tgtPortList.length) return { pts: null };

  // Admissible estimate: Manhattan distance plus the fewest bends that can still reach the port
  // and arrive in its inward direction, plus the port's side cost.
  const heuristic = (vid, din) => {
    const i = vid % nx;
    const j = (vid - i) / nx;
    const x = xs[i];
    const y = ys[j];
    let best = Infinity;
    for (const p of tgtPortList) {
      const dx = p.x - x;
      const dy = p.y - y;
      const est = Math.abs(dx) + Math.abs(dy) + bendPenalty * minBends(din, p.dir, dx, dy) + p.cost;
      if (est < best) best = est;
    }
    return best;
  };

  const store = makeStore(nx * ny * 4);
  const heap = new StateHeap();
  const out = { f: 0, g: 0, s: 0 };
  const hWeight = HEURISTIC_WEIGHT;
  const initSide = new Map();
  for (const p of srcPorts) {
    const st = p.vid * 4 + p.dir;
    if (store.g(st) <= p.cost) continue;
    store.set(st, p.cost, -1);
    initSide.set(st, p.side);
    heap.push(p.cost + hWeight * heuristic(p.vid, p.dir), p.cost, st);
  }

  let bestCost = Infinity;
  let bestState = -1;
  let bestPort = null;
  let expansions = 0;
  // Deterministic work limit: this search may use what is left of the shared budget.
  const cap = Math.min(MAX_EXPANSIONS, Math.max(0, work.limit - work.used));
  while (heap.size) {
    heap.pop(out);
    if (out.f >= bestCost) break;
    const st = out.s;
    if (out.g > store.g(st) + 1e-9) continue;
    if (++expansions > cap) break;
    const g = out.g;
    const din = st % 4;
    const vid = (st - din) / 4;
    const ports = tgtPorts.get(vid);
    if (ports) {
      for (const p of ports) {
        const total = g + p.cost + (din !== p.dir ? bendPenalty : 0);
        if (total < bestCost) {
          bestCost = total;
          bestState = st;
          bestPort = p;
        }
      }
    }
    const i = vid % nx;
    const j = (vid - i) / nx;
    for (let d = 0; d < 4; d++) {
      if (d === ((din + 2) & 3)) continue;
      const ni = i + DX[d];
      const nj = j + DY[d];
      if (ni < 0 || nj < 0 || ni >= nx || nj >= ny) continue;
      const nvid = nj * nx + ni;
      if (!vertexFree(nvid)) continue;
      if (!segmentFree(vid, nvid, d)) continue;
      const len = Math.abs(xs[ni] - xs[i]) + Math.abs(ys[nj] - ys[j]);
      const ng = g + len + (d !== din ? bendPenalty : 0);
      const ns = nvid * 4 + d;
      if (store.g(ns) <= ng) continue;
      store.set(ns, ng, st);
      heap.push(ng + hWeight * heuristic(nvid, d), ng, ns);
    }
  }
  work.used += expansions + 1;
  if (work.stats) {
    const S = work.stats;
    S.searches = (S.searches || 0) + 1;
    S.expansions = (S.expansions || 0) + expansions;
    S.gridVertices = (S.gridVertices || 0) + nx * ny;
    if (corridor) S.corridorSearches = (S.corridorSearches || 0) + 1;
    if (bestState < 0) S.failedSearches = (S.failedSearches || 0) + 1;
  }
  if (bestState < 0) return { pts: null };

  const chain = [];
  let cur = bestState;
  let guard = 0;
  while (cur >= 0 && guard++ < 50000000) {
    chain.push(cur);
    cur = store.parent(cur);
  }
  chain.reverse();
  const srcSide = initSide.get(chain[0]);
  if (!srcSide) return { pts: null };
  const pts = [borderPoint(src, srcSide)];
  for (const s of chain) {
    const v = (s - (s % 4)) / 4;
    const vi = v % nx;
    const vj = (v - vi) / nx;
    pts.push({ x: xs[vi], y: ys[vj] });
  }
  pts.push(borderPoint(tgt, bestPort.side));
  return { pts: simplifyPolyline(pts, 1e-6) };
}

// ───────────────────────── coarse corridor ─────────────────────────

/**
 * Coarse cell grid over the whole drawing, sized to the typical obstacle and capped at about
 * COARSE_TARGET_CELLS cells. Cell contents (the obstacles touching each cell, per margin) are
 * cached once and shared by every edge; which of them block depends on the edge's containers.
 */
function buildCoarse(obstacles, global, margin, index) {
  const dims = obstacles.map(o => Math.min(o.w, o.h)).sort((a, b) => a - b);
  const median = dims.length ? dims[Math.floor(dims.length / 2)] : 100;
  const W = global.r - global.l;
  const H = global.b - global.t;
  let cell = Math.min(600, Math.max(2 * margin + 16, median, Math.sqrt((W * H) / COARSE_TARGET_CELLS)));
  while (Math.ceil(W / cell) * Math.ceil(H / cell) > 4 * COARSE_TARGET_CELLS) cell *= 1.5;
  const C = { cell, l: global.l, t: global.t, cols: Math.max(1, Math.ceil(W / cell)), rows: Math.max(1, Math.ceil(H / cell)), caches: new Map() };
  /** Obstacles touching cell k (expanded by m), and those covering it fully or at its centre. */
  C.entry = (k, m) => {
    let cache = C.caches.get(m);
    if (!cache) {
      cache = new Map();
      C.caches.set(m, cache);
    }
    let e = cache.get(k);
    if (e) return e;
    const c = k % C.cols;
    const r = (k - c) / C.cols;
    const l = C.l + c * cell;
    const t = C.t + r * cell;
    const cx = l + cell / 2;
    const cy = t + cell / 2;
    e = { all: [], full: [], centre: [] };
    index.query(l - m, t - m, l + cell + m, t + cell + m, O => {
      e.all.push(O);
      const L = O.l - m;
      const T = O.t - m;
      const R = O.r + m;
      const B = O.b + m;
      if (L <= l && T <= t && R >= l + cell && B >= t + cell) e.full.push(O);
      else if (cx > L && cx < R && cy > T && cy < B) e.centre.push(O);
    });
    cache.set(k, e);
    return e;
  };
  return C;
}

function coarseCol(C, x) {
  return Math.min(C.cols - 1, Math.max(0, Math.floor((x - C.l) / C.cell)));
}

function coarseRow(C, y) {
  return Math.min(C.rows - 1, Math.max(0, Math.floor((y - C.t) / C.cell)));
}

/**
 * Global routing: A* over coarse cells from the source's cell to the target's. A cell fully
 * inside one inflated blocker is impassable; a cell whose centre is covered costs five times as
 * much. Returns the corridor (path cells dilated by `dilation`, plus the cells around both end
 * nodes) or null.
 */
function coarseCorridor(C, isBlocker, src, tgt, margin, dilation, work) {
  const cell = C.cell;
  const costMemo = new Map();
  const endCells = new Set();
  for (const n of [src, tgt]) {
    const c0 = coarseCol(C, n.l - margin - 1);
    const c1 = coarseCol(C, n.r + margin + 1);
    const r0 = coarseRow(C, n.t - margin - 1);
    const r1 = coarseRow(C, n.b + margin + 1);
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) endCells.add(r * C.cols + c);
  }
  const cellCost = (k) => {
    let v = costMemo.get(k);
    if (v !== undefined) return v;
    if (endCells.has(k)) {
      costMemo.set(k, 1);
      return 1;
    }
    const e = C.entry(k, margin);
    if (e.full.some(isBlocker)) v = Infinity;
    else v = e.centre.some(isBlocker) ? (globalThis.__ohCovered || 5) : 1;
    costMemo.set(k, v);
    return v;
  };

  const start = coarseRow(C, src.cy) * C.cols + coarseCol(C, src.cx);
  const goal = coarseRow(C, tgt.cy) * C.cols + coarseCol(C, tgt.cx);
  const gc = coarseCol(C, tgt.cx);
  const gr = coarseRow(C, tgt.cy);
  const gScore = new Map();
  const parent = new Map();
  const heap = new StateHeap();
  const out = { f: 0, g: 0, s: 0 };
  // States: cell * 5 + incoming direction (4 = none).
  const s0 = start * 5 + 4;
  gScore.set(s0, 0);
  parent.set(s0, -1);
  const h = (k) => {
    const c = k % C.cols;
    const r = (k - c) / C.cols;
    return (Math.abs(c - gc) + Math.abs(r - gr)) * cell;
  };
  heap.push(h(start), 0, s0);
  let found = -1;
  let expansions = 0;
  const cap = C.cols * C.rows * 5 + 16;
  while (heap.size) {
    heap.pop(out);
    const st = out.s;
    const gv = gScore.get(st);
    if (gv === undefined || out.g > gv + 1e-9) continue;
    const din = st % 5;
    const k = (st - din) / 5;
    if (k === goal) {
      found = st;
      break;
    }
    if (++expansions > cap) break;
    const c = k % C.cols;
    const r = (k - c) / C.cols;
    for (let d = 0; d < 4; d++) {
      if (din !== 4 && d === ((din + 2) & 3)) continue;
      const nc = c + DX[d];
      const nr = r + DY[d];
      if (nc < 0 || nr < 0 || nc >= C.cols || nr >= C.rows) continue;
      const nk = nr * C.cols + nc;
      const w = cellCost(nk);
      if (w === Infinity) continue;
      const ng = out.g + w * cell + (din !== 4 && d !== din ? cell * 0.5 : 0);
      const ns = nk * 5 + d;
      const old = gScore.get(ns);
      if (old !== undefined && old <= ng) continue;
      gScore.set(ns, ng);
      parent.set(ns, st);
      heap.push(ng + h(nk), ng, ns);
    }
  }
  // Coarse expansions are cheap; they are charged to the deterministic budget at a tenth.
  if (work) work.used += Math.ceil(expansions / 10);
  if (found < 0) return null;

  const cells = new Set(endCells);
  let cur = found;
  while (cur >= 0) {
    const k = (cur - (cur % 5)) / 5;
    const c = k % C.cols;
    const r = (k - c) / C.cols;
    for (let dr = -dilation; dr <= dilation; dr++) {
      for (let dc = -dilation; dc <= dilation; dc++) {
        const cc = c + dc;
        const rr = r + dr;
        if (cc >= 0 && rr >= 0 && cc < C.cols && rr < C.rows) cells.add(rr * C.cols + cc);
      }
    }
    cur = parent.get(cur);
  }
  let minC = Infinity;
  let minR = Infinity;
  let maxC = -Infinity;
  let maxR = -Infinity;
  cells.forEach(k => {
    const c = k % C.cols;
    const r = (k - c) / C.cols;
    minC = Math.min(minC, c);
    maxC = Math.max(maxC, c);
    minR = Math.min(minR, r);
    maxR = Math.max(maxR, r);
  });
  return {
    cells,
    win: { l: C.l + minC * cell, t: C.t + minR * cell, r: C.l + (maxC + 1) * cell, b: C.t + (maxR + 1) * cell },
    contains: (x, y) => {
      const c = Math.floor((x - C.l) / cell);
      const r = Math.floor((y - C.t) / cell);
      if (c < 0 || r < 0 || c >= C.cols || r >= C.rows) return false;
      if (cells.has(r * C.cols + c)) return true;
      // Points on a cell border belong to the corridor when the neighbouring cell does.
      const fx = (x - C.l) / cell - c;
      const fy = (y - C.t) / cell - r;
      if (fx < 1e-9 && c > 0 && cells.has(r * C.cols + c - 1)) return true;
      if (fy < 1e-9 && r > 0 && cells.has((r - 1) * C.cols + c)) return true;
      return false;
    }
  };
}

// ───────────────────────── nudging ─────────────────────────

function sign(v) {
  return v > 1e-6 ? 1 : (v < -1e-6 ? -1 : 0);
}

/** Live geometry of segment i of a route along one axis. */
function segGeom(routes, seg, horizontal) {
  const p = routes[seg.ri].pts;
  const a = p[seg.i];
  const b = p[seg.i + 1];
  return horizontal
    ? { c: a.y, lo: Math.min(a.x, b.x), hi: Math.max(a.x, b.x) }
    : { c: a.x, lo: Math.min(a.y, b.y), hi: Math.max(a.y, b.y) };
}

/**
 * Spreads collinear overlapping segments of different routes apart along one axis.
 * horizontal: segments of constant y are moved in y; otherwise segments of constant x in x.
 */
function nudgeAxis(routes, horizontal, index, spacing) {
  const all = [];
  routes.forEach((R, ri) => {
    const p = R.pts;
    for (let i = 0; i < p.length - 1; i++) {
      const a = p[i];
      const b = p[i + 1];
      const along = horizontal ? Math.abs(a.x - b.x) : Math.abs(a.y - b.y);
      const across = horizontal ? Math.abs(a.y - b.y) : Math.abs(a.x - b.x);
      if (across > 1e-6 || along <= 1e-6) continue;
      all.push({ ri, i, id: all.length });
    }
  });
  // Neighbour lookup by coordinate bucket (coordinates move by at most a few spacings).
  const bucketSize = Math.max(32, spacing * 8);
  const buckets = new Map();
  const bucketOf = (c) => Math.floor(c / bucketSize);
  const place = (seg) => {
    const g = segGeom(routes, seg, horizontal);
    seg.bucket = bucketOf(g.c);
    if (!buckets.has(seg.bucket)) buckets.set(seg.bucket, new Set());
    buckets.get(seg.bucket).add(seg);
  };
  all.forEach(place);
  const neighbours = (c, reach, fn) => {
    const b0 = bucketOf(c - reach);
    const b1 = bucketOf(c + reach);
    for (let b = b0; b <= b1; b++) {
      const set = buckets.get(b);
      if (set) set.forEach(fn);
    }
  };
  const move = (seg, v) => {
    const p = routes[seg.ri].pts;
    if (horizontal) {
      p[seg.i].y = v;
      p[seg.i + 1].y = v;
    } else {
      p[seg.i].x = v;
      p[seg.i + 1].x = v;
    }
    const nb = bucketOf(v);
    if (nb !== seg.bucket) {
      buckets.get(seg.bucket).delete(seg);
      seg.bucket = nb;
      if (!buckets.has(nb)) buckets.set(nb, new Set());
      buckets.get(nb).add(seg);
    }
  };

  // Clusters: same line, transitively overlapping ranges, from at least two routes. All clusters
  // are found first, so neighbouring lines can share the space between them fairly.
  const groups = new Map();
  all.forEach(seg => {
    const g = segGeom(routes, seg, horizontal);
    const key = Math.round(g.c * 1000);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(Object.assign(seg, { c0: g.c, lo: g.lo, hi: g.hi, clusterSize: 1 }));
  });
  const clusters = [];
  const keys = [...groups.keys()].sort((a, b) => a - b);
  for (const key of keys) {
    const segs = groups.get(key).sort((a, b) => a.lo - b.lo || a.ri - b.ri || a.i - b.i);
    let cluster = [];
    let maxHi = -Infinity;
    const flush = () => {
      if (cluster.length > 1 && new Set(cluster.map(s => s.ri)).size > 1) {
        cluster.forEach(s => { s.clusterSize = cluster.length; });
        clusters.push(cluster);
      }
      cluster = [];
      maxHi = -Infinity;
    };
    for (const s of segs) {
      if (cluster.length && s.lo >= maxHi - 0.5) flush();
      cluster.push(s);
      maxHi = Math.max(maxHi, s.hi);
    }
    flush();
  }
  for (const cluster of clusters) spreadCluster(cluster, routes, horizontal, index, spacing, neighbours, move);
}

function legInfo(seg, routes, horizontal) {
  const p = routes[seg.ri].pts;
  const a = p[seg.i];
  const b = p[seg.i + 1];
  const aIsLow = horizontal ? a.x <= b.x : a.y <= b.y;
  const lowIdx = aIsLow ? seg.i : seg.i + 1;
  const highIdx = aIsLow ? seg.i + 1 : seg.i;
  const dirAt = (endIdx) => {
    const nb = endIdx === seg.i ? seg.i - 1 : seg.i + 2;
    if (nb < 0 || nb >= p.length) return 0;
    return horizontal ? sign(p[nb].y - p[endIdx].y) : sign(p[nb].x - p[endIdx].x);
  };
  const along = (idx) => (horizontal ? p[idx].x : p[idx].y);
  return {
    lowPos: along(lowIdx),
    highPos: along(highIdx),
    lowDir: dirAt(lowIdx),
    highDir: dirAt(highIdx),
    lo: Math.min(along(lowIdx), along(highIdx)),
    hi: Math.max(along(lowIdx), along(highIdx))
  };
}

/**
 * Crossings created by placing A before B (A at the smaller coordinate): A's legs turning toward
 * B inside B's span, and B's legs turning toward A inside A's span.
 */
function crossingsIfFirst(A, B) {
  const inside = (pos, S) => pos > S.lo + 0.5 && pos < S.hi - 0.5;
  let c = 0;
  if (A.lowDir === 1 && inside(A.lowPos, B)) c++;
  if (A.highDir === 1 && inside(A.highPos, B)) c++;
  if (B.lowDir === -1 && inside(B.lowPos, A)) c++;
  if (B.highDir === -1 && inside(B.highPos, A)) c++;
  return c;
}

function spreadCluster(cluster, routes, horizontal, index, spacing, neighbours, move) {
  const members = cluster.map(seg => Object.assign({ seg }, legInfo(seg, routes, horizontal)));
  // Insertion sort with the crossing comparator (deterministic; the relation need not be transitive).
  const cmp = (A, B) => {
    const d = crossingsIfFirst(A, B) - crossingsIfFirst(B, A);
    if (d !== 0) return d;
    return A.seg.ri - B.seg.ri || A.seg.i - B.seg.i;
  };
  for (let i = 1; i < members.length; i++) {
    const m = members[i];
    let j = i - 1;
    while (j >= 0 && cmp(members[j], m) > 0) {
      members[j + 1] = members[j];
      j--;
    }
    members[j + 1] = m;
  }

  const k = members.length;
  const c = cluster[0].c0;
  const reach = spacing * k + 8;
  const inCluster = new Set(cluster);
  const neighbourSep = spacing / 2;

  for (const M of members) {
    const R = routes[M.seg.ri];
    const p = R.pts;
    // Hard limits (obstacles, legs, node sides) and soft limits (neighbouring parallel segments).
    let minC = c - reach;
    let maxC = c + reach;
    let softMin = -Infinity;
    let softMax = Infinity;
    let frozen = false;
    const lo = M.lo;
    const hi = M.hi;
    // Obstacles beside the segment (packages containing the route are passable).
    const qL = horizontal ? lo : c - reach;
    const qR = horizontal ? hi : c + reach;
    const qT = horizontal ? c - reach : lo;
    const qB = horizontal ? c + reach : hi;
    index.query(qL, qT, qR, qB, O => {
      if (R.containers.has(O.id)) return;
      const oLo = horizontal ? O.l : O.t;
      const oHi = horizontal ? O.r : O.b;
      if (!(oLo < hi - 0.5 && oHi > lo + 0.5)) return;
      const oNear = horizontal ? O.t : O.l;
      const oFar = horizontal ? O.b : O.r;
      if (oFar <= c + 1e-6) minC = Math.max(minC, oFar + NUDGE_CLEARANCE);
      else if (oNear >= c - 1e-6) maxC = Math.min(maxC, oNear - NUDGE_CLEARANCE);
      else frozen = true;
    });
    // Other parallel segments near the line keep their room: the space between two lines is
    // split in proportion to how many segments each must hold, and nothing may come closer
    // than half a spacing to a segment's current position.
    neighbours(c, reach, T => {
      if (inCluster.has(T)) return;
      const g = segGeom(routes, T, horizontal);
      if (!(g.lo < hi - 0.5 && g.hi > lo + 0.5)) return;
      const t0 = T.c0 !== undefined ? T.c0 : g.c;
      const share = k / (k + (T.clusterSize || 1));
      const split = c + (t0 - c) * share;
      if (t0 < c - 1e-6) softMin = Math.max(softMin, split + spacing / 4);
      else if (t0 > c + 1e-6) softMax = Math.min(softMax, split - spacing / 4);
      if (g.c < c - 1e-6) softMin = Math.max(softMin, g.c + neighbourSep);
      else if (g.c > c + 1e-6) softMax = Math.min(softMax, g.c - neighbourSep);
    });
    // Adjacent legs may shorten but never reverse.
    for (const nb of [M.seg.i - 1, M.seg.i + 2]) {
      if (nb < 0 || nb >= p.length) continue;
      const v = horizontal ? p[nb].y : p[nb].x;
      if (v < c - 1e-6) minC = Math.max(minC, v + 1);
      else if (v > c + 1e-6) maxC = Math.min(maxC, v - 1);
      else frozen = true;
    }
    // End stubs stay on their node side, clear of its corners.
    const constrainToSide = (node) => {
      const sLo = (horizontal ? node.t : node.l) + STUB_CORNER_INSET;
      const sHi = (horizontal ? node.b : node.r) - STUB_CORNER_INSET;
      if (sHi < sLo) frozen = true;
      minC = Math.max(minC, sLo);
      maxC = Math.min(maxC, sHi);
    };
    if (M.seg.i === 0) constrainToSide(R.src);
    if (M.seg.i + 1 === p.length - 1) constrainToSide(R.tgt);
    // The hard interval always contains the current line (it is valid); a member may leave its
    // line for the soft interval when the two intersect, otherwise only the hard limits apply.
    if (frozen || minC > c + 1e-6 || maxC < c - 1e-6 || minC > maxC) {
      M.minC = c;
      M.maxC = c;
    } else {
      const sMin = Math.max(minC, softMin);
      const sMax = Math.min(maxC, softMax);
      M.minC = sMin <= sMax ? sMin : minC;
      M.maxC = sMin <= sMax ? sMax : maxC;
    }
  }

  // Place members in order, sep apart, as close as possible to the evenly spread targets:
  // with q_m = p_m - m * sep the targets are all equal and the order constraints become q
  // non-decreasing, so the least-squares optimum is the common target clamped to the running
  // maximum of the lower bounds and the running minimum (from the right) of the upper bounds.
  // The separation shrinks when the free space is too narrow.
  const placeWith = (list, sep) => {
    const n = list.length;
    const target = c - ((n - 1) / 2) * sep;
    const L = new Array(n);
    const U = new Array(n);
    let run = -Infinity;
    for (let m = 0; m < n; m++) {
      run = Math.max(run, list[m].minC - m * sep);
      L[m] = run;
    }
    run = Infinity;
    for (let m = n - 1; m >= 0; m--) {
      run = Math.min(run, list[m].maxC - m * sep);
      U[m] = run;
    }
    const pos = new Array(n);
    for (let m = 0; m < n; m++) {
      if (L[m] > U[m] + 1e-9) return null;
      pos[m] = Math.min(U[m], Math.max(L[m], target)) + m * sep;
    }
    return pos;
  };
  const seps = [spacing, spacing / 2, spacing / 4, Math.min(1, spacing / 4)];
  for (const sep of seps) {
    const pos = placeWith(members, sep);
    if (pos) {
      members.forEach((M, m) => move(M.seg, pos[m]));
      return;
    }
  }
  // Very crowded line: the largest separation that still fits, down to a quarter pixel.
  {
    let lo = 0;
    let hi = seps[seps.length - 1];
    let bestPos = null;
    for (let iter = 0; iter < 14; iter++) {
      const mid = (lo + hi) / 2;
      const pos = placeWith(members, mid);
      if (pos) {
        lo = mid;
        bestPos = pos;
      } else {
        hi = mid;
      }
    }
    if (bestPos && lo >= 0.25) {
      members.forEach((M, m) => move(M.seg, bestPos[m]));
      return;
    }
  }
  // Members pinned to the line (by a node corner or a zero-length leg) cannot all move: spread
  // the free ones on the side away from the pinned ones.
  const free = members.filter(M => M.maxC - M.minC > 1e-9);
  if (free.length < 2) return;
  for (const sep of seps) {
    const pos = placeWith(free, sep);
    if (pos) {
      free.forEach((M, m) => move(M.seg, pos[m]));
      return;
    }
  }
}

// ───────────────────────── simple fallback ─────────────────────────

/**
 * The cheapest valid straight, L or Z route between a port of each end node, or null. Used for
 * edges left after the work budget; costs O(candidates x nearby obstacles) and no search.
 */
function simpleRoute(src, tgt, containers, srcCosts, tgtCosts, margin, bendPenalty, index) {
  const probe = { src, tgt, containers, pts: null };
  let best = null;
  let bestCost = Infinity;
  for (const ss of SIDES) {
    const sc = sideCost(srcCosts, ss);
    if (sc === Infinity) continue;
    for (const ts of SIDES) {
      const tc = sideCost(tgtCosts, ts);
      if (tc === Infinity) continue;
      const bs = borderPoint(src, ss);
      const ps = portPoint(src, ss, margin);
      const pt = portPoint(tgt, ts, margin);
      const bt = borderPoint(tgt, ts);
      const mx = (ps.x + pt.x) / 2;
      const my = (ps.y + pt.y) / 2;
      const shapes = [
        [bs, ps, pt, bt],
        [bs, ps, { x: pt.x, y: ps.y }, pt, bt],
        [bs, ps, { x: ps.x, y: pt.y }, pt, bt],
        [bs, ps, { x: mx, y: ps.y }, { x: mx, y: pt.y }, pt, bt],
        [bs, ps, { x: ps.x, y: my }, { x: pt.x, y: my }, pt, bt]
      ];
      for (const raw of shapes) {
        let axisParallel = true;
        for (let i = 1; i < raw.length; i++) {
          if (Math.abs(raw[i].x - raw[i - 1].x) > 1e-6 && Math.abs(raw[i].y - raw[i - 1].y) > 1e-6) {
            axisParallel = false;
            break;
          }
        }
        if (!axisParallel) continue;
        const pts = simplifyPolyline(raw, 1e-6);
        // Reject paths that double back on themselves.
        let backtrack = false;
        for (let i = 2; i < pts.length; i++) {
          const ax = pts[i - 1].x - pts[i - 2].x;
          const ay = pts[i - 1].y - pts[i - 2].y;
          const bx = pts[i].x - pts[i - 1].x;
          const by = pts[i].y - pts[i - 1].y;
          if (ax * bx + ay * by < -1e-9) {
            backtrack = true;
            break;
          }
        }
        if (backtrack) continue;
        let len = 0;
        for (let i = 1; i < pts.length; i++) len += Math.abs(pts[i].x - pts[i - 1].x) + Math.abs(pts[i].y - pts[i - 1].y);
        const cost = len + bendPenalty * Math.max(0, pts.length - 2) + sc + tc;
        if (cost >= bestCost) continue;
        probe.pts = pts;
        if (!routeValid(probe, index)) continue;
        best = pts;
        bestCost = cost;
      }
    }
  }
  return best;
}

// ───────────────────────── validation ─────────────────────────

function routeValid(R, index) {
  const p = R.pts;
  if (!p || p.length < 2) return false;
  const onBorder = (pt, n) => {
    const dx = Math.max(n.l - pt.x, 0, pt.x - n.r);
    const dy = Math.max(n.t - pt.y, 0, pt.y - n.b);
    if (dx > 0.5 || dy > 0.5) return false;
    return !(pt.x > n.l + 0.5 && pt.x < n.r - 0.5 && pt.y > n.t + 0.5 && pt.y < n.b - 0.5);
  };
  if (!onBorder(p[0], R.src) || !onBorder(p[p.length - 1], R.tgt)) return false;
  for (let i = 1; i < p.length; i++) {
    const a = p[i - 1];
    const b = p[i];
    if (Math.abs(a.x - b.x) > 1e-6 && Math.abs(a.y - b.y) > 1e-6) return false;
    // The end nodes count too: a route touches them only at its end points on their borders.
    let bad = false;
    index.query(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.max(a.x, b.x), Math.max(a.y, b.y), O => {
      if (bad || R.containers.has(O.id)) return;
      if (segmentIntersectsRectInterior(a, b, realRect(O), 0.5)) bad = true;
    });
    if (bad) return false;
  }
  return true;
}

// ───────────────────────── entry point ─────────────────────────

/**
 * Routes edges orthogonally around obstacles.
 * @param {{ obstacles: Array<{ id: string, cx: number, cy: number, w: number, h: number, ancestors?: string[] }>,
 *   edges: Array<{ id: string, source: string, target: string,
 *     sourceSides?: { top?: number, bottom?: number, left?: number, right?: number },
 *     targetSides?: { top?: number, bottom?: number, left?: number, right?: number } }> }} input
 *   obstacles: every leaf card and every package footprint; ancestors lists the compounds that
 *   contain it, innermost first. Side costs bias (a number) or forbid (Infinity) leaving or
 *   entering through a node side.
 * @param {{ margin?: number, bendPenalty?: number, spacing?: number, window?: number, timeBudgetMs?: number, workBudget?: number }} [options]
 *   margin: clearance kept from obstacles (16); bendPenalty: cost of one bend in px (40); spacing:
 *   distance between parallel segments after nudging (10); window: initial search padding around
 *   the two end nodes (600). timeBudgetMs (2500) sets a deterministic work budget of
 *   max(4,000,000, timeBudgetMs x 2,000) search expansions shared by all edges (or pass
 *   workBudget directly); edges past it get the best valid straight, L or Z route, then a
 *   coarse-grid corridor route with a fixed allowance. The same input always yields the same
 *   output; only an emergency wall-clock stop at max(30 s, 10 x timeBudgetMs) can cut it short.
 * @returns {Map<string, { points: Array<{ x: number, y: number }> }>} per routed edge, an
 *   axis-parallel polyline from the source border to the target border that enters no obstacle
 *   other than the two end nodes and the packages containing them. Self loops, edges with a
 *   missing end, edges where one end contains the other, and edges that could not be routed are
 *   omitted.
 */
export function routeOrthogonal(input, options = {}) {
  const opts = options || {};
  const margin = Number.isFinite(opts.margin) && opts.margin >= 0 ? opts.margin : 16;
  const bendPenalty = Number.isFinite(opts.bendPenalty) && opts.bendPenalty >= 0 ? opts.bendPenalty : 40;
  const spacing = Number.isFinite(opts.spacing) && opts.spacing > 0 ? opts.spacing : 10;
  const windowPad = Number.isFinite(opts.window) && opts.window > 0 ? opts.window : 600;
  const budgetMs = Number.isFinite(opts.timeBudgetMs) && opts.timeBudgetMs > 0 ? opts.timeBudgetMs : 2500;
  // Deterministic work budget (search expansions shared by all edges). The wall clock is only an
  // emergency stop far above normal cost; within it, the same input always gives the same output.
  const work = {
    used: 0,
    limit: Number.isFinite(opts.workBudget) && opts.workBudget > 0 ? opts.workBudget : Math.max(4000000, Math.round(budgetMs * WORK_PER_MS)),
    // Optional counters for diagnostics (options.stats, an object the caller owns).
    stats: opts.stats && typeof opts.stats === 'object' ? opts.stats : null
  };
  const emergencyMs = Math.max(30000, 10 * budgetMs);
  const started = now();
  const result = new Map();

  const obstacleList = [];
  const byId = new Map();
  for (const o of (input && input.obstacles) || []) {
    if (!o || o.id === undefined || o.id === null || !finiteRect(o)) continue;
    const id = String(o.id);
    if (byId.has(id)) continue;
    const ancList = Array.isArray(o.ancestors) ? o.ancestors.map(String) : [];
    const rec = {
      id,
      cx: o.cx,
      cy: o.cy,
      w: o.w,
      h: o.h,
      l: o.cx - o.w / 2,
      t: o.cy - o.h / 2,
      r: o.cx + o.w / 2,
      b: o.cy + o.h / 2,
      ancList,
      ancSet: new Set(ancList),
      order: obstacleList.length
    };
    byId.set(id, rec);
    obstacleList.push(rec);
  }
  if (!obstacleList.length) return result;
  const index = new GridIndex(obstacleList);
  let gl = Infinity;
  let gt = Infinity;
  let gr = -Infinity;
  let gb = -Infinity;
  for (const o of obstacleList) {
    gl = Math.min(gl, o.l);
    gt = Math.min(gt, o.t);
    gr = Math.max(gr, o.r);
    gb = Math.max(gb, o.b);
  }
  const outer = 2 * margin + 24;
  const global = { l: gl - outer, t: gt - outer, r: gr + outer, b: gb + outer };
  const coarse = buildCoarse(obstacleList, global, margin, index);

  const routeOne = (src, tgt, srcCosts, tgtCosts) => {
    const containers = new Set([...src.ancList, ...tgt.ancList]);
    const isBlocker = (O) => {
      if (containers.has(O.id)) return false;
      for (const a of O.ancList) if (!containers.has(a)) return false;
      return true;
    };
    const collect = (l, t, r, b) => {
      const list = [];
      index.query(l, t, r, b, O => {
        if (isBlocker(O)) list.push(O);
      });
      return list;
    };
    const bl = Math.min(src.l, tgt.l);
    const bt = Math.min(src.t, tgt.t);
    const br = Math.max(src.r, tgt.r);
    const bb = Math.max(src.b, tgt.b);
    const windowAt = (pad) => ({
      l: Math.max(global.l, bl - pad),
      t: Math.max(global.t, bt - pad),
      r: Math.min(global.r, br + pad),
      b: Math.min(global.b, bb + pad)
    });
    const covers = (w) => w.l <= global.l && w.t <= global.t && w.r >= global.r && w.b >= global.b;

    const corridorSearch = (m, dilations, budget) => {
      for (const dilation of dilations) {
        const corridor = coarseCorridor(coarse, isBlocker, src, tgt, m, dilation, budget);
        if (!corridor) return null;
        // Blockers: every obstacle touching a corridor cell (from the shared cell cache).
        const blockers = [];
        const seen = new Set();
        corridor.cells.forEach(k => {
          for (const O of coarse.entry(k, m).all) {
            if (!seen.has(O)) {
              seen.add(O);
              if (isBlocker(O)) blockers.push(O);
            }
          }
        });
        blockers.sort((a, b) => a.order - b.order);
        const found = searchRegion(corridor.win, blockers, src, tgt, srcCosts, tgtCosts, m, bendPenalty, corridor, budget);
        if (found.pts) return found.pts;
      }
      return null;
    };

    // Full search while the shared work budget lasts. Long edges search a coarse corridor first
    // (global routing), so the detailed search only visits vertices along the way.
    const minMargin = Math.min(4, margin / 4);
    const span = Math.abs(src.cx - tgt.cx) + Math.abs(src.cy - tgt.cy);
    const long = span > Math.max(LONG_EDGE_PX, LONG_EDGE_CELLS * coarse.cell);
    if (work.used < work.limit) {
      for (const m of [margin, margin / 2, minMargin]) {
        if (m !== margin && !(m > 0 && m < margin)) continue;
        if (long) {
          const pts = corridorSearch(m, CORRIDOR_DILATIONS, work);
          if (pts) return pts;
          if (work.used >= work.limit) break;
        }
        let pad = windowPad;
        let useCorridor = false;
        for (let tries = 0; tries < 8 && work.used < work.limit; tries++) {
          const win = windowAt(pad);
          const blockers = collect(win.l - m, win.t - m, win.r + m, win.b + m);
          if (blockers.length > WINDOW_BLOCKER_LIMIT) {
            useCorridor = true;
            break;
          }
          const found = searchRegion(win, blockers, src, tgt, srcCosts, tgtCosts, m, bendPenalty, null, work);
          if (found.pts) return found.pts;
          if (found.tooBig) {
            useCorridor = true;
            break;
          }
          if (covers(win)) break;
          pad *= 2;
        }
        if (useCorridor && !long && work.used < work.limit) {
          const pts = corridorSearch(m, CORRIDOR_DILATIONS, work);
          if (pts) return pts;
        }
      }
    }
    // Cheap fallback: the best valid straight, L or Z route between two ports.
    const simple = simpleRoute(src, tgt, containers, srcCosts, tgtCosts, margin, bendPenalty, index);
    if (work.stats) work.stats.simpleFallbacks = (work.stats.simpleFallbacks || 0) + (simple ? 1 : 0);
    if (simple) return simple;
    if (work.stats) work.stats.corridorFallbacks = (work.stats.corridorFallbacks || 0) + 1;
    // Coarse global grid with a wide corridor and a fixed allowance of its own.
    return corridorSearch(minMargin > 0 ? minMargin : margin, [FALLBACK_DILATION], { used: 0, limit: FALLBACK_EXPANSIONS, stats: work.stats });
  };

  const routes = [];
  const parallel = new Map();
  const seenEdges = new Set();
  for (const e of (input && input.edges) || []) {
    if (!e || e.id === undefined || e.id === null) continue;
    const id = String(e.id);
    if (seenEdges.has(id)) continue;
    seenEdges.add(id);
    const src = byId.get(String(e.source));
    const tgt = byId.get(String(e.target));
    if (!src || !tgt || src === tgt) continue;
    if (src.ancSet.has(tgt.id) || tgt.ancSet.has(src.id)) continue;
    // Packages containing either end: the route may (and must) pass through them.
    const containers = new Set([...src.ancList, ...tgt.ancList]);
    const key = src.id < tgt.id ? `${src.id}\u0000${tgt.id}` : `${tgt.id}\u0000${src.id}`;
    const shared = parallel.get(key);
    if (shared) {
      const pts = shared.src === src ? shared.raw : shared.raw.slice().reverse();
      routes.push({ id, src, tgt, containers, raw: pts.map(p => ({ x: p.x, y: p.y })), pts: pts.map(p => ({ x: p.x, y: p.y })) });
      continue;
    }
    if (now() - started > emergencyMs) continue;
    const pts = routeOne(src, tgt, e.sourceSides, e.targetSides);
    if (!pts || pts.length < 2) continue;
    const rec = { id, src, tgt, containers, raw: pts.map(p => ({ x: p.x, y: p.y })), pts: pts.map(p => ({ x: p.x, y: p.y })) };
    routes.push(rec);
    parallel.set(key, rec);
  }

  if (routes.length > 1) {
    nudgeAxis(routes, true, index, spacing);
    nudgeAxis(routes, false, index, spacing);
  }

  for (const R of routes) {
    R.pts = simplifyPolyline(R.pts, 1e-6);
    if (!routeValid(R, index)) {
      R.pts = simplifyPolyline(R.raw, 1e-6);
      if (!routeValid(R, index)) continue;
    }
    result.set(R.id, { points: R.pts.map(p => ({ x: p.x, y: p.y })) });
  }
  return result;
}
