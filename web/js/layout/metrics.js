/**
 * OpenHeart layout quality metrics.
 *
 * Measures a finished drawing against the standard aesthetic criteria of graph drawing:
 * H. C. Purchase, "Metrics for Graph Drawing Aesthetics", Journal of Visual Languages and
 * Computing 13(5), 2002 (edge crossings, bends, edge length, area and aspect ratio), plus the
 * two hard constraints the engine must never break: node overlap and edges drawn through nodes.
 * Pair tests use uniform-grid spatial hashing so they stay near-linear on large drawings.
 *
 * scoreLayout turns a metrics object into one number (lower is better) used to pick between
 * candidate layouts; hard-constraint violations dominate every soft criterion.
 */
import {
  rectsOverlap,
  boundsOf,
  clipToRectBorder,
  segmentIntersectsRectInterior,
  segmentsProperlyIntersect,
  polylineLength
} from './geometry.js';

const MAX_GRID_DIM = 256;
const MIN_CELL = 16;
const MAX_ANCESTOR_STEPS = 256;

function makeGrid(bounds, count) {
  const W = Math.max(1, bounds.maxX - bounds.minX);
  const H = Math.max(1, bounds.maxY - bounds.minY);
  let cell = Math.sqrt((W * H) / Math.max(1, count));
  cell = Math.max(cell, MIN_CELL, W / MAX_GRID_DIM, H / MAX_GRID_DIM);
  const cols = Math.max(1, Math.min(MAX_GRID_DIM, Math.ceil(W / cell) + 1));
  const rows = Math.max(1, Math.min(MAX_GRID_DIM, Math.ceil(H / cell) + 1));
  return { minX: bounds.minX, minY: bounds.minY, cell, cols, rows, cells: new Array(cols * rows) };
}

function cellX(g, x) {
  const c = Math.floor((x - g.minX) / g.cell);
  return c < 0 ? 0 : c >= g.cols ? g.cols - 1 : c;
}

function cellY(g, y) {
  const c = Math.floor((y - g.minY) / g.cell);
  return c < 0 ? 0 : c >= g.rows ? g.rows - 1 : c;
}

function gridInsert(g, x0, y0, x1, y1, value) {
  const gx0 = cellX(g, Math.min(x0, x1));
  const gx1 = cellX(g, Math.max(x0, x1));
  const gy0 = cellY(g, Math.min(y0, y1));
  const gy1 = cellY(g, Math.max(y0, y1));
  for (let gy = gy0; gy <= gy1; gy++) {
    for (let gx = gx0; gx <= gx1; gx++) {
      const k = gy * g.cols + gx;
      if (g.cells[k]) g.cells[k].push(value);
      else g.cells[k] = [value];
    }
  }
  return { gx0, gy0, gx1, gy1 };
}

const finiteOr = (v, fallback) => (Number.isFinite(v) ? v : fallback);

/**
 * Measures a drawing.
 * @param {{ nodes: Array<{ id: string, cx: number, cy: number, w: number, h: number, parent: string|null, isCompound: boolean }>,
 *           edges: Array<{ id: string, source: string, target: string, points?: Array<{ x: number, y: number }> }> }} drawing
 *   points is the full absolute polyline including both attachment points; when absent the
 *   straight segment between the two rectangle borders is measured.
 * @param {{ viewportAspect?: number }} [options]
 * @returns {{ nodes: number, edges: number, nodeOverlaps: number, edgeNodeCrossings: number, edgeCrossings: number,
 *   bends: number, totalEdgeLength: number, meanEdgeLength: number, normalizedEdgeLength: number, maxEdgeLength: number,
 *   width: number, height: number, area: number, areaEfficiency: number, aspectRatio: number, aspectDeviation: number }}
 */
export function measureLayout(drawing, options = {}) {
  const opts = options || {};
  const viewportAspect = Number.isFinite(opts.viewportAspect) && opts.viewportAspect > 0 ? opts.viewportAspect : 1.6;

  // ── Nodes ──
  const rawNodes = drawing && Array.isArray(drawing.nodes) ? drawing.nodes : [];
  const nodes = [];
  const indexById = new Map();
  for (const n of rawNodes) {
    if (!n || n.id === undefined || n.id === null) continue;
    if (!Number.isFinite(n.cx) || !Number.isFinite(n.cy)) continue;
    const id = String(n.id);
    if (indexById.has(id)) continue;
    indexById.set(id, nodes.length);
    nodes.push({
      id,
      cx: n.cx,
      cy: n.cy,
      w: Number.isFinite(n.w) && n.w > 0 ? n.w : 0,
      h: Number.isFinite(n.h) && n.h > 0 ? n.h : 0,
      parent: n.parent === undefined || n.parent === null ? null : String(n.parent),
      isCompound: Boolean(n.isCompound)
    });
  }
  const N = nodes.length;
  const parentIdx = new Int32Array(N).fill(-1);
  for (let i = 0; i < N; i++) {
    const p = nodes[i].parent;
    if (p === null) continue;
    const j = indexById.get(p);
    if (j !== undefined && j !== i) parentIdx[i] = j;
  }
  const isAncestor = (a, b) => {
    let cur = parentIdx[b];
    let steps = 0;
    while (cur !== -1 && steps++ < MAX_ANCESTOR_STEPS) {
      if (cur === a) return true;
      cur = parentIdx[cur];
    }
    return false;
  };

  // ── Node overlaps (sweep over left edges) ──
  let nodeOverlaps = 0;
  {
    const left = (i) => nodes[i].cx - nodes[i].w / 2;
    const right = (i) => nodes[i].cx + nodes[i].w / 2;
    const order = Array.from({ length: N }, (_, i) => i).sort((a, b) => (left(a) - left(b)) || (a - b));
    let active = [];
    for (const i of order) {
      const li = left(i);
      if (active.length) active = active.filter(j => right(j) > li);
      for (const j of active) {
        if (rectsOverlap(nodes[i], nodes[j], -1) && !isAncestor(i, j) && !isAncestor(j, i)) nodeOverlaps++;
      }
      active.push(i);
    }
  }

  // ── Edge polylines ──
  const rawEdges = drawing && Array.isArray(drawing.edges) ? drawing.edges : [];
  const edges = [];
  for (const e of rawEdges) {
    if (!e) continue;
    const si = indexById.get(String(e.source));
    const ti = indexById.get(String(e.target));
    if (si === undefined || ti === undefined) continue;
    let pts = null;
    if (Array.isArray(e.points) && e.points.length >= 2 &&
        e.points.every(p => p && Number.isFinite(p.x) && Number.isFinite(p.y))) {
      pts = e.points.map(p => ({ x: p.x, y: p.y }));
    } else {
      if (si === ti) continue; // a self-loop without a route has nothing measurable
      const s = nodes[si];
      const t = nodes[ti];
      pts = [clipToRectBorder(s, { x: t.cx, y: t.cy }), clipToRectBorder(t, { x: s.cx, y: s.cy })];
    }
    edges.push({ si, ti, pts });
  }

  let bends = 0;
  let totalEdgeLength = 0;
  let maxEdgeLength = 0;
  for (const e of edges) {
    bends += Math.max(0, e.pts.length - 2);
    const len = polylineLength(e.pts);
    totalEdgeLength += len;
    if (len > maxEdgeLength) maxEdgeLength = len;
  }
  const meanEdgeLength = edges.length ? totalEdgeLength / edges.length : 0;

  // ── Extents ──
  const nb = boundsOf(nodes);
  const width = nb.w;
  const height = nb.h;
  const area = width * height;
  let leafArea = 0;
  let leafSizeSum = 0;
  let leafCount = 0;
  for (const n of nodes) {
    if (n.isCompound) continue;
    leafArea += n.w * n.h;
    leafSizeSum += (n.w + n.h) / 2;
    leafCount++;
  }
  if (leafCount === 0) {
    for (const n of nodes) leafSizeSum += (n.w + n.h) / 2;
    leafCount = N;
  }
  const meanLeafSize = leafCount ? leafSizeSum / leafCount : 0;
  const normalizedEdgeLength = meanLeafSize > 0 ? meanEdgeLength / meanLeafSize : 0;
  const areaEfficiency = area > 0 ? leafArea / area : 0;
  const aspectRatio = height > 0 ? width / height : 0;
  const aspectDeviation = N > 0 && aspectRatio > 0 ? Math.abs(Math.log(aspectRatio / viewportAspect)) : 0;

  // ── Segments ──
  const segs = [];
  for (let ei = 0; ei < edges.length; ei++) {
    const pts = edges[ei].pts;
    for (let k = 0; k + 1 < pts.length; k++) {
      const a = pts[k];
      const b = pts[k + 1];
      if (a.x === b.x && a.y === b.y) continue;
      segs.push({ ei, a, b, gx0: 0, gy0: 0, gx1: 0, gy1: 0 });
    }
  }

  let edgeNodeCrossings = 0;
  let edgeCrossings = 0;
  if (segs.length > 0 && N > 0) {
    const bounds = { minX: nb.minX, minY: nb.minY, maxX: nb.maxX, maxY: nb.maxY };
    for (const s of segs) {
      bounds.minX = Math.min(bounds.minX, s.a.x, s.b.x);
      bounds.minY = Math.min(bounds.minY, s.a.y, s.b.y);
      bounds.maxX = Math.max(bounds.maxX, s.a.x, s.b.x);
      bounds.maxY = Math.max(bounds.maxY, s.a.y, s.b.y);
    }

    // ── Edges through nodes ──
    const nodeGrid = makeGrid(bounds, N);
    for (let i = 0; i < N; i++) {
      const n = nodes[i];
      gridInsert(nodeGrid, n.cx - n.w / 2, n.cy - n.h / 2, n.cx + n.w / 2, n.cy + n.h / 2, i);
    }
    const exclMark = new Int32Array(N);
    const seenNode = new Int32Array(N);
    let segStamp = 0;
    let si = 0;
    for (let ei = 0; ei < edges.length; ei++) {
      const stamp = ei + 1;
      const markUp = (start) => {
        let cur = start;
        let steps = 0;
        while (cur !== -1 && steps++ < MAX_ANCESTOR_STEPS) {
          exclMark[cur] = stamp;
          cur = parentIdx[cur];
        }
      };
      markUp(edges[ei].si);
      markUp(edges[ei].ti);
      while (si < segs.length && segs[si].ei === ei) {
        const s = segs[si++];
        segStamp++;
        const gx0 = cellX(nodeGrid, Math.min(s.a.x, s.b.x));
        const gx1 = cellX(nodeGrid, Math.max(s.a.x, s.b.x));
        const gy0 = cellY(nodeGrid, Math.min(s.a.y, s.b.y));
        const gy1 = cellY(nodeGrid, Math.max(s.a.y, s.b.y));
        for (let gy = gy0; gy <= gy1; gy++) {
          for (let gx = gx0; gx <= gx1; gx++) {
            const list = nodeGrid.cells[gy * nodeGrid.cols + gx];
            if (!list) continue;
            for (const j of list) {
              if (seenNode[j] === segStamp) continue;
              seenNode[j] = segStamp;
              if (exclMark[j] === stamp) continue;
              if (segmentIntersectsRectInterior(s.a, s.b, nodes[j], 3)) edgeNodeCrossings++;
            }
          }
        }
      }
    }

    // ── Edge crossings ──
    const segGrid = makeGrid(bounds, segs.length);
    for (let k = 0; k < segs.length; k++) {
      const s = segs[k];
      const r = gridInsert(segGrid, s.a.x, s.a.y, s.b.x, s.b.y, k);
      s.gx0 = r.gx0;
      s.gy0 = r.gy0;
      s.gx1 = r.gx1;
      s.gy1 = r.gy1;
    }
    const shareEndpoint = (i, j) => {
      const a = edges[i];
      const b = edges[j];
      return a.si === b.si || a.si === b.ti || a.ti === b.si || a.ti === b.ti;
    };
    for (let gy = 0; gy < segGrid.rows; gy++) {
      for (let gx = 0; gx < segGrid.cols; gx++) {
        const list = segGrid.cells[gy * segGrid.cols + gx];
        if (!list || list.length < 2) continue;
        for (let p = 0; p < list.length; p++) {
          const s = segs[list[p]];
          for (let q = p + 1; q < list.length; q++) {
            const t = segs[list[q]];
            if (s.ei === t.ei) continue;
            // Count each pair once: in the first cell their cell ranges share.
            if (gx !== Math.max(s.gx0, t.gx0) || gy !== Math.max(s.gy0, t.gy0)) continue;
            if (shareEndpoint(s.ei, t.ei)) continue;
            if (segmentsProperlyIntersect(s.a, s.b, t.a, t.b)) edgeCrossings++;
          }
        }
      }
    }
  }

  return {
    nodes: N,
    edges: edges.length,
    nodeOverlaps,
    edgeNodeCrossings,
    edgeCrossings,
    bends,
    totalEdgeLength,
    meanEdgeLength,
    normalizedEdgeLength,
    maxEdgeLength,
    width,
    height,
    area,
    areaEfficiency,
    aspectRatio,
    aspectDeviation
  };
}

/**
 * One number for comparing candidate layouts; lower is better. Hard-constraint violations
 * (overlaps, edges through nodes) dominate every soft criterion. Non-finite terms count as 0.
 * @param {ReturnType<typeof measureLayout>} metrics
 * @returns {number}
 */
export function scoreLayout(metrics) {
  const m = metrics || {};
  const v = (k) => finiteOr(m[k], 0);
  // Soft terms are per edge or relative, so a diagram's size does not decide which criterion
  // dominates: crossings per edge, bends per edge, mean edge length in card sizes (sprawl),
  // unused area and distance from the viewport's shape all weigh in on the same scale.
  const edges = Math.max(1, v('edges'));
  const score =
    1e6 * v('nodeOverlaps') +
    1e4 * v('edgeNodeCrossings') +
    25 * (v('edgeCrossings') / edges) +
    3 * (v('bends') / edges) +
    10 * v('normalizedEdgeLength') +
    40 * (1 - v('areaEfficiency')) +
    20 * v('aspectDeviation');
  return Number.isFinite(score) ? score : 0;
}
