/**
 * OpenHeart layout: packing of independent boxes (isolated nodes, connected components,
 * detached sub-drawings) into a tidy arrangement near a target aspect ratio.
 *
 * Two strategies, chosen by how uniform the box sizes are:
 *  - Flexible grid (near-uniform sizes): every column count is tried; a column takes the width
 *    of its widest box and a row the height of its tallest, which keeps cards aligned on both
 *    axes and in caller order (left to right, then top to bottom).
 *  - Skyline packing (mixed sizes): the skyline bottom-left heuristic as described by
 *    J. Jylanki, "A Thousand Ways to Pack the Bin - A Practical Approach to Two-Dimensional
 *    Rectangle Bin Packing" (2010), after B. Chazelle, "The Bottom-Left Bin-Packing Heuristic:
 *    An Efficient Implementation", IEEE Transactions on Computers C-32(8), 1983. In caller
 *    order, each box goes to the skyline position that keeps it highest (smallest y), left-most
 *    on ties; several strip widths are tried.
 * Both are scored by cost = (1 - box area / (W * H)) + 0.6 * |ln((W / H) / aspectRatio)|, ties
 * broken by fewer rows, then smaller width.
 */
import { SPACING } from './geometry.js';

const UNIFORM_RATIO = 1.6;
const STRIP_FACTORS = [0.7, 0.85, 1, 1.15, 1.3, 1.6];
const EPS = 1e-9;

function finiteNonNegative(value, fallback) {
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

function cost(boxArea, W, H, aspectRatio) {
  if (!(W > 0) || !(H > 0)) return Infinity;
  return (1 - boxArea / (W * H)) + 0.6 * Math.abs(Math.log((W / H) / aspectRatio));
}

function better(a, b) {
  if (a.cost < b.cost - 1e-12) return true;
  if (a.cost > b.cost + 1e-12) return false;
  if (a.rows !== b.rows) return a.rows < b.rows;
  return a.width < b.width - EPS;
}

function flexibleGrid(items, gapX, gapY, aspectRatio) {
  const n = items.length;
  let boxArea = 0;
  for (const it of items) boxArea += it.w * it.h;
  const colW = new Float64Array(n);
  const rowH = new Float64Array(n);
  let best = null;
  for (let c = 1; c <= n; c++) {
    const rows = Math.ceil(n / c);
    colW.fill(0, 0, c);
    rowH.fill(0, 0, rows);
    for (let i = 0; i < n; i++) {
      const col = i % c;
      const row = (i / c) | 0;
      if (items[i].w > colW[col]) colW[col] = items[i].w;
      if (items[i].h > rowH[row]) rowH[row] = items[i].h;
    }
    let W = (c - 1) * gapX;
    for (let k = 0; k < c; k++) W += colW[k];
    let H = (rows - 1) * gapY;
    for (let k = 0; k < rows; k++) H += rowH[k];
    const cand = { cost: cost(boxArea, W, H, aspectRatio), rows, width: W, height: H, cols: c };
    if (!best || better(cand, best)) best = cand;
  }

  const c = best.cols;
  const rows = Math.ceil(n / c);
  colW.fill(0, 0, c);
  rowH.fill(0, 0, rows);
  for (let i = 0; i < n; i++) {
    const col = i % c;
    const row = (i / c) | 0;
    if (items[i].w > colW[col]) colW[col] = items[i].w;
    if (items[i].h > rowH[row]) rowH[row] = items[i].h;
  }
  const colX = new Float64Array(c);
  for (let k = 1; k < c; k++) colX[k] = colX[k - 1] + colW[k - 1] + gapX;
  const rowY = new Float64Array(rows);
  for (let k = 1; k < rows; k++) rowY[k] = rowY[k - 1] + rowH[k - 1] + gapY;
  const xs = new Float64Array(n);
  const ys = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const col = i % c;
    const row = (i / c) | 0;
    xs[i] = colX[col] + (colW[col] - items[i].w) / 2;
    ys[i] = rowY[row];
  }
  return { xs, ys, width: best.width, height: best.height };
}

/** Replaces the skyline under [x, x + w) with one segment at height y; merges equal neighbours. */
function placeSegment(segs, x, w, y) {
  const end = x + w;
  const out = [];
  let inserted = false;
  for (const s of segs) {
    const sEnd = s.x + s.w;
    if (sEnd <= x + EPS) {
      out.push(s);
      continue;
    }
    if (s.x >= end - EPS) {
      if (!inserted) {
        out.push({ x, w, y });
        inserted = true;
      }
      out.push(s);
      continue;
    }
    if (s.x < x - EPS) out.push({ x: s.x, w: x - s.x, y: s.y });
    if (!inserted) {
      out.push({ x, w, y });
      inserted = true;
    }
    if (sEnd > end + EPS) out.push({ x: end, w: sEnd - end, y: s.y });
  }
  if (!inserted) out.push({ x, w, y });
  const merged = [];
  for (const s of out) {
    const last = merged[merged.length - 1];
    if (last && Math.abs(last.y - s.y) <= EPS && Math.abs(last.x + last.w - s.x) <= 1e-6) last.w += s.w;
    else merged.push({ x: s.x, w: s.w, y: s.y });
  }
  return merged;
}

/** One skyline run for a strip of usable width stripW (boxes are inflated by the gaps). */
function skylineRun(items, stripW, gapX, gapY) {
  const limit = stripW + gapX;
  let segs = [{ x: 0, w: limit, y: 0 }];
  const n = items.length;
  const xs = new Float64Array(n);
  const ys = new Float64Array(n);
  let width = 0;
  let height = 0;
  const rowTops = new Set();
  for (let i = 0; i < n; i++) {
    const bw = Math.min(items[i].w + gapX, limit);
    const bh = items[i].h + gapY;
    let bestX = 0;
    let bestY = Infinity;
    const consider = (x, y) => {
      if (y < bestY - EPS || (Math.abs(y - bestY) <= EPS && x < bestX - EPS)) {
        bestX = x;
        bestY = y;
      }
    };
    for (let s = 0; s < segs.length; s++) {
      // Left-aligned with this segment's start.
      const x = segs[s].x;
      if (x + bw <= limit + EPS) {
        let y = 0;
        for (let t = s; t < segs.length && segs[t].x < x + bw - EPS; t++) if (segs[t].y > y) y = segs[t].y;
        consider(x, y);
      }
      // Right-aligned with this segment's end (reaches back over earlier segments).
      const xr = segs[s].x + segs[s].w - bw;
      if (xr >= -EPS && Math.abs(xr - x) > EPS) {
        let y = 0;
        for (let t = s; t >= 0 && segs[t].x + segs[t].w > xr + EPS; t--) if (segs[t].y > y) y = segs[t].y;
        consider(Math.max(0, xr), y);
      }
    }
    if (bestY === Infinity) {
      bestX = 0;
      bestY = segs.reduce((m, s) => Math.max(m, s.y), 0);
    }
    xs[i] = bestX;
    ys[i] = bestY;
    width = Math.max(width, bestX + items[i].w);
    height = Math.max(height, bestY + items[i].h);
    rowTops.add(Math.round(bestY * 1000));
    segs = placeSegment(segs, bestX, bw, bestY + bh);
  }
  return { xs, ys, width, height, rows: rowTops.size };
}

function skyline(items, gapX, gapY, aspectRatio) {
  let inflatedArea = 0;
  let boxArea = 0;
  let maxW = 0;
  for (const it of items) {
    inflatedArea += (it.w + gapX) * (it.h + gapY);
    boxArea += it.w * it.h;
    if (it.w > maxW) maxW = it.w;
  }
  const base = Math.sqrt(inflatedArea * aspectRatio);
  const widths = [];
  for (const f of STRIP_FACTORS) {
    const sw = Math.max(maxW, base * f - gapX);
    if (!widths.some(w => Math.abs(w - sw) < 0.5)) widths.push(sw);
  }
  let best = null;
  for (const sw of widths) {
    const run = skylineRun(items, sw, gapX, gapY);
    const cand = { cost: cost(boxArea, run.width, run.height, aspectRatio), rows: run.rows, width: run.width, height: run.height, run };
    if (!best || better(cand, best)) best = cand;
  }
  return { xs: best.run.xs, ys: best.run.ys, width: best.width, height: best.height };
}

/**
 * Packs boxes into a tidy arrangement close to a target aspect ratio.
 * Near-uniform sizes (max/min width and height ratios both <= 1.6) use a flexible grid;
 * otherwise skyline bottom-left packing over several strip widths. Caller order is kept as the
 * placement order.
 * @param {Array<{ id: string, w: number, h: number }>} boxes in caller order
 * @param {{ gapX?: number, gapY?: number, aspectRatio?: number }} [options]
 * @returns {{ positions: Map<string, { cx: number, cy: number }>, width: number, height: number }}
 *   box centres, with the arrangement's bounding box starting at (0, 0)
 */
export function packBoxes(boxes, options = {}) {
  const opts = options || {};
  const gapX = finiteNonNegative(opts.gapX, SPACING.packGap);
  const gapY = finiteNonNegative(opts.gapY, SPACING.packGap);
  const aspectRatio = Number.isFinite(opts.aspectRatio) && opts.aspectRatio > 0 ? opts.aspectRatio : 1.6;
  const items = [];
  if (Array.isArray(boxes)) {
    for (const b of boxes) {
      if (!b) continue;
      items.push({
        id: b.id,
        w: Number.isFinite(b.w) && b.w > 0 ? b.w : 1,
        h: Number.isFinite(b.h) && b.h > 0 ? b.h : 1
      });
    }
  }
  const positions = new Map();
  if (items.length === 0) return { positions, width: 0, height: 0 };

  let minW = Infinity;
  let maxW = 0;
  let minH = Infinity;
  let maxH = 0;
  for (const it of items) {
    if (it.w < minW) minW = it.w;
    if (it.w > maxW) maxW = it.w;
    if (it.h < minH) minH = it.h;
    if (it.h > maxH) maxH = it.h;
  }
  const uniform = maxW / minW <= UNIFORM_RATIO && maxH / minH <= UNIFORM_RATIO;
  const layout = uniform
    ? flexibleGrid(items, gapX, gapY, aspectRatio)
    : skyline(items, gapX, gapY, aspectRatio);

  for (let i = 0; i < items.length; i++) {
    positions.set(items[i].id, { cx: layout.xs[i] + items[i].w / 2, cy: layout.ys[i] + items[i].h / 2 });
  }
  return { positions, width: layout.width, height: layout.height };
}
