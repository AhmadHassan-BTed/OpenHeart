/**
 * OpenHeart adaptive layout engine.
 *
 * Places every diagram the way its notation reads, keeps related elements next to each other,
 * untangles edges and never lets boxes collide. The pipeline:
 *
 *  1. Semantics (semantics.js). The diagram type picks a profile: layout family, conventional
 *     flow directions, relationship roles as edge priorities, first/last-layer pinning and
 *     spacing. Sequence and timing diagrams use dedicated layouts (sequence.js).
 *  2. Model (model.js). A containment tree of packages and cards with the exact padding and
 *     label bands the stylesheet draws, and edges tagged with their role and direction.
 *  3. Decomposition. The root splits into connected components. Inside every package, children
 *     no edge reaches ("detached") are laid out on their own and packed into one tidy block, so
 *     a package with thirty unrelated classes becomes a grid, not a thirty-card row.
 *  4. Placement. Each connected block goes to ELK (elk-backend.js): layered layout with network
 *     simplex layering, layer-sweep crossing minimisation, Brandes-Koepf or network simplex node
 *     placement and orthogonal edge routing, across package borders (hierarchy handling
 *     INCLUDE_CHILDREN), or stress majorisation followed by VPSC overlap removal for
 *     undirected collaboration graphs.
 *  5. Portfolio. Several candidate configurations run per block (directions, node placement,
 *     stress versus layered, wrapping for very long chains). Each is scored with measured
 *     quality metrics (metrics.js: overlaps, edges through cards, crossings, bends, edge length,
 *     area use, fit to the viewport); the best wins. A time budget bounds the search.
 *  6. Packing (pack.js). Components and detached groups are packed toward the viewport's
 *     aspect ratio.
 *  7. Write-back. Cards get positions, packages get the exact box to draw (data.layoutBox),
 *     edges get their routes (data.layoutRoute); apply-routes.js hands both to Cytoscape.
 *
 * ROUTE FORMAT, stored on an edge as data.layoutRoute:
 *   { style: 'orthogonal' | 'straight' | 'bezier', points?: [{x, y}], label?: {x, y} }
 *   points are absolute, in the edge's own source-to-target direction; first and last are the
 *   attachment points on the source and target, the rest are bends. label is the label centre.
 *   Self loops use { style: 'loop-group', count } on the first of three or more loops of a node
 *   (drawn once, labelled with the count) and { style: 'loop-grouped', representative } on the
 *   others (not drawn); sequence self messages use { style: 'embedded' } (drawn in the lifeline).
 * PACKAGE BOX, stored on a compound as data.layoutBox: { cx, cy, w, h }, the drawn box without
 *   the label band above it.
 *
 * When ELK is not loaded, or a diagram is too large for it within the time budget, the legacy
 * deterministic layout (uml-layout.js) is used and the result says so.
 */

import { computeDeterministicLayout } from '../uml-layout.js';
import { profileFor, thoroughnessFor, edgePriority } from './semantics.js';
import { buildModel, childOfContainer, subtreeNodes, lowestCommonAncestor } from './model.js';
import { elkAvailable, runElk } from './elk-backend.js';
import { packBoxes } from './pack.js';
import { removeOverlaps, compact } from './overlap.js';
import { fitAspect } from './geometry.js';
import { routeOrthogonal } from './router.js';
import { measureLayout, scoreLayout } from './metrics.js';
import { layoutSequence, layoutTiming } from './sequence.js';

const LIMITS = Object.freeze({
  maxNodesForElk: 5000,
  maxEdgesForElk: 15000,
  emergencyMs: 30000,
  workBudget: 6000,
  portfolioMaxNodes: 400,
  portfolioMaxEdges: 900,
  networkSimplexMaxNodes: 150,
  networkSimplexMaxUnits: 60,
  layeredFollowUpMax: 400,
  labelMaxEdges: 400
});

/** Card kinds in the order unrelated cards are packed within a package. */
const KIND_RANK = Object.freeze({ interface: 0, abstract: 1, class: 2, record: 3, enum: 4, object: 5 });

const now = () => (typeof performance !== 'undefined' && typeof performance.now === 'function' ? performance.now() : Date.now());

function isEdgeElement(el) {
  return Boolean(el && el.data && el.data.source !== undefined && el.data.target !== undefined);
}

function clearPreviousGeometry(elements) {
  for (const el of elements) {
    if (!el || !el.data) continue;
    delete el.data.layoutRoute;
    delete el.data.layoutBox;
    delete el.data.layoutEmbedded;
  }
}

/**
 * Lays out one diagram.
 * @param {object[]} elements Cytoscape elements (positions and layout data are written in place)
 * @param {string} graphType diagram type id
 * @param {{ viewport?: { width: number, height: number }, budgetMs?: number,
 *   diagnostics?: { onRouterInput?: (input: object, options: object) => void } }} [options]
 *   diagnostics are for test harnesses only (the studio never sets them).
 * @returns {Promise<{ elements: object[], strategy: string, metrics: object|null, warnings: string[] }>}
 */
export async function computeLayout(elements, graphType, options = {}) {
  const list = Array.isArray(elements) ? elements : [];
  const profile = profileFor(graphType);
  const warnings = [];
  clearPreviousGeometry(list);

  if (profile.family === 'sequence') {
    layoutSequence(list, {});
    return { elements: list, strategy: 'sequence: lifelines ordered to shorten messages, one row per message', metrics: null, warnings };
  }
  if (profile.family === 'timing') {
    layoutTiming(list, {});
    return { elements: list, strategy: 'timing: stacked tracks', metrics: null, warnings };
  }

  let nodeCount = 0;
  let edgeCount = 0;
  for (const el of list) {
    if (isEdgeElement(el)) edgeCount++;
    else if (el && el.data) nodeCount++;
  }
  if (!elkAvailable()) {
    warnings.push('The ELK layout library is not loaded, so the legacy layout was used.');
    return { elements: computeDeterministicLayout(list, graphType), strategy: 'legacy (ELK not loaded)', metrics: null, warnings };
  }
  if (nodeCount > LIMITS.maxNodesForElk || edgeCount > LIMITS.maxEdgesForElk) {
    warnings.push(`The diagram has ${nodeCount} nodes and ${edgeCount} edges, above the adaptive engine's limits (${LIMITS.maxNodesForElk} nodes, ${LIMITS.maxEdgesForElk} edges), so the legacy layout was used.`);
    return { elements: computeDeterministicLayout(list, graphType), strategy: 'legacy (diagram too large)', metrics: null, warnings };
  }

  const model = buildModel(list, profile);
  const viewport = options.viewport || {};
  const rawAspect = viewport.width > 0 && viewport.height > 0 ? viewport.width / viewport.height : 1.6;
  const aspect = Math.min(2.6, Math.max(0.6, rawAspect));
  const ctx = {
    model,
    profile,
    aspect,
    // A wall-clock stop far above normal cost: it only ends a pathological search early. The
    // regular candidate budget is deterministic (workUsed against LIMITS.workBudget).
    emergencyStop: now() + (Number.isFinite(options.budgetMs) ? options.budgetMs : LIMITS.emergencyMs),
    workUsed: 0,
    warnings,
    diagnostics: options.diagnostics || null,
    internal: countInternalEdges(model),
    edgeStyles: new Map(),
    stats: { blocks: 0, elkRuns: 0, candidates: 0, packedGroups: 0, winners: new Map(), trials: [] }
  };

  let drawing;
  try {
    drawing = await layoutRoot(ctx);
  } catch (err) {
    // The model already pruned empty package chains, so the fallback gets the pruned list.
    const reason = (err && err.message) || String(err);
    warnings.push(`The adaptive layout failed (${reason}), so the legacy layout was used.`);
    clearPreviousGeometry(model.elements);
    return { elements: computeDeterministicLayout(model.elements, graphType), strategy: 'legacy (adaptive layout failed)', metrics: null, warnings };
  }
  writeBack(model, drawing, ctx);

  let metrics = null;
  try {
    metrics = measureLayout(metricsInput(drawing, model, model.edges.filter(e => e.drawable)), { viewportAspect: aspect });
    if (metrics.nodeOverlaps > 0) warnings.push(`${metrics.nodeOverlaps} overlapping box pairs remain after layout.`);
  } catch (err) {
    warnings.push(`Layout metrics could not be computed: ${(err && err.message) || err}`);
  }
  return { elements: model.elements, strategy: describe(ctx), metrics, warnings, candidates: ctx.stats.trials };
}

/** internal(X) = number of drawable edges whose lowest common ancestor is X or inside X. */
function countInternalEdges(model) {
  const internal = new Map();
  for (const e of model.edges) {
    if (!e.drawable) continue;
    let cur = lowestCommonAncestor(e.source, e.target);
    while (cur) {
      internal.set(cur, (internal.get(cur) || 0) + 1);
      cur = cur.parent;
    }
  }
  return internal;
}

/** Root: connected components over edges projected to root level, each laid out, then packed. */
async function layoutRoot(ctx) {
  const { model } = ctx;
  const parentOf = new Map(model.roots.map(r => [r.id, r.id]));
  const find = (id) => {
    let r = id;
    while (parentOf.get(r) !== r) r = parentOf.get(r);
    let c = id;
    while (parentOf.get(c) !== r) {
      const next = parentOf.get(c);
      parentOf.set(c, r);
      c = next;
    }
    return r;
  };
  for (const e of model.edges) {
    if (!e.drawable) continue;
    const a = childOfContainer(e.source, null);
    const b = childOfContainer(e.target, null);
    if (!a || !b || a === b) continue;
    const ra = find(a.id);
    const rb = find(b.id);
    if (ra !== rb) {
      // Union toward the earlier root so group order follows input order.
      const ia = model.nodes.get(ra).index;
      const ib = model.nodes.get(rb).index;
      if (ia <= ib) parentOf.set(rb, ra);
      else parentOf.set(ra, rb);
    }
  }
  const groups = new Map();
  for (const r of model.roots) {
    const key = find(r.id);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(r);
  }

  const parts = [];
  for (const group of groups.values()) {
    const hasEdges = group.length > 1 || (ctx.internal.get(group[0]) || 0) > 0;
    const drawing = hasEdges ? await layoutBlock(group, ctx) : layoutEdgeFree(group[0], ctx);
    parts.push({ drawing, hasEdges, lead: group[0] });
  }
  if (parts.length === 0) return emptyDrawing();
  if (parts.length === 1) return parts[0].drawing;

  const connected = parts.filter(p => p.hasEdges)
    .sort((a, b) => (b.drawing.w * b.drawing.h) - (a.drawing.w * a.drawing.h) || a.lead.index - b.lead.index);
  // Unconnected cards keep their kind order; unconnected packages pack largest first, which is
  // what keeps skyline packing tight when one package dwarfs the others.
  const freeParts = parts.filter(p => !p.hasEdges);
  const freeCards = sortForPacking(freeParts.filter(p => !p.lead.isCompound), p => p.lead);
  const freePackages = freeParts.filter(p => p.lead.isCompound)
    .sort((a, b) => (b.drawing.w * b.drawing.h) - (a.drawing.w * a.drawing.h) || a.lead.index - b.lead.index);
  const free = [...freePackages, ...freeCards];
  ctx.stats.packedGroups++;
  return packDrawings([...connected, ...free].map(p => p.drawing), ctx.profile.spacing.component, ctx.aspect);
}

/** A node laid out on its own: ELK when its subtree has edges, packing otherwise. */
async function layoutDetached(node, ctx) {
  if ((ctx.internal.get(node) || 0) > 0) return layoutBlock([node], ctx);
  return layoutEdgeFree(node, ctx);
}

/** A subtree without any edge: cards packed into a grid, packages wrapped around their content. */
function layoutEdgeFree(node, ctx) {
  if (!node.isCompound) return leafDrawing(node);
  const kids = sortForPacking(node.children, n => n);
  const content = packDrawings(kids.map(k => layoutEdgeFree(k, ctx)), ctx.profile.spacing.packGap, 1.6);
  ctx.stats.packedGroups++;
  return wrapCompound(node, content);
}

/**
 * One connected block of siblings: detached children inside its packages are laid out and
 * packed first, then the block runs through the candidate portfolio.
 */
async function layoutBlock(items, ctx) {
  ctx.stats.blocks++;
  const blockNodes = new Set();
  const packed = new Map();
  const excluded = new Set();

  const visit = async (node) => {
    blockNodes.add(node.id);
    if (!node.isCompound) return;
    const detached = node.children.filter(c => c.crossCount === 0);
    const attached = node.children.filter(c => c.crossCount > 0);
    if (detached.length) {
      const drawings = [];
      for (const c of sortForPacking(detached, n => n)) {
        drawings.push(await layoutDetached(c, ctx));
        subtreeNodes(c).forEach(n => excluded.add(n.id));
      }
      packed.set(node.id, packDrawings(drawings, ctx.profile.spacing.packGap, 1.6));
      ctx.stats.packedGroups++;
    }
    for (const c of attached) await visit(c);
  };
  for (const it of items) await visit(it);

  const edges = ctx.model.edges.filter(e => e.drawable && blockNodes.has(e.source.id) && blockNodes.has(e.target.id));
  let leaves = packed.size;
  blockNodes.forEach(id => {
    if (!ctx.model.nodes.get(id).isCompound) leaves++;
  });
  const stats = {
    n: leaves,
    e: edges.length,
    flat: packed.size === 0 && items.every(i => !i.isCompound),
    hasHierarchy: edges.some(e => e.role === 'hierarchy'),
    meanCard: meanCardSize(blockNodes, ctx.model)
  };
  const block = { items, edges, packed, excluded };
  const configs = candidateConfigs(stats, ctx);

  // How many candidates a block gets depends only on its size and on the work already spent
  // in this layout, never on the clock, so the same diagram always gets the same layout.
  const units = stats.n + stats.e;
  let allowed = units <= 60 ? Infinity : units <= 250 ? 4 : units <= 700 ? 2 : 1;
  if (ctx.workUsed > LIMITS.workBudget) allowed = 1;

  let best = null;
  let ran = 0;
  for (let i = 0; i < configs.length; i++) {
    if (ran >= allowed) break;
    if (i > 0 && now() > ctx.emergencyStop) break;
    const config = configs[i];
    // ELK layered on a big block costs seconds; as a follow-up candidate it is not worth it.
    if (i > 0 && config.strategy !== 'cluster' && config.algorithm !== 'stress' && stats.n + stats.e > LIMITS.layeredFollowUpMax) continue;
    let drawing;
    ran++;
    ctx.workUsed += units;
    const startedAt = now();
    try {
      ctx.stats.elkRuns++;
      if (config.strategy === 'cluster') {
        drawing = await layoutClustered(block, config, ctx);
      } else {
        drawing = await runElk(block, config);
        if (config.algorithm === 'stress') routeMissing(drawing, edges, ctx);
      }
    } catch (err) {
      if (!best && i === configs.length - 1) throw err;
      if (!best && i === 0) {
        ctx.warnings.push(`Layout candidate ${describeConfig(config)} failed: ${(err && err.message) || err}`);
      }
      continue;
    }
    normalizeDrawing(drawing, items);
    fixCompoundAttachments(drawing, edges);
    ctx.stats.candidates++;
    let score;
    let m = null;
    try {
      m = measureLayout(metricsInput(drawing, ctx.model, edges), { viewportAspect: ctx.aspect });
      // A small preference for earlier (more conventional) candidates breaks near ties.
      score = scoreLayout(m) * (1 + 0.04 * i);
      if (i === 0 && config.algorithm === 'layered' && stats.flat && !config.wrapping &&
          m.aspectDeviation > Math.log(3) && !configs.some(c => c.wrapping)) {
        configs.push(Object.assign({}, config, { wrapping: true }));
      }
    } catch (_) {
      score = i;
    }
    // Every scored candidate is reported with its cost, for tuning and for the studio's status.
    ctx.stats.trials.push({ block: ctx.stats.blocks, candidate: describeConfig(config), nodes: stats.n, edges: stats.e, ms: Math.round(now() - startedAt), score: Math.round(score * 100) / 100,
      metrics: m ? { crossings: m.edgeCrossings, bends: m.bends, edgeLength: Math.round(m.normalizedEdgeLength * 100) / 100, areaUse: Math.round(m.areaEfficiency * 1000) / 1000, aspect: Math.round(m.aspectRatio * 100) / 100 } : null });
    if (!best || score < best.score) best = { drawing, score, config };
  }
  if (!best) throw new Error('No layout candidate produced a drawing.');

  const label = describeConfig(best.config);
  ctx.stats.winners.set(label, (ctx.stats.winners.get(label) || 0) + 1);
  // Edges the router could not route in a stress or cluster drawing are drawn straight.
  if (best.config.algorithm === 'stress' || best.config.strategy === 'cluster') {
    markStraightEdges(edges.filter(e => !best.drawing.routes.has(e.id)), ctx);
  }
  return best.drawing;
}

// ── Cluster strategy: every package laid out on its own, packages arranged so connected ones
//    sit together, edges across package borders routed orthogonally around everything else. ──

/**
 * Lays out a block bottom-up. At each level the children (cards and packages) are boxes:
 * a level of cards with edges among them goes to ELK layered (its routes are kept); a level with
 * packages is arranged by stress over the edges projected onto the children, followed by VPSC
 * overlap removal and compaction; a level without edges is packed. Each package's content is
 * mirrored when that brings its cross-package partners closer. Finally every edge still without
 * a route is routed by the orthogonal router over the finished geometry.
 */
async function layoutClustered(block, config, ctx) {
  const items = block.items;
  const container = items[0] ? items[0].parent : null;
  const level = await clusterArrange(container, items, block, config, ctx, null);
  routeMissing(level, block.edges, ctx);
  return level;
}

async function clusterArrange(container, nodes, block, config, ctx, packedDrawing) {
  const parts = [];
  for (const n of nodes) {
    let d;
    if (!n.isCompound) {
      d = leafDrawing(n);
    } else {
      const attachedKids = n.children.filter(c => c.crossCount > 0);
      const inner = await clusterArrange(n, attachedKids, block, config, ctx, block.packed.get(n.id) || null);
      d = wrapCompound(n, inner);
    }
    parts.push({ key: n.id, node: n, drawing: d });
  }
  if (packedDrawing) parts.push({ key: `__oh_pack__:${container ? container.id : 'root'}`, node: null, drawing: packedDrawing });
  if (parts.length === 0) return emptyDrawing();
  if (parts.length === 1) return parts[0].drawing;

  const partIds = new Set(parts.map(p => p.key));
  const pairs = [];
  const levelEdges = [];
  for (const e of block.edges) {
    const a = childOfContainer(e.source, container);
    const b = childOfContainer(e.target, container);
    if (!a || !b || a === b || !partIds.has(a.id) || !partIds.has(b.id)) continue;
    pairs.push([a.id, b.id]);
    if (a === e.source && b === e.target) levelEdges.push(e);
  }
  if (pairs.length === 0) {
    return packDrawings(sortParts(parts).map(p => p.drawing), ctx.profile.spacing.packGap, 1.6);
  }

  // A level of cards only: ELK layered draws it, routes included.
  if (parts.every(p => p.node && !p.node.isCompound)) {
    const leafConfig = Object.assign({}, config, { strategy: 'elk', algorithm: 'layered', thoroughness: thoroughnessFor(parts.length, levelEdges.length) });
    const packed = packedDrawing ? new Map([['__oh_root__', packedDrawing]]) : new Map();
    const d = await runElk({ items: nodes, edges: levelEdges, packed, excluded: new Set() }, leafConfig);
    // The backend copies the pack node's contents in; normalize over everything it placed.
    return packedDrawing ? normalizeAll(d) : normalizeDrawing(d, nodes);
  }

  // Components of the projected graph: each arranged by stress, then all packed together.
  const uf = new Map(parts.map(p => [p.key, p.key]));
  const find = (k) => {
    while (uf.get(k) !== k) {
      uf.set(k, uf.get(uf.get(k)));
      k = uf.get(k);
    }
    return k;
  };
  pairs.forEach(([a, b]) => {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) uf.set(rb, ra);
  });
  const comps = new Map();
  parts.forEach(p => {
    const r = find(p.key);
    if (!comps.has(r)) comps.set(r, []);
    comps.get(r).push(p);
  });
  const arranged = [];
  const singles = [];
  for (const comp of comps.values()) {
    if (comp.length === 1) {
      singles.push(comp[0]);
      continue;
    }
    const keys = new Set(comp.map(p => p.key));
    const compPairs = pairs.filter(([a]) => keys.has(a));
    arranged.push(await stressArrange(comp, compPairs, config, ctx));
  }
  arranged.sort((a, b) => b.w * b.h - a.w * a.h);
  const drawings = arranged.concat(sortParts(singles).map(p => p.drawing));
  return packDrawings(drawings, ctx.profile.spacing.component, ctx.aspect);
}

/** Packing order for parts: cards by kind, then packages, then pre-packed groups. */
function sortParts(parts) {
  return parts.slice().sort((a, b) => {
    const ra = a.node ? (a.node.isCompound ? 100 : (KIND_RANK[String(a.node.kind).toLowerCase()] ?? 50)) : 200;
    const rb = b.node ? (b.node.isCompound ? 100 : (KIND_RANK[String(b.node.kind).toLowerCase()] ?? 50)) : 200;
    return ra - rb || (a.node ? a.node.index : 1e9) - (b.node ? b.node.index : 1e9);
  });
}

/**
 * Arranges fixed-size parts by stress over their projected edges (ELK stress), then removes
 * overlaps with VPSC, compacts, removes overlaps again, and mirrors package contents toward
 * their partners.
 */
async function stressArrange(parts, pairs, config, ctx) {
  const fakeNodes = parts.map(p => ({ id: p.key, isCompound: false, w: Math.max(1, p.drawing.w), h: Math.max(1, p.drawing.h), children: [] }));
  const byKey = new Map(fakeNodes.map(n => [n.id, n]));
  const fakeEdges = pairs.map(([a, b], i) => ({ id: `__oh_pair__${i}`, source: byKey.get(a), target: byKey.get(b), selfLoop: false, nested: false }));
  const meanSize = fakeNodes.reduce((s, n) => s + (n.w + n.h) / 2, 0) / fakeNodes.length;
  const gap = Math.round(ctx.profile.spacing.component * 0.75);
  const stressConfig = Object.assign({}, config, {
    algorithm: 'stress',
    desiredEdgeLength: Math.round(meanSize * 0.6 + gap),
    spacing: Object.assign({}, config.spacing, { overlapGap: gap, rootPadding: 0 })
  });
  const d = await runElk({ items: fakeNodes, edges: fakeEdges }, stressConfig);
  const rects = fakeNodes.map(n => {
    const b = d.boxes.get(n.id);
    return { id: n.id, cx: b ? b.x + b.w / 2 : 0, cy: b ? b.y + b.h / 2 : 0, w: n.w, h: n.h };
  });
  fitAspect(rects, ctx.aspect);
  compact(rects, { gap, strength: 0.6 });
  removeOverlaps(rects, { gap });
  // Same settling as flat stress layouts: correct the shape again, then clear overlaps again.
  fitAspect(rects, ctx.aspect);
  removeOverlaps(rects, { gap });

  const rectByKey = new Map(rects.map(r => [r.id, r]));
  mirrorTowardPartners(parts, rectByKey, ctx);

  const out = emptyDrawing();
  parts.forEach(p => {
    const r = rectByKey.get(p.key);
    translateInto(out, p.drawing, r.cx - r.w / 2, r.cy - r.h / 2);
  });
  return normalizeAll(out);
}

/**
 * Mirrors a package's content horizontally when that shortens its edges to partners outside it.
 * Only left-right mirroring is used, so top-to-bottom conventions (generalizations pointing up,
 * flow running down) are kept.
 */
function mirrorTowardPartners(parts, rectByKey, ctx) {
  const vertical = ctx.profile.directions[0] === 'DOWN' || ctx.profile.directions[0] === 'UP';
  if (!vertical) return;
  const absCenter = (part, id) => {
    const r = rectByKey.get(part.key);
    const b = part.drawing.boxes.get(id);
    return b ? { x: r.cx - r.w / 2 + b.x + b.w / 2, y: r.cy - r.h / 2 + b.y + b.h / 2 } : null;
  };
  const partOfNode = new Map();
  parts.forEach(p => p.drawing.boxes.forEach((_, id) => partOfNode.set(id, p)));
  for (const p of parts) {
    if (!p.node || !p.node.isCompound) continue;
    let cost = 0;
    let mirroredCost = 0;
    const r = rectByKey.get(p.key);
    for (const e of ctx.model.edges) {
      if (!e.drawable) continue;
      const ps = partOfNode.get(e.source.id);
      const pt = partOfNode.get(e.target.id);
      if (!ps || !pt || ps === pt || (ps !== p && pt !== p)) continue;
      const innerId = ps === p ? e.source.id : e.target.id;
      const outer = ps === p ? absCenter(pt, e.target.id) : absCenter(ps, e.source.id);
      const inner = absCenter(p, innerId);
      if (!inner || !outer) continue;
      const mirroredX = 2 * r.cx - inner.x;
      cost += Math.abs(inner.x - outer.x);
      mirroredCost += Math.abs(mirroredX - outer.x);
    }
    if (mirroredCost < cost - 1) mirrorDrawingX(p.drawing, p.key);
  }
}

/** Mirrors a drawing left-right in place, keeping the box of `keepId` (the package itself). */
function mirrorDrawingX(d, keepId) {
  const W = d.w;
  d.boxes.forEach((b, id) => {
    if (id === keepId) return;
    b.x = W - b.x - b.w;
  });
  d.routes.forEach(r => {
    r.points.forEach(p => { p.x = W - p.x; });
    if (r.label) r.label.x = W - r.label.x;
  });
}

/** Normalizes a composed drawing so the bounding box of every box and route starts at (0, 0). */
function normalizeAll(d) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  d.boxes.forEach(b => {
    minX = Math.min(minX, b.x);
    minY = Math.min(minY, b.y);
    maxX = Math.max(maxX, b.x + b.w);
    maxY = Math.max(maxY, b.y + b.h);
  });
  d.routes.forEach(r => r.points.forEach(p => {
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x);
    maxY = Math.max(maxY, p.y);
  }));
  if (minX === Infinity) return d;
  const out = emptyDrawing();
  translateInto(out, d, -minX, -minY);
  out.w = maxX - minX;
  out.h = maxY - minY;
  return out;
}

/** Side preferences for the router: generalizations leave a child's top and enter a parent's bottom. */
function sideCosts(edge, ctx) {
  const vertical = ctx.profile.directions[0] === 'DOWN';
  if (edge.role === 'hierarchy' && edge.reverse) {
    return { sourceSides: { bottom: 90, left: 30, right: 30 }, targetSides: { top: 90, left: 30, right: 30 } };
  }
  if (edge.role === 'flow' && vertical) {
    return { sourceSides: { top: 60, left: 20, right: 20 }, targetSides: { bottom: 60, left: 20, right: 20 } };
  }
  return {};
}

/** Routes every block edge the drawing has no route for, over the drawing's geometry. */
function routeMissing(drawing, edges, ctx) {
  const missing = edges.filter(e => !drawing.routes.has(e.id) && drawing.boxes.has(e.source.id) && drawing.boxes.has(e.target.id));
  if (!missing.length) return;
  const obstacles = [];
  drawing.boxes.forEach((b, id) => {
    const node = ctx.model.nodes.get(id);
    if (!node) return;
    const ancestors = [];
    for (let p = node.parent; p; p = p.parent) if (drawing.boxes.has(p.id)) ancestors.push(p.id);
    obstacles.push({ id, cx: b.x + b.w / 2, cy: b.y + b.h / 2, w: b.w, h: b.h, ancestors });
  });
  // A fixed work budget (A* expansions) sized by the number of edges keeps routing deterministic;
  // the time limit is only the router's emergency stop.
  const workBudget = Math.max(4000000, missing.length * 6000);
  const routerInput = {
    obstacles,
    edges: missing.map(e => Object.assign({ id: e.id, source: e.source.id, target: e.target.id }, sideCosts(e, ctx)))
  };
  const routerOptions = { margin: 14, bendPenalty: 40, spacing: 10, workBudget, timeBudgetMs: 30000 };
  // Diagnostics only: lets a test harness capture exactly what the router was asked to do.
  if (ctx.diagnostics && typeof ctx.diagnostics.onRouterInput === 'function') ctx.diagnostics.onRouterInput(routerInput, routerOptions);
  const routes = routeOrthogonal(routerInput, routerOptions);
  ctx.stats.routed = (ctx.stats.routed || 0) + routes.size;
  routes.forEach((r, id) => {
    if (!r || !Array.isArray(r.points) || r.points.length < 2) return;
    drawing.routes.set(id, { points: r.points, label: labelOnLongestSegment(r.points) });
  });
}

/** Label centre: the middle of the route's longest segment. */
function labelOnLongestSegment(points) {
  let best = 0;
  let at = null;
  for (let i = 1; i < points.length; i++) {
    const len = Math.abs(points[i].x - points[i - 1].x) + Math.abs(points[i].y - points[i - 1].y);
    if (len > best) {
      best = len;
      at = { x: (points[i].x + points[i - 1].x) / 2, y: (points[i].y + points[i - 1].y) / 2 };
    }
  }
  return at || undefined;
}

/** Candidate configurations for a block, most conventional first. */
function candidateConfigs(stats, ctx) {
  const p = ctx.profile;
  const base = {
    algorithm: 'layered',
    direction: p.directions[0],
    nodePlacement: 'BRANDES_KOEPF',
    thoroughness: thoroughnessFor(stats.n, stats.e, !stats.flat),
    cycleBreaking: p.cycleBreaking,
    modelOrder: p.modelOrder,
    wrapping: false,
    aspectRatio: ctx.aspect,
    edgeLabels: Boolean(p.edgeLabels) && stats.e <= LIMITS.labelMaxEdges,
    spacing: p.spacing,
    desiredEdgeLength: Math.round(stats.meanCard * 0.8 + 60),
    edgePriority
  };
  const stressAllowed = stats.flat && stats.n >= 3 && (p.family === 'auto' || p.family === 'stress');
  const clusterAllowed = !stats.flat && p.family !== 'sequence' && p.family !== 'timing';
  const cluster = Object.assign({}, base, { strategy: 'cluster' });
  const stress = Object.assign({}, base, { algorithm: 'stress' });
  const layered = [];
  for (const dir of p.directions) layered.push(Object.assign({}, base, { direction: dir }));
  if (p.type === 'class' && !stats.hasHierarchy && !p.directions.includes('RIGHT')) {
    layered.push(Object.assign({}, base, { direction: 'RIGHT' }));
  }
  // ELK's network-simplex node placement straightens edges but is pathological on graphs with
  // many long edges (measured: 33 s on a 40-node, 84-edge state machine where Brandes-Koepf
  // took 2 s), so it only competes on small blocks.
  const simplex = stats.n + stats.e <= LIMITS.networkSimplexMaxUnits
    ? p.directions.map(dir => Object.assign({}, base, { direction: dir, nodePlacement: 'NETWORK_SIMPLEX' }))
    : [];
  const compactAlt = clusterAllowed ? [cluster] : (stressAllowed ? [stress] : []);

  // Large or dense blocks try the compact strategy first: ELK layered is slow on them and tends
  // to sprawl. Otherwise the conventional layered drawing goes first and the compact strategy
  // second, so it always competes even when the block's budget allows only two candidates.
  const dense = stats.n > LIMITS.networkSimplexMaxNodes || stats.e > 2.5 * Math.max(1, stats.n);
  let out;
  if (p.family === 'stress' && stressAllowed) out = [stress, ...layered, ...simplex];
  else if (dense && compactAlt.length) out = [...compactAlt, ...layered, ...simplex];
  else out = [layered[0], ...compactAlt, ...layered.slice(1), ...simplex];

  // Tiny blocks have little to choose between.
  if (stats.n <= 3) return out.slice(0, 1);
  if (stats.n <= 8) return out.slice(0, 2);
  if (stats.n > LIMITS.portfolioMaxNodes || stats.e > LIMITS.portfolioMaxEdges) return out.slice(0, 2);
  return out;
}

function describeConfig(c) {
  if (c.strategy === 'cluster') return 'clustered packages + orthogonal routing';
  if (c.algorithm === 'stress') return 'stress + VPSC + orthogonal routing';
  const placement = c.nodePlacement === 'NETWORK_SIMPLEX' ? 'network simplex' : 'Brandes-Koepf';
  return `layered ${c.direction.toLowerCase()} / ${placement}${c.wrapping ? ' / wrapped' : ''}`;
}

function describe(ctx) {
  const s = ctx.stats;
  const winners = [...s.winners.entries()].map(([k, v]) => (v > 1 ? `${k} x${v}` : k)).join(', ');
  const parts = [];
  if (winners) parts.push(winners);
  parts.push(`${s.blocks} block${s.blocks === 1 ? '' : 's'}`);
  if (s.candidates) parts.push(`${s.candidates} candidate${s.candidates === 1 ? '' : 's'} scored`);
  if (s.packedGroups) parts.push(`${s.packedGroups} packed group${s.packedGroups === 1 ? '' : 's'}`);
  return `adaptive: ${parts.join('; ')}`;
}

/** Edges of a stress drawing are drawn straight; parallel edges fan out as bezier curves. */
function markStraightEdges(edges, ctx) {
  const pairCount = new Map();
  const key = (e) => (e.source.id < e.target.id ? `${e.source.id}\u0000${e.target.id}` : `${e.target.id}\u0000${e.source.id}`);
  edges.forEach(e => pairCount.set(key(e), (pairCount.get(key(e)) || 0) + 1));
  edges.forEach(e => ctx.edgeStyles.set(e.id, pairCount.get(key(e)) > 1 ? 'bezier' : 'straight'));
}

function meanCardSize(nodeIds, model) {
  let sum = 0;
  let n = 0;
  nodeIds.forEach(id => {
    const node = model.nodes.get(id);
    if (node && !node.isCompound) {
      sum += (node.w + node.h) / 2;
      n++;
    }
  });
  return n ? sum / n : 180;
}

/** Leaves first (interfaces, abstract types, classes, enums, others), then packages; input order within. */
function sortForPacking(list, nodeOf) {
  const rank = (node) => {
    if (node.isCompound) return 100;
    const k = String(node.kind || '').toLowerCase();
    return KIND_RANK[k] !== undefined ? KIND_RANK[k] : 50;
  };
  return list.slice().sort((a, b) => {
    const na = nodeOf(a);
    const nb = nodeOf(b);
    return rank(na) - rank(nb) || na.index - nb.index;
  });
}

// ── Drawings: { w, h, boxes: Map<id, {x, y, w, h}> (top-left), routes: Map<edgeId, {points, label?}> } ──

function emptyDrawing() {
  return { w: 0, h: 0, boxes: new Map(), routes: new Map() };
}

function leafDrawing(node) {
  return { w: node.w, h: node.h, boxes: new Map([[node.id, { x: 0, y: 0, w: node.w, h: node.h }]]), routes: new Map() };
}

function translateInto(target, source, dx, dy) {
  source.boxes.forEach((b, id) => target.boxes.set(id, { x: b.x + dx, y: b.y + dy, w: b.w, h: b.h }));
  source.routes.forEach((r, id) => {
    target.routes.set(id, {
      points: r.points.map(p => ({ x: p.x + dx, y: p.y + dy })),
      label: r.label ? { x: r.label.x + dx, y: r.label.y + dy } : undefined
    });
  });
}

/** Packs drawings as boxes; the result's bounding box starts at (0, 0). */
function packDrawings(drawings, gap, aspectRatio) {
  if (drawings.length === 0) return emptyDrawing();
  if (drawings.length === 1) return drawings[0];
  const boxes = drawings.map((d, i) => ({ id: String(i), w: Math.max(1, d.w), h: Math.max(1, d.h) }));
  const packedBoxes = packBoxes(boxes, { gapX: gap, gapY: gap, aspectRatio });
  const out = emptyDrawing();
  drawings.forEach((d, i) => {
    const pos = packedBoxes.positions.get(String(i));
    const x = pos ? pos.cx - boxes[i].w / 2 : 0;
    const y = pos ? pos.cy - boxes[i].h / 2 : 0;
    translateInto(out, d, x, y);
  });
  out.w = packedBoxes.width;
  out.h = packedBoxes.height;
  return out;
}

/** Wraps a package around its content: padding on every side, label band on top, label-wide. */
function wrapCompound(node, content) {
  const pad = node.padding;
  const band = node.labelBand;
  const w = Math.max(content.w + 2 * pad, node.minWidth, 2 * pad + 40);
  const h = content.h + 2 * pad + band;
  const out = emptyDrawing();
  translateInto(out, content, (w - content.w) / 2, band + pad);
  out.boxes.set(node.id, { x: 0, y: 0, w, h });
  out.w = w;
  out.h = h;
  return out;
}

/**
 * Moves a drawing so the bounding box of its top-level items, routes and labels starts at
 * (0, 0), and sets w/h to that box.
 */
function normalizeDrawing(drawing, items) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const take = (x1, y1, x2, y2) => {
    minX = Math.min(minX, x1);
    minY = Math.min(minY, y1);
    maxX = Math.max(maxX, x2);
    maxY = Math.max(maxY, y2);
  };
  items.forEach(it => {
    const b = drawing.boxes.get(it.id);
    if (b) take(b.x, b.y, b.x + b.w, b.y + b.h);
  });
  drawing.routes.forEach(r => {
    r.points.forEach(p => take(p.x, p.y, p.x, p.y));
    if (r.label) take(r.label.x - 4, r.label.y - 4, r.label.x + 4, r.label.y + 4);
  });
  if (minX === Infinity) {
    drawing.w = 0;
    drawing.h = 0;
    return drawing;
  }
  const shifted = emptyDrawing();
  translateInto(shifted, drawing, -minX, -minY);
  drawing.boxes = shifted.boxes;
  drawing.routes = shifted.routes;
  drawing.w = maxX - minX;
  drawing.h = maxY - minY;
  return drawing;
}

/**
 * ELK attaches edges to a package's full footprint, whose top includes the label band. The
 * canvas draws the package box below that band, so attachments inside the band move onto the
 * drawn box.
 */
function fixCompoundAttachments(drawing, edges) {
  for (const e of edges) {
    const route = drawing.routes.get(e.id);
    if (!route || route.points.length < 2) continue;
    for (const [node, atStart] of [[e.source, true], [e.target, false]]) {
      if (!node.isCompound || !node.labelBand) continue;
      const box = drawing.boxes.get(node.id);
      if (!box) continue;
      const drawnTop = box.y + node.labelBand;
      const pts = route.points;
      const i = atStart ? 0 : pts.length - 1;
      const j = atStart ? 1 : pts.length - 2;
      const p = pts[i];
      if (p.y >= drawnTop - 0.5 || p.x < box.x - 0.5 || p.x > box.x + box.w + 0.5) continue;
      const q = pts[j];
      if (Math.abs(q.x - p.x) < 0.5) {
        p.y = drawnTop;
      } else {
        const y = Math.min(drawnTop + 8, box.y + box.h - 1);
        p.y = y;
        q.y = y;
      }
    }
  }
}

/** Metrics input from a drawing (compound boxes are full footprints, label band included). */
function metricsInput(drawing, model, edges) {
  const nodes = [];
  drawing.boxes.forEach((b, id) => {
    const node = model.nodes.get(id);
    if (!node) return;
    nodes.push({
      id,
      cx: b.x + b.w / 2,
      cy: b.y + b.h / 2,
      w: b.w,
      h: b.h,
      parent: node.parent && drawing.boxes.has(node.parent.id) ? node.parent.id : null,
      isCompound: node.isCompound
    });
  });
  const edgeList = [];
  for (const e of edges) {
    if (!drawing.boxes.has(e.source.id) || !drawing.boxes.has(e.target.id)) continue;
    const r = drawing.routes.get(e.id);
    edgeList.push({ id: e.id, source: e.source.id, target: e.target.id, points: r && r.points.length >= 2 ? r.points : undefined });
  }
  return { nodes, edges: edgeList };
}

/** Writes positions, package boxes and edge routes onto the Cytoscape elements. */
function writeBack(model, drawing, ctx) {
  const unplaced = [];
  model.nodes.forEach(node => {
    const b = drawing.boxes.get(node.id);
    if (!b) {
      if (!node.isCompound) unplaced.push(node);
      return;
    }
    if (node.isCompound) {
      const band = node.labelBand;
      const box = { cx: b.x + b.w / 2, cy: b.y + band + (b.h - band) / 2, w: b.w, h: Math.max(1, b.h - band) };
      node.el.data.layoutBox = box;
      node.el.position = { x: box.cx, y: box.cy };
    } else {
      node.el.position = { x: b.x + b.w / 2, y: b.y + b.h / 2 };
    }
  });
  // Defensive: anything the layout did not place goes in a row below the drawing.
  if (unplaced.length) {
    ctx.warnings.push(`${unplaced.length} node(s) were not placed by the layout and were set below the drawing.`);
    let x = 0;
    const y = drawing.h + 120;
    unplaced.forEach(node => {
      node.el.position = { x: x + node.w / 2, y: y + node.h / 2 };
      x += node.w + 56;
    });
  }
  // Repeated self loops on one node: Cytoscape grows each further loop (the 300th spans
  // thousands of pixels), so beyond two they draw as one loop labelled with the count.
  const loopsByNode = new Map();
  for (const e of model.edges) {
    if (!e.selfLoop) continue;
    if (!loopsByNode.has(e.source.id)) loopsByNode.set(e.source.id, []);
    loopsByNode.get(e.source.id).push(e);
  }
  loopsByNode.forEach(list => {
    if (list.length <= 2) return;
    list.forEach((e, i) => {
      e.el.data.layoutRoute = i === 0
        ? { style: 'loop-group', count: list.length }
        : { style: 'loop-grouped', representative: list[0].id };
      // The stylesheet hides flagged edges from the moment the canvas is created.
      if (i > 0) e.el.data.layoutEmbedded = true;
    });
  });

  for (const e of model.edges) {
    if (e.selfLoop) continue;
    if (e.mergedInto) {
      // A parallel message drawn by its link's representative (communication notation).
      e.el.data.layoutRoute = { style: 'merged', representative: e.mergedInto.id };
      e.el.data.layoutEmbedded = true;
      continue;
    }
    const r = drawing.routes.get(e.id);
    if (r && r.points && r.points.length >= 2) {
      e.el.data.layoutRoute = { style: 'orthogonal', points: r.points, label: r.label || undefined };
    } else if (ctx.edgeStyles.has(e.id)) {
      e.el.data.layoutRoute = { style: ctx.edgeStyles.get(e.id) };
    }
    if (e.mergedLabel && e.el.data.layoutRoute) e.el.data.layoutRoute.labelText = e.mergedLabel;
  }
}
