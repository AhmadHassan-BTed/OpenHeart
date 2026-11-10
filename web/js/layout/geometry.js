/**
 * OpenHeart layout geometry: the single source of truth for every size the layout engine
 * reserves and the Cytoscape stylesheet draws. Package padding, package label bands and edge
 * label sizes are defined here once, so the space a layout reserves and the box the canvas
 * draws cannot drift apart.
 *
 * Conventions used by every module in web/js/layout/:
 *  - A rectangle is { cx, cy, w, h }: centre and size, in model pixels, y grows downward.
 *  - A point is { x, y }.
 *  - An edge route is stored on the edge as data.layoutRoute (see ROUTE FORMAT in engine.js).
 */

/** Compound (package) padding by nest level, as drawn by the stylesheet. Deeper levels use the last. */
export const PACKAGE_PADDING = Object.freeze([44, 38, 32, 28, 24]);
export const PACKAGE_FALLBACK_PADDING = 36;

/**
 * Package label style by nest level. The label is drawn centred above the package box
 * (text-valign top, text-halign center), raised by marginY.
 */
export const PACKAGE_LABEL = Object.freeze([
  Object.freeze({ fontSize: 12.5, padX: 14, padY: 6, border: 2, marginY: 14, letterSpacing: 0.03 }),
  Object.freeze({ fontSize: 12, padX: 12, padY: 5, border: 1.8, marginY: 13, letterSpacing: 0 }),
  Object.freeze({ fontSize: 11.5, padX: 12, padY: 5, border: 1.8, marginY: 13, letterSpacing: 0 }),
  Object.freeze({ fontSize: 11, padX: 10, padY: 4, border: 1.8, marginY: 12, letterSpacing: 0 }),
  Object.freeze({ fontSize: 11, padX: 10, padY: 4, border: 2, marginY: 12, letterSpacing: 0 })
]);
export const PACKAGE_FALLBACK_LABEL = Object.freeze({ fontSize: 11, padX: 12, padY: 5, border: 1.5, marginY: 13, letterSpacing: 0 });

/** Edge label style (matches the default edge rule in the stylesheet). */
export const EDGE_LABEL = Object.freeze({ fontSize: 10, padX: 6, padY: 3, border: 1 });

/** Spacing defaults in model pixels. Profiles in semantics.js scale these per diagram type. */
export const SPACING = Object.freeze({
  nodeNode: 56,
  betweenLayers: 76,
  edgeNode: 22,
  edgeEdge: 14,
  component: 96,
  packGap: 56,
  overlapGap: 28,
  rootPadding: 40
});

export const DEFAULT_NODE_SIZE = Object.freeze({ w: 260, h: 140 });

function levelIndex(nestLevel) {
  const n = Number.isFinite(nestLevel) ? Math.max(0, Math.floor(nestLevel)) : 0;
  return Math.min(n, PACKAGE_PADDING.length - 1);
}

/** Padding the stylesheet draws inside a package box at this nest level. */
export function packagePadding(nestLevel) {
  return PACKAGE_PADDING[levelIndex(nestLevel)];
}

/** Label style for a package at this nest level. */
export function packageLabelStyle(nestLevel) {
  return PACKAGE_LABEL[levelIndex(nestLevel)] || PACKAGE_FALLBACK_LABEL;
}

/**
 * Monospace text width estimate. The studio draws every label in JetBrains Mono, whose advance
 * is 0.6em for every glyph and weight; 0.62 leaves a small safety margin.
 * @param {string} text
 * @param {number} fontSize px
 * @param {{ monospace?: boolean, letterSpacing?: number }} [opts] letterSpacing in em
 */
export function estimateTextWidth(text, fontSize = 11, opts = {}) {
  const str = text === undefined || text === null ? '' : String(text);
  if (!str) return 0;
  let longest = 0;
  for (const line of str.split('\n')) longest = Math.max(longest, [...line].length);
  const advance = opts.monospace === false ? 0.56 : 0.62;
  return longest * fontSize * (advance + (opts.letterSpacing || 0));
}

/** Number of text lines in a label. */
export function lineCount(text) {
  const str = text === undefined || text === null ? '' : String(text);
  return str ? str.split('\n').length : 0;
}

/** Height of the band above a package's drawn box that its label occupies, plus clearance. */
export function packageLabelBand(nestLevel) {
  const s = packageLabelStyle(nestLevel);
  return Math.ceil(s.marginY + s.fontSize * 1.25 + 2 * s.padY + 2 * s.border + 6);
}

/** Minimum width a package needs so its centred label fits above it. */
export function packageLabelWidth(text, nestLevel) {
  const s = packageLabelStyle(nestLevel);
  const textW = estimateTextWidth(text, s.fontSize, { letterSpacing: s.letterSpacing });
  return Math.ceil(textW + 2 * s.padX + 2 * s.border + 16);
}

/** Size of an edge label box, or null when the edge has no label. */
export function edgeLabelSize(text) {
  const str = text === undefined || text === null ? '' : String(text).trim();
  if (!str) return null;
  const lines = lineCount(str);
  return {
    w: Math.ceil(estimateTextWidth(str, EDGE_LABEL.fontSize) + 2 * EDGE_LABEL.padX + 2 * EDGE_LABEL.border),
    h: Math.ceil(lines * EDGE_LABEL.fontSize * 1.25 + 2 * EDGE_LABEL.padY + 2 * EDGE_LABEL.border)
  };
}

/** Card size from Cytoscape node data, falling back to the default card size. */
export function nodeSize(data) {
  const w = data && Number.isFinite(data.width) && data.width > 0 ? data.width : DEFAULT_NODE_SIZE.w;
  const h = data && Number.isFinite(data.height) && data.height > 0 ? data.height : DEFAULT_NODE_SIZE.h;
  return { w, h };
}

/** Left, top, right, bottom of a { cx, cy, w, h } rectangle. */
export function edgesOf(r) {
  return { left: r.cx - r.w / 2, top: r.cy - r.h / 2, right: r.cx + r.w / 2, bottom: r.cy + r.h / 2 };
}

/**
 * True when a and b are closer than gap: their borders are less than gap apart on both axes
 * (gap 0 tests interior overlap; a negative gap is a tolerance).
 */
export function rectsOverlap(a, b, gap = 0) {
  return Math.abs(a.cx - b.cx) * 2 < a.w + b.w + 2 * gap && Math.abs(a.cy - b.cy) * 2 < a.h + b.h + 2 * gap;
}

/** How much a and b intrude into a clearance of gap along x and y (positive when they do). */
export function overlapExtent(a, b, gap = 0) {
  return {
    x: (a.w + b.w) / 2 + gap - Math.abs(a.cx - b.cx),
    y: (a.h + b.h) / 2 + gap - Math.abs(a.cy - b.cy)
  };
}

/** Bounding box of rectangles: { minX, minY, maxX, maxY, w, h }; all zero when empty. */
export function boundsOf(rects) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const r of rects) {
    if (!r) continue;
    minX = Math.min(minX, r.cx - r.w / 2);
    minY = Math.min(minY, r.cy - r.h / 2);
    maxX = Math.max(maxX, r.cx + r.w / 2);
    maxY = Math.max(maxY, r.cy + r.h / 2);
  }
  if (minX === Infinity) return { minX: 0, minY: 0, maxX: 0, maxY: 0, w: 0, h: 0 };
  return { minX, minY, maxX, maxY, w: maxX - minX, h: maxY - minY };
}

/** Distance from point p to the border of rectangle r (0 when p lies on it). */
export function distanceToRectBorder(p, r) {
  const e = edgesOf(r);
  const inside = p.x > e.left && p.x < e.right && p.y > e.top && p.y < e.bottom;
  if (inside) return Math.min(p.x - e.left, e.right - p.x, p.y - e.top, e.bottom - p.y);
  const dx = Math.max(e.left - p.x, 0, p.x - e.right);
  const dy = Math.max(e.top - p.y, 0, p.y - e.bottom);
  return Math.hypot(dx, dy);
}

/** Point where the ray from r's centre toward p leaves r. */
export function clipToRectBorder(r, p) {
  const dx = p.x - r.cx;
  const dy = p.y - r.cy;
  if (dx === 0 && dy === 0) return { x: r.cx, y: r.cy };
  const sx = dx !== 0 ? (r.w / 2) / Math.abs(dx) : Infinity;
  const sy = dy !== 0 ? (r.h / 2) / Math.abs(dy) : Infinity;
  const s = Math.min(sx, sy);
  return { x: r.cx + dx * s, y: r.cy + dy * s };
}

/**
 * True when segment pq passes through the interior of r shrunk by inset on every side
 * (Liang-Barsky clipping against the shrunk rectangle).
 */
export function segmentIntersectsRectInterior(p, q, r, inset = 0) {
  const left = r.cx - r.w / 2 + inset;
  const right = r.cx + r.w / 2 - inset;
  const top = r.cy - r.h / 2 + inset;
  const bottom = r.cy + r.h / 2 - inset;
  if (left >= right || top >= bottom) return false;
  const dx = q.x - p.x;
  const dy = q.y - p.y;
  let t0 = 0;
  let t1 = 1;
  const clip = (den, num) => {
    if (den === 0) return num < 0 ? false : true;
    const t = num / den;
    if (den > 0) {
      if (t < t0) return false;
      if (t < t1) t1 = t;
    } else {
      if (t > t1) return false;
      if (t > t0) t0 = t;
    }
    return true;
  };
  // Strict interior: a segment that only touches the shrunk border does not count.
  if (!clip(-dx, p.x - left - 1e-9)) return false;
  if (!clip(dx, right - p.x - 1e-9)) return false;
  if (!clip(-dy, p.y - top - 1e-9)) return false;
  if (!clip(dy, bottom - p.y - 1e-9)) return false;
  return t1 - t0 > 1e-9;
}

/** True when segments ab and cd cross at a single interior point (shared endpoints do not count). */
export function segmentsProperlyIntersect(a, b, c, d) {
  const o = (p, q, r) => (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
  const d1 = o(c, d, a);
  const d2 = o(c, d, b);
  const d3 = o(a, b, c);
  const d4 = o(a, b, d);
  return ((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) &&
    ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9));
}

/** Total length of a polyline. */
export function polylineLength(points) {
  let len = 0;
  for (let i = 1; i < points.length; i++) len += Math.hypot(points[i].x - points[i - 1].x, points[i].y - points[i - 1].y);
  return len;
}

/** Removes consecutive duplicate points and collinear interior points from a polyline. */
export function simplifyPolyline(points, eps = 0.5) {
  const out = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (last && Math.abs(last.x - p.x) <= eps && Math.abs(last.y - p.y) <= eps) continue;
    out.push({ x: p.x, y: p.y });
  }
  if (out.length < 3) return out;
  const result = [out[0]];
  for (let i = 1; i < out.length - 1; i++) {
    const a = result[result.length - 1];
    const b = out[i];
    const c = out[i + 1];
    const cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    const dot = (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y);
    if (Math.abs(cross) <= eps * Math.max(1, Math.hypot(c.x - a.x, c.y - a.y)) && dot >= 0) continue;
    result.push(b);
  }
  result.push(out[out.length - 1]);
  return result;
}

/**
 * Stretches a stress arrangement toward the viewport's aspect ratio before overlap removal.
 * Stress keeps relative positions but not the overall shape; overlap removal on a lopsided
 * arrangement then stacks boxes along its long axis, which lengthens every edge. Scaling the
 * centres by s along x and 1/s along y (s = sqrt(target / current), using the box-inclusive
 * extent) keeps neighbourhoods and orders while fixing the shape.
 */
export function fitAspect(rects, target) {
  if (rects.length < 3 || !(target > 0)) return;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let sx0 = 0;
  let sy0 = 0;
  rects.forEach(r => {
    minX = Math.min(minX, r.cx - r.w / 2);
    maxX = Math.max(maxX, r.cx + r.w / 2);
    minY = Math.min(minY, r.cy - r.h / 2);
    maxY = Math.max(maxY, r.cy + r.h / 2);
    sx0 += r.cx;
    sy0 += r.cy;
  });
  const w = maxX - minX;
  const h = maxY - minY;
  if (!(w > 0) || !(h > 0)) return;
  const current = w / h;
  // Only correct a clearly wrong shape; mild deviations are the stress layout's own choice.
  if (Math.abs(Math.log(current / target)) < Math.log(1.5)) return;
  const s = Math.sqrt(target / current);
  const cx = sx0 / rects.length;
  const cy = sy0 / rects.length;
  rects.forEach(r => {
    r.cx = cx + (r.cx - cx) * s;
    r.cy = cy + (r.cy - cy) / s;
  });
}
