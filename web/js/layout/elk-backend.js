/**
 * OpenHeart ELK backend: runs the Eclipse Layout Kernel (elkjs, loaded as a classic script that
 * defines globalThis.ELK) on one layout block and returns a Drawing.
 *
 * A block is a set of sibling model nodes with their whole subtrees and the edges among them.
 * Two algorithms are used:
 *  - layered: ELK's Sugiyama pipeline with INCLUDE_CHILDREN hierarchy handling, so packages are
 *    laid out together with their contents and edges that cross package borders are routed
 *    orthogonally in one pass (network-simplex layering, layer-sweep crossing minimisation with
 *    greedy switch, Brandes-Koepf or network-simplex node placement). Detached children the
 *    engine already packed are given to ELK as one fixed-size leaf per compound and expanded
 *    afterwards.
 *  - stress: ELK's stress majorisation on a flat block, followed by minimal-displacement overlap
 *    removal (overlap.js). Stress drawings carry no routes; the engine draws straight edges.
 *
 * Coordinates in the returned Drawing are relative to the drawing origin; boxes are top-left
 * { x, y, w, h }; routes run in each edge's own source-to-target direction. Because ELK versions
 * disagree on the frame of edge coordinates, each route's frame is chosen per edge as the
 * candidate offset that puts its end points on the borders of its end nodes.
 */

import { distanceToRectBorder, simplifyPolyline, SPACING, fitAspect } from './geometry.js';
import { removeOverlaps } from './overlap.js';

const ROOT_ID = '__oh_root__';
const PACK_PREFIX = '__oh_pack__:';

/**
 * @typedef {{ x: number, y: number, w: number, h: number }} Box  top-left box
 * @typedef {{ points: Array<{x: number, y: number}>, label?: { x: number, y: number } }} Route
 * @typedef {{ w: number, h: number, boxes: Map<string, Box>, routes: Map<string, Route> }} Drawing
 */

let elkInstance = null;

/** True when elkjs is loaded (globalThis.ELK is its constructor). */
export function elkAvailable() {
  return typeof globalThis !== 'undefined' && typeof globalThis.ELK === 'function';
}

function getElk() {
  if (!elkAvailable()) {
    throw new Error('ELK (elkjs) is not loaded: globalThis.ELK is missing, so the layered and stress layouts cannot run.');
  }
  if (!elkInstance) elkInstance = new globalThis.ELK();
  return elkInstance;
}

function num(value, fallback) {
  return Number.isFinite(value) ? value : fallback;
}

function fmt(value) {
  const v = Number.isFinite(value) ? value : 0;
  return String(Math.round(v * 100) / 100);
}

function int(value, fallback, min = 0) {
  return String(Math.max(min, Math.round(num(value, fallback))));
}

function paddingString(top, left, bottom, right) {
  return `[top=${fmt(top)},left=${fmt(left)},bottom=${fmt(bottom)},right=${fmt(right)}]`;
}

function resolveSpacing(config) {
  return Object.assign({}, SPACING, (config && config.spacing) || {});
}

/** Size an ELK leaf gets for a model node (a compound with nothing inside gets its minimum). */
function leafSize(node) {
  if (node.isCompound) {
    const pad = num(node.padding, 36);
    const band = num(node.labelBand, 40);
    return {
      w: Math.max(num(node.minWidth, 0), 2 * pad + 40, 1),
      h: Math.max(2 * pad + band + 40, 1)
    };
  }
  return { w: Math.max(num(node.w, 1), 1), h: Math.max(num(node.h, 1), 1) };
}

function layerOptions(node) {
  if (node.layer === 'first') return { 'elk.layered.layering.layerConstraint': 'FIRST' };
  if (node.layer === 'last') return { 'elk.layered.layering.layerConstraint': 'LAST' };
  return {};
}

function sharedLayeredOptions(config, spacing) {
  return {
    'elk.algorithm': 'layered',
    'elk.direction': config.direction || 'DOWN',
    'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
    'elk.edgeRouting': 'ORTHOGONAL',
    'elk.layered.layering.strategy': 'NETWORK_SIMPLEX',
    'elk.layered.nodePlacement.strategy': config.nodePlacement || 'BRANDES_KOEPF',
    'elk.layered.nodePlacement.bk.fixedAlignment': 'BALANCED',
    'elk.layered.crossingMinimization.strategy': 'LAYER_SWEEP',
    'elk.layered.crossingMinimization.greedySwitch.type': 'TWO_SIDED',
    'elk.layered.thoroughness': int(config.thoroughness, 7, 1),
    'elk.layered.cycleBreaking.strategy': config.cycleBreaking || 'GREEDY',
    'elk.layered.considerModelOrder.strategy': config.modelOrder ? 'NODES_AND_EDGES' : 'NONE',
    'elk.spacing.nodeNode': fmt(spacing.nodeNode),
    'elk.layered.spacing.nodeNodeBetweenLayers': fmt(spacing.betweenLayers),
    'elk.spacing.edgeNode': fmt(spacing.edgeNode),
    'elk.spacing.edgeEdge': fmt(spacing.edgeEdge),
    'elk.layered.spacing.edgeNodeBetweenLayers': fmt(spacing.edgeNode),
    'elk.layered.spacing.edgeEdgeBetweenLayers': fmt(spacing.edgeEdge),
    'elk.spacing.componentComponent': fmt(spacing.component),
    'elk.spacing.edgeLabel': '6',
    'elk.edgeLabels.placement': 'CENTER',
    'elk.layered.mergeEdges': 'false',
    'elk.separateConnectedComponents': 'true'
  };
}

function emptyDrawing(spacing) {
  const rp = num(spacing.rootPadding, SPACING.rootPadding);
  return { w: 2 * rp, h: 2 * rp, boxes: new Map(), routes: new Map() };
}

function translateRoute(route, dx, dy) {
  const out = { points: route.points.map(p => ({ x: p.x + dx, y: p.y + dy })) };
  if (route.label) out.label = { x: route.label.x + dx, y: route.label.y + dy };
  return out;
}

/** Copies every box and route of a packed Drawing into target, translated by (dx, dy). */
function mergeDrawing(target, drawing, dx, dy) {
  if (!drawing) return;
  if (drawing.boxes) {
    drawing.boxes.forEach((b, id) => {
      target.boxes.set(id, { x: b.x + dx, y: b.y + dy, w: b.w, h: b.h });
    });
  }
  if (drawing.routes) {
    drawing.routes.forEach((r, id) => {
      if (r && Array.isArray(r.points)) target.routes.set(id, translateRoute(r, dx, dy));
    });
  }
}

/**
 * Orders an edge's sections along their incoming/outgoing chain and concatenates their points.
 * @returns {Array<{x: number, y: number}>|null}
 */
function sectionPoints(sections) {
  if (!Array.isArray(sections) || sections.length === 0) return null;
  let ordered = sections;
  if (sections.length > 1) {
    const byId = new Map();
    sections.forEach(s => { if (s && s.id !== undefined) byId.set(s.id, s); });
    const hasIncoming = s => Array.isArray(s.incomingSections) && s.incomingSections.some(id => byId.has(id));
    const start = sections.find(s => s && !hasIncoming(s)) || sections[0];
    ordered = [];
    const seen = new Set();
    let cur = start;
    while (cur && !seen.has(cur)) {
      ordered.push(cur);
      seen.add(cur);
      const nextIds = Array.isArray(cur.outgoingSections) ? cur.outgoingSections : [];
      let next = null;
      for (const id of nextIds) {
        const s = byId.get(id);
        if (s && !seen.has(s)) { next = s; break; }
      }
      cur = next;
    }
  }
  const pts = [];
  const push = p => {
    if (!p || !Number.isFinite(p.x) || !Number.isFinite(p.y)) return;
    const last = pts[pts.length - 1];
    if (last && Math.abs(last.x - p.x) < 1e-6 && Math.abs(last.y - p.y) < 1e-6) return;
    pts.push({ x: p.x, y: p.y });
  };
  for (const s of ordered) {
    if (!s) continue;
    push(s.startPoint);
    (Array.isArray(s.bendPoints) ? s.bendPoints : []).forEach(push);
    push(s.endPoint);
  }
  return pts.length >= 2 ? pts : null;
}

/**
 * Builds the ELK JSON graph for a layered block.
 * @returns {{ root: object, elkNodes: Map<string, object>, elkParent: Map<string, string>, modelIds: Set<string>, packOwner: Map<string, string>, edgeById: Map<string, object> }}
 */
function buildLayeredGraph(block, config, spacing) {
  const shared = sharedLayeredOptions(config, spacing);
  const rp = num(spacing.rootPadding, SPACING.rootPadding);
  const root = {
    id: ROOT_ID,
    layoutOptions: Object.assign({}, shared, {
      'elk.padding': paddingString(rp, rp, rp, rp),
      'elk.aspectRatio': fmt(num(config.aspectRatio, 1.6)),
      'elk.layered.wrapping.strategy': config.wrapping ? 'MULTI_EDGE' : 'OFF',
      'elk.randomSeed': '1'
    }),
    children: [],
    edges: []
  };

  const excluded = block.excluded instanceof Set ? block.excluded : new Set();
  const packed = block.packed instanceof Map ? block.packed : new Map();
  const elkNodes = new Map([[ROOT_ID, root]]);
  const elkParent = new Map();
  const modelIds = new Set();
  const packOwner = new Map();

  const addPackNode = (ownerId, container) => {
    const drawing = packed.get(ownerId);
    if (!drawing) return;
    const pid = PACK_PREFIX + ownerId;
    if (elkNodes.has(pid)) return;
    const packNode = { id: pid, width: Math.max(num(drawing.w, 1), 1), height: Math.max(num(drawing.h, 1), 1) };
    container.children.push(packNode);
    elkNodes.set(pid, packNode);
    elkParent.set(pid, container.id);
    packOwner.set(pid, ownerId);
  };

  const visit = (node, container) => {
    if (!node || excluded.has(node.id) || elkNodes.has(node.id)) return;
    const kids = node.isCompound ? (node.children || []).filter(c => c && !excluded.has(c.id)) : [];
    const hasPack = node.isCompound && packed.has(node.id);
    let elkNode;
    if (node.isCompound && (kids.length > 0 || hasPack)) {
      const pad = num(node.padding, 36);
      const band = num(node.labelBand, 40);
      const minW = Math.max(num(node.minWidth, 0), 2 * pad + 40, 1);
      const minH = 2 * pad + band + 40;
      elkNode = {
        id: node.id,
        layoutOptions: Object.assign({}, shared, {
          'elk.padding': paddingString(pad + band, pad, pad, pad),
          'elk.nodeSize.constraints': 'MINIMUM_SIZE',
          'elk.nodeSize.minimum': `(${fmt(minW)},${fmt(minH)})`
        }),
        children: [],
        edges: []
      };
    } else {
      const size = leafSize(node);
      elkNode = { id: node.id, width: size.w, height: size.h, layoutOptions: layerOptions(node) };
    }
    container.children.push(elkNode);
    elkNodes.set(node.id, elkNode);
    elkParent.set(node.id, container.id);
    modelIds.add(node.id);
    if (elkNode.children) {
      kids.forEach(child => visit(child, elkNode));
      if (hasPack) addPackNode(node.id, elkNode);
    }
  };

  (block.items || []).forEach(item => visit(item, root));
  // Defensive: a root-level packed group (keyed by the root id) becomes one leaf in the root.
  if (packed.has(ROOT_ID)) addPackNode(ROOT_ID, root);

  const ancestorsOf = id => {
    const chain = [];
    let cur = elkParent.get(id);
    while (cur !== undefined) {
      chain.push(cur);
      if (cur === ROOT_ID) break;
      cur = elkParent.get(cur);
    }
    if (chain[chain.length - 1] !== ROOT_ID) chain.push(ROOT_ID);
    return chain;
  };

  const edgeById = new Map();
  for (const edge of block.edges || []) {
    if (!edge || edge.selfLoop || edge.nested || edgeById.has(edge.id)) continue;
    if (!edge.source || !edge.target) continue;
    const src = edge.reverse ? edge.target.id : edge.source.id;
    const tgt = edge.reverse ? edge.source.id : edge.target.id;
    if (src === tgt || !modelIds.has(src) || !modelIds.has(tgt)) continue;

    const targetAncestors = new Set(ancestorsOf(tgt));
    let containerId = ROOT_ID;
    for (const a of ancestorsOf(src)) {
      if (targetAncestors.has(a)) { containerId = a; break; }
    }
    let container = elkNodes.get(containerId);
    if (!container || !Array.isArray(container.edges)) container = root;

    const elkEdge = { id: edge.id, sources: [src], targets: [tgt] };
    const pr = typeof config.edgePriority === 'function' ? config.edgePriority(edge) : null;
    if (pr) {
      elkEdge.layoutOptions = {
        'elk.layered.priority.direction': int(pr.direction, 0),
        'elk.layered.priority.shortness': int(pr.shortness, 0),
        'elk.layered.priority.straightness': int(pr.straightness, 0)
      };
    }
    if (config.edgeLabels && edge.labelSize && Number.isFinite(edge.labelSize.w) && Number.isFinite(edge.labelSize.h)) {
      elkEdge.labels = [{
        id: `${edge.id}::label`,
        text: String(edge.label === undefined || edge.label === null ? '' : edge.label),
        width: Math.max(1, edge.labelSize.w),
        height: Math.max(1, edge.labelSize.h)
      }];
    }
    container.edges.push(elkEdge);
    edgeById.set(edge.id, edge);
  }

  return { root, elkNodes, elkParent, modelIds, packOwner, edgeById, packed };
}

/** Reads absolute positions of every node in an ELK result. */
function collectAbsolute(out) {
  const abs = new Map([[ROOT_ID, { x: 0, y: 0 }]]);
  const size = new Map([[ROOT_ID, { w: num(out.width, 0), h: num(out.height, 0) }]]);
  const stack = [[out, { x: 0, y: 0 }]];
  while (stack.length) {
    const [node, origin] = stack.pop();
    const kids = Array.isArray(node.children) ? node.children : [];
    for (const c of kids) {
      const p = { x: origin.x + num(c.x, 0), y: origin.y + num(c.y, 0) };
      abs.set(c.id, p);
      size.set(c.id, { w: num(c.width, 0), h: num(c.height, 0) });
      stack.push([c, p]);
    }
  }
  return { abs, size };
}

async function runLayered(block, config, spacing) {
  const built = buildLayeredGraph(block, config, spacing);
  if (built.modelIds.size === 0 && built.packOwner.size === 0) return emptyDrawing(spacing);

  const out = await getElk().layout(built.root);
  const { abs, size } = collectAbsolute(out);

  /** @type {Drawing} */
  const drawing = { w: num(out.width, 0), h: num(out.height, 0), boxes: new Map(), routes: new Map() };

  abs.forEach((p, id) => {
    if (id === ROOT_ID) return;
    if (built.packOwner.has(id)) return;
    if (!built.modelIds.has(id)) return;
    const s = size.get(id);
    drawing.boxes.set(id, { x: p.x, y: p.y, w: s.w, h: s.h });
  });
  built.packOwner.forEach((ownerId, pid) => {
    const p = abs.get(pid);
    if (!p) return;
    mergeDrawing(drawing, built.packed.get(ownerId), p.x, p.y);
  });

  const rectOf = id => {
    const p = abs.get(id);
    const s = size.get(id);
    if (!p || !s) return null;
    return { cx: p.x + s.w / 2, cy: p.y + s.h / 2, w: s.w, h: s.h };
  };

  // Every ELK node (root included) may hold edges; remember where each edge was defined.
  const holders = [];
  const walk = [out];
  while (walk.length) {
    const n = walk.pop();
    if (Array.isArray(n.edges) && n.edges.length) holders.push(n);
    if (Array.isArray(n.children)) n.children.forEach(c => walk.push(c));
  }
  // Process edges in model order for a deterministic result.
  const outEdges = new Map();
  holders.forEach(h => h.edges.forEach(e => {
    if (e && built.edgeById.has(e.id) && !outEdges.has(e.id)) outEdges.set(e.id, { e, holderId: h.id === undefined ? ROOT_ID : h.id });
  }));

  built.edgeById.forEach((edge, edgeId) => {
    const entry = outEdges.get(edgeId);
    if (!entry) return;
    const { e, holderId } = entry;
    const points = sectionPoints(e.sections);
    if (!points) return;
    const src = Array.isArray(e.sources) ? e.sources[0] : null;
    const tgt = Array.isArray(e.targets) ? e.targets[0] : null;
    const srcRect = rectOf(src);
    const tgtRect = rectOf(tgt);
    if (!srcRect || !tgtRect) return;

    const candidates = [];
    const addCandidate = id => {
      const p = id === undefined ? null : abs.get(id);
      if (!p) return;
      if (!candidates.some(c => c.x === p.x && c.y === p.y)) candidates.push({ x: p.x, y: p.y });
    };
    addCandidate(holderId);
    addCandidate(ROOT_ID);
    addCandidate(built.elkParent.get(src));
    addCandidate(built.elkParent.get(tgt));

    const first = points[0];
    const last = points[points.length - 1];
    let best = candidates[0];
    let bestCost = Infinity;
    for (const c of candidates) {
      const cost = distanceToRectBorder({ x: first.x + c.x, y: first.y + c.y }, srcRect) +
        distanceToRectBorder({ x: last.x + c.x, y: last.y + c.y }, tgtRect);
      if (cost < bestCost - 1e-6) {
        bestCost = cost;
        best = c;
      }
    }

    let routePoints = points.map(p => ({ x: p.x + best.x, y: p.y + best.y }));
    if (edge.reverse) routePoints.reverse();
    routePoints = simplifyPolyline(routePoints);
    if (routePoints.length < 2) return;

    /** @type {Route} */
    const route = { points: routePoints };
    const lbl = Array.isArray(e.labels) ? e.labels[0] : null;
    if (lbl && Number.isFinite(lbl.x) && Number.isFinite(lbl.y)) {
      route.label = {
        x: lbl.x + num(lbl.width, 0) / 2 + best.x,
        y: lbl.y + num(lbl.height, 0) / 2 + best.y
      };
    }
    drawing.routes.set(edgeId, route);
  });

  return drawing;
}

async function runStress(block, config, spacing) {
  const rp = num(spacing.rootPadding, SPACING.rootPadding);
  const excluded = block.excluded instanceof Set ? block.excluded : new Set();
  const leaves = (block.items || []).filter(n => n && !excluded.has(n.id));
  if (leaves.length === 0) return emptyDrawing(spacing);

  const ids = new Set(leaves.map(n => n.id));
  const sizes = new Map(leaves.map(n => [n.id, leafSize(n)]));
  const desired = num(config.desiredEdgeLength, 160);
  const root = {
    id: ROOT_ID,
    layoutOptions: {
      'elk.algorithm': 'stress',
      'elk.stress.desiredEdgeLength': fmt(desired),
      'elk.stress.epsilon': '0.0001',
      'elk.stress.iterationLimit': '400',
      'elk.padding': paddingString(rp, rp, rp, rp),
      'elk.randomSeed': '1'
    },
    children: leaves.map(n => ({ id: n.id, width: sizes.get(n.id).w, height: sizes.get(n.id).h })),
    edges: []
  };
  const seenEdges = new Set();
  for (const edge of block.edges || []) {
    if (!edge || edge.selfLoop || edge.nested || seenEdges.has(edge.id)) continue;
    if (!edge.source || !edge.target) continue;
    const s = edge.source.id;
    const t = edge.target.id;
    if (s === t || !ids.has(s) || !ids.has(t)) continue;
    root.edges.push({ id: edge.id, sources: [s], targets: [t] });
    seenEdges.add(edge.id);
  }

  const out = await getElk().layout(root);
  const placed = new Map();
  (Array.isArray(out.children) ? out.children : []).forEach(c => placed.set(c.id, c));

  // Rects in model order so overlap removal tie-breaks are stable.
  const rects = leaves.map((n, i) => {
    const s = sizes.get(n.id);
    const c = placed.get(n.id);
    const x = c ? num(c.x, 0) : i * (s.w + spacing.nodeNode);
    const y = c ? num(c.y, 0) : 0;
    return { id: n.id, cx: x + s.w / 2, cy: y + s.h / 2, w: s.w, h: s.h };
  });
  // Overlap removal separates the last overlapping pairs vertically, which undoes a shape
  // correction made before it; alternating the two settles near the viewport's shape.
  const overlapGap = num(spacing.overlapGap, SPACING.overlapGap);
  for (let round = 0; round < 2; round++) {
    fitAspect(rects, num(config.aspectRatio, 1.6));
    removeOverlaps(rects, { gap: overlapGap });
  }

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  rects.forEach(r => {
    minX = Math.min(minX, r.cx - r.w / 2);
    minY = Math.min(minY, r.cy - r.h / 2);
    maxX = Math.max(maxX, r.cx + r.w / 2);
    maxY = Math.max(maxY, r.cy + r.h / 2);
  });
  const dx = rp - minX;
  const dy = rp - minY;

  /** @type {Drawing} */
  const drawing = { w: maxX - minX + 2 * rp, h: maxY - minY + 2 * rp, boxes: new Map(), routes: new Map() };
  rects.forEach(r => {
    drawing.boxes.set(r.id, { x: r.cx - r.w / 2 + dx, y: r.cy - r.h / 2 + dy, w: r.w, h: r.h });
  });
  return drawing;
}

/**
 * Lays out one block with ELK.
 * @param {{ items: object[], edges: object[], packed?: Map<string, Drawing>, excluded?: Set<string> }} block
 *   items: sibling model nodes (MNode) with their subtrees; edges: MEdge list with both ends in the
 *   block (self loops and nested edges are skipped); packed: compound id -> pre-packed Drawing of
 *   its detached children, placed as one leaf `__oh_pack__:<compoundId>`; excluded: ids of nodes
 *   living inside packed drawings.
 * @param {{ algorithm?: 'layered'|'stress', direction?: string, nodePlacement?: string, thoroughness?: number,
 *   cycleBreaking?: string, modelOrder?: boolean, wrapping?: boolean, aspectRatio?: number, edgeLabels?: boolean,
 *   spacing?: object, desiredEdgeLength?: number, edgePriority?: (edge: object) => { direction: number, shortness: number, straightness: number } }} config
 * @returns {Promise<Drawing>} boxes are top-left relative to the drawing origin; routes run in each edge's own direction.
 */
export async function runElk(block, config = {}) {
  const cfg = config || {};
  const spacing = resolveSpacing(cfg);
  const safeBlock = block || { items: [], edges: [] };
  if (cfg.algorithm === 'stress') return runStress(safeBlock, cfg, spacing);
  return runLayered(safeBlock, cfg, spacing);
}
