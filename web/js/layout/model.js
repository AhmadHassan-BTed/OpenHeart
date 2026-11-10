/**
 * OpenHeart layout model: the hierarchical graph the layout engine works on, built from the
 * Cytoscape elements of one diagram.
 *
 *  - Nodes form a containment tree (packages, partitions and composite parts are compounds;
 *    cards are leaves). Parent cycles and dangling parent references are cut.
 *  - Root package chains that carry nothing of their own (one sub-package, no cards, no edges)
 *    are pruned, so a "com > enterprise > system" chain draws as one package.
 *  - Every compound carries the padding, label band and label width the stylesheet draws
 *    (geometry.js), so the space reserved equals the box drawn.
 *  - Every edge carries its semantic role (semantics.js), whether it is laid out reversed
 *    (generalizations point up), its label size, and whether it is a self loop or a nested
 *    edge (one endpoint contains the other).
 *  - crossCount(X) counts the edges that cross the boundary of X's subtree; the engine uses it
 *    to separate connected parts from detached ones, which are laid out on their own and packed.
 */

import {
  nodeSize,
  packagePadding,
  packageLabelBand,
  packageLabelWidth,
  edgeLabelSize
} from './geometry.js';
import { relationshipRole } from './semantics.js';

/** Padding for compounds that are not packages (composite parts, superstates). */
const NON_PACKAGE_COMPOUND_PADDING = 24;

function isEdgeElement(el) {
  return Boolean(el && el.data && el.data.source !== undefined && el.data.target !== undefined);
}

/**
 * Builds the layout model.
 * @param {object[]} elements Cytoscape elements (mutated only when root chains are pruned)
 * @param {object} profile layout profile from semantics.profileFor
 * @returns {{ type: string, profile: object, elements: object[], nodes: Map<string, object>,
 *   roots: object[], edges: object[], leafCount: number }}
 */
export function buildModel(elements, profile) {
  const type = profile.type;
  const nodeEls = [];
  const edgeEls = [];
  for (const el of elements || []) {
    if (!el || !el.data || el.data.id === undefined || el.data.id === null) continue;
    if (isEdgeElement(el)) edgeEls.push(el);
    else nodeEls.push(el);
  }

  const byId = new Map();
  nodeEls.forEach(el => {
    const id = String(el.data.id);
    if (!byId.has(id)) byId.set(id, el);
  });

  // Parent links, with dangling references and cycles cut.
  const parentOf = new Map();
  byId.forEach((el, id) => {
    const p = el.data.parent !== undefined && el.data.parent !== null ? String(el.data.parent) : null;
    if (p && p !== id && byId.has(p)) parentOf.set(id, p);
  });
  byId.forEach((_, id) => {
    const seen = new Set([id]);
    let cur = parentOf.get(id);
    while (cur) {
      if (seen.has(cur)) {
        parentOf.delete(id);
        break;
      }
      seen.add(cur);
      cur = parentOf.get(cur);
    }
  });

  const childIds = new Map();
  byId.forEach((_, id) => childIds.set(id, []));
  byId.forEach((_, id) => {
    const p = parentOf.get(id);
    if (p) childIds.get(p).push(id);
  });

  // Prune root chains: a root compound with exactly one child, which is itself a compound,
  // with no edge attached, is dropped and its child becomes a root.
  const edgeEndpoints = new Set();
  edgeEls.forEach(el => {
    edgeEndpoints.add(String(el.data.source));
    edgeEndpoints.add(String(el.data.target));
  });
  const pruned = new Set();
  byId.forEach((el, id) => {
    if (parentOf.has(id)) return;
    let cur = id;
    for (;;) {
      const kids = childIds.get(cur);
      if (!kids || kids.length !== 1) break;
      const only = kids[0];
      if ((childIds.get(only) || []).length === 0) break; // the single child is a card, keep the package
      if (edgeEndpoints.has(cur)) break;
      pruned.add(cur);
      parentOf.delete(only);
      const onlyEl = byId.get(only);
      if (onlyEl && onlyEl.data) onlyEl.data.parent = undefined;
      cur = only;
    }
  });

  const keptElements = pruned.size
    ? elements.filter(el => !(el && el.data && !isEdgeElement(el) && pruned.has(String(el.data.id))))
    : elements;

  // Nodes in input order.
  const nodes = new Map();
  let index = 0;
  nodeEls.forEach(el => {
    const id = String(el.data.id);
    if (pruned.has(id) || nodes.has(id)) return;
    const d = el.data;
    const isCompound = (childIds.get(id) || []).some(c => !pruned.has(c));
    const nestLevel = Number.isFinite(d.nestLevel) ? d.nestLevel : 0;
    const size = isCompound ? { w: 0, h: 0 } : nodeSize(d);
    const isPackage = Boolean(d.isPackage);
    const label = d.label || d.textLabel || d.name || id;
    nodes.set(id, {
      id,
      el,
      index: index++,
      kind: String(d.kind || (isPackage ? 'package' : 'class')),
      name: String(d.name || id),
      label: String(label),
      w: size.w,
      h: size.h,
      isCompound,
      isPackage,
      parent: null,
      children: [],
      depth: 0,
      nestLevel,
      padding: isCompound ? (isPackage ? packagePadding(nestLevel) : NON_PACKAGE_COMPOUND_PADDING) : 0,
      labelBand: isCompound && isPackage ? packageLabelBand(nestLevel) : 0,
      minWidth: isCompound && isPackage ? packageLabelWidth(label, nestLevel) : 0,
      layer: null,
      crossCount: 0,
      degree: { in: 0, out: 0 }
    });
  });

  nodes.forEach(node => {
    const p = parentOf.get(node.id);
    if (p && nodes.has(p)) {
      node.parent = nodes.get(p);
      node.parent.children.push(node);
    }
  });
  const roots = [];
  nodes.forEach(node => {
    if (!node.parent) roots.push(node);
  });
  // Depth by walking down from the roots (parents are acyclic now).
  const stack = roots.map(r => [r, 0]);
  while (stack.length) {
    const [n, depth] = stack.pop();
    n.depth = depth;
    n.children.forEach(c => stack.push([c, depth + 1]));
  }

  // Edges.
  const edges = [];
  let edgeIndex = 0;
  edgeEls.forEach(el => {
    const d = el.data;
    const source = nodes.get(String(d.source));
    const target = nodes.get(String(d.target));
    if (!source || !target) return;
    const kind = String(d.uml_kind || d.kind || 'association');
    const role = relationshipRole(kind, type);
    const selfLoop = source === target;
    const nested = !selfLoop && (isAncestor(source, target) || isAncestor(target, source));
    const label = typeof d.label === 'string' ? d.label : '';
    const edge = {
      id: String(d.id),
      el,
      index: edgeIndex++,
      source,
      target,
      kind,
      role,
      reverse: role === 'hierarchy' && Boolean(profile.reverseHierarchy),
      label,
      labelSize: edgeLabelSize(label),
      selfLoop,
      nested
    };
    edges.push(edge);
  });

  // Communication notation: one link per pair of participants carries all their messages, so
  // parallel messages merge into the first one (its label lists them, numbered in order).
  if (profile.mergeParallelEdges) mergeParallel(edges);

  edges.forEach(edge => {
    edge.drawable = !edge.selfLoop && !edge.nested && !edge.mergedInto;
    if (!edge.selfLoop && !edge.mergedInto) {
      edge.source.degree.out++;
      edge.target.degree.in++;
    }
    if (edge.drawable) markCrossings(edge);
  });

  // Semantic layer pinning (entries, exits, actors, terminals).
  if (typeof profile.layerOf === 'function') {
    nodes.forEach(node => {
      if (!node.isCompound) node.layer = profile.layerOf(node, node.degree) || null;
    });
  }

  let leafCount = 0;
  nodes.forEach(n => {
    if (!n.isCompound) leafCount++;
  });

  return { type, profile, elements: keptElements, nodes, roots, edges, leafCount };
}

const MERGED_LABEL_LINES = 8;

/**
 * Merges parallel edges (same unordered pair of endpoints) into their first edge. The first edge
 * gets mergedLabel (its messages in order, one per line, at most MERGED_LABEL_LINES, then a
 * "+N more" line) and a label size to match; the others get mergedInto.
 */
function mergeParallel(edges) {
  const groups = new Map();
  for (const e of edges) {
    if (e.selfLoop || e.nested) continue;
    const key = e.source.id < e.target.id ? `${e.source.id}\u0000${e.target.id}` : `${e.target.id}\u0000${e.source.id}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(e);
  }
  groups.forEach(list => {
    if (list.length < 2) return;
    const rep = list[0];
    const labels = list.map(e => {
      const text = (e.label || '').trim() || e.kind;
      // Direction matters in a message list: mark messages that run against the link.
      return e.source === rep.source ? `→ ${text}` : `← ${text}`;
    });
    const shown = labels.slice(0, MERGED_LABEL_LINES);
    if (labels.length > MERGED_LABEL_LINES) shown.push(`+${labels.length - MERGED_LABEL_LINES} more`);
    rep.mergedLabel = shown.join('\n');
    rep.labelSize = edgeLabelSize(rep.mergedLabel);
    for (let i = 1; i < list.length; i++) list[i].mergedInto = rep;
  });
}

/** True when a is a strict ancestor of b. */
export function isAncestor(a, b) {
  let cur = b ? b.parent : null;
  while (cur) {
    if (cur === a) return true;
    cur = cur.parent;
  }
  return false;
}

/** Lowest common ancestor of two nodes (null when they share no ancestor, i.e. the root). */
export function lowestCommonAncestor(a, b) {
  let x = a;
  let y = b;
  while (x && y && x.depth > y.depth) x = x.parent;
  while (x && y && y.depth > x.depth) y = y.parent;
  while (x && y && x !== y) {
    x = x.parent;
    y = y.parent;
  }
  return x && y && x === y ? x : null;
}

/** The ancestor-or-self of node whose parent is container (container null = the root). */
export function childOfContainer(node, container) {
  let cur = node;
  while (cur && cur.parent !== container) cur = cur.parent;
  return cur || null;
}

/** Every node of a subtree, root first. */
export function subtreeNodes(node, out = []) {
  const stack = [node];
  while (stack.length) {
    const n = stack.pop();
    out.push(n);
    for (let i = n.children.length - 1; i >= 0; i--) stack.push(n.children[i]);
  }
  return out;
}

/**
 * Marks every node whose subtree boundary the edge crosses: the nodes on the paths from each
 * endpoint up to (not including) the lowest common ancestor.
 */
function markCrossings(edge) {
  const lca = lowestCommonAncestor(edge.source, edge.target);
  for (const end of [edge.source, edge.target]) {
    let cur = end;
    while (cur && cur !== lca) {
      cur.crossCount++;
      cur = cur.parent;
    }
  }
}
