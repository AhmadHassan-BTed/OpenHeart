/**
 * OpenHeart layout semantics: what each diagram type means for placement.
 *
 * A profile tells the engine how a diagram should read, the way a person who draws that kind of
 * diagram by hand would place it:
 *  - which layout family fits (layered for flow and hierarchy, stress for undirected
 *    collaboration graphs, dedicated layouts for sequence and timing),
 *  - which flow directions are conventional (inheritance points up, control flow runs down,
 *    actors stand left of their use cases),
 *  - what role each relationship kind plays (a generalization shapes the layering much more
 *    than a dependency does), expressed as ELK edge priorities,
 *  - which nodes are pinned to the first or last layer (entry blocks, initial states, actors;
 *    exit blocks, final states, BDD terminals),
 *  - spacing that suits the card sizes of that diagram.
 * The engine combines a profile with measured graph statistics to choose candidate layouts.
 */

import { SPACING } from './geometry.js';

/** Canonical diagram type names (aliases map onto these). */
const TYPE_ALIASES = Object.freeze({
  use_case: 'usecase',
  state: 'statemachine',
  state_machine: 'statemachine',
  call_graph: 'callgraph',
  interaction_overview: 'interaction'
});

export function normalizeDiagramType(graphType) {
  const t = String(graphType || 'class').toLowerCase().trim();
  return TYPE_ALIASES[t] || t;
}

/**
 * Relationship role from an edge kind. Exact names come from diagrams/manifest.json
 * relationship_types; substring rules cover kinds from other sources (PlantUML, engine IR).
 */
export function relationshipRole(kind, diagramType) {
  const k = String(kind || '').toLowerCase();
  if (FLOW_TYPES.has(diagramType)) return 'flow';
  if (diagramType === 'sequence' || k === 'message') return 'message';
  if (k === 'extends' || k === 'extension' || k.includes('generaliz') || k.includes('inherit')) return 'hierarchy';
  if (k.includes('realiz') || k.includes('implement')) {
    // In component diagrams a realization ties a provided interface to its component:
    // a wiring relation, not a classifier hierarchy.
    return diagramType === 'component' || diagramType === 'deployment' ? 'dependency' : 'hierarchy';
  }
  if (k.includes('compos') || k.includes('aggreg') || k.includes('contain') || k.includes('nest')) return 'composition';
  if (k.includes('assoc') || k === 'link' || k === 'connector' || k.includes('assembly')) return 'association';
  if (k.includes('depend') || k.includes('usage') || k === 'use' || k === 'uses' || k.includes('import') ||
      k.includes('include') || k === 'extend' || k.includes('instantiat') || k.includes('deploy') || k.includes('manifest')) return 'dependency';
  if (k.includes('flow') || k.includes('transition') || k.includes('call') || k.includes('branch')) return 'flow';
  return 'other';
}

/**
 * ELK layered edge priorities per role: direction (keep the edge pointing along the layout
 * direction), shortness (keep its endpoints in near layers), straightness (keep it vertical).
 */
const ROLE_PRIORITY = Object.freeze({
  hierarchy: Object.freeze({ direction: 10, shortness: 3, straightness: 10 }),
  composition: Object.freeze({ direction: 4, shortness: 5, straightness: 3 }),
  association: Object.freeze({ direction: 1, shortness: 3, straightness: 1 }),
  dependency: Object.freeze({ direction: 0, shortness: 1, straightness: 0 }),
  flow: Object.freeze({ direction: 5, shortness: 2, straightness: 4 }),
  message: Object.freeze({ direction: 1, shortness: 1, straightness: 1 }),
  other: Object.freeze({ direction: 1, shortness: 1, straightness: 1 })
});

export function edgePriority(edge) {
  return ROLE_PRIORITY[edge && edge.role] || ROLE_PRIORITY.other;
}

/** Diagram types whose every edge is a flow edge (control, data, call, transition, BDD branch). */
const FLOW_TYPES = new Set(['cfg', 'cdg', 'dfg', 'callgraph', 'statemachine', 'activity', 'interaction', 'robdd']);

const ENTRY_RE = /(^|[^a-z])(entry|start|init|initial|begin)([^a-z]|$)/i;
const EXIT_RE = /(^|[^a-z])(exit|stop|end|final|terminal|return)([^a-z]|$)/i;

function nodeText(node) {
  const d = (node && node.el && node.el.data) || {};
  return `${d.kind || ''} ${d.name || ''} ${d.id || ''}`;
}

function entryLike(node) {
  const d = node.el.data || {};
  if (d.kind === 'entry' || d.kind === 'initial' || d.name === 'start' || d.name === '[*]') return true;
  return ENTRY_RE.test(String(d.id)) || ENTRY_RE.test(String(d.name || ''));
}

function exitLike(node) {
  const d = node.el.data || {};
  if (d.kind === 'exit' || d.kind === 'final' || d.name === 'stop' || d.name === 'end') return true;
  return EXIT_RE.test(String(d.id)) || EXIT_RE.test(String(d.name || ''));
}

/** First/last layer pinning for flow graphs: entries with outgoing flow first, sinks named as exits last. */
function flowLayer(node, deg) {
  if (node.isCompound) return null;
  const d = node.el.data || {};
  if (d.name === '[*]') return deg.out > 0 ? 'first' : (deg.in > 0 ? 'last' : null);
  if (entryLike(node) && deg.out > 0 && !exitLike(node)) return 'first';
  if (exitLike(node) && deg.out === 0 && deg.in > 0) return 'last';
  return null;
}

const BASE_SPACING = Object.freeze({
  nodeNode: SPACING.nodeNode,
  betweenLayers: SPACING.betweenLayers,
  edgeNode: SPACING.edgeNode,
  edgeEdge: SPACING.edgeEdge,
  component: SPACING.component,
  rootPadding: 0,
  overlapGap: SPACING.overlapGap,
  packGap: SPACING.packGap
});

function spacing(scale = 1, overrides = {}) {
  const out = {};
  for (const [k, v] of Object.entries(BASE_SPACING)) out[k] = Math.round(v * scale);
  return Object.assign(out, overrides);
}

/**
 * Layout profile per diagram type.
 * family: 'layered' (ELK layered only), 'auto' (layered and stress compete), 'stress'
 * (stress preferred, layered competes), 'sequence', 'timing'.
 */
const PROFILES = {
  class: {
    family: 'auto', directions: ['DOWN'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: true, spacing: spacing(1, { betweenLayers: 96 })
  },
  composite: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: true, spacing: spacing(1)
  },
  profile: {
    family: 'layered', directions: ['DOWN'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: true, spacing: spacing(1)
  },
  object: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: true, spacing: spacing(1)
  },
  package: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: true, spacing: spacing(1)
  },
  component: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(1)
  },
  deployment: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(1)
  },
  communication: {
    family: 'stress', directions: ['RIGHT', 'DOWN'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    mergeParallelEdges: true,
    reverseHierarchy: false, spacing: spacing(1.1)
  },
  usecase: {
    family: 'layered', directions: ['RIGHT', 'DOWN'], modelOrder: true, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(1, { betweenLayers: 120 }),
    layerOf: (node) => (!node.isCompound && (node.el.data || {}).kind === 'actor' ? 'first' : null)
  },
  activity: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.85), layerOf: flowLayer
  },
  statemachine: {
    family: 'auto', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.9), layerOf: flowLayer
  },
  interaction: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.9), layerOf: flowLayer
  },
  cfg: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.8), layerOf: flowLayer
  },
  cdg: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.8), layerOf: flowLayer
  },
  dfg: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.8), layerOf: flowLayer
  },
  callgraph: {
    family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.9)
  },
  robdd: {
    family: 'layered', directions: ['DOWN'], modelOrder: true, cycleBreaking: 'DEPTH_FIRST', edgeLabels: true,
    reverseHierarchy: false, spacing: spacing(0.7),
    layerOf: (node) => (!node.isCompound && (node.el.data || {}).kind === 'bdd_terminal' ? 'last' : null)
  },
  sequence: { family: 'sequence' },
  timing: { family: 'timing' }
};

const DEFAULT_PROFILE = {
  family: 'auto', directions: ['DOWN', 'RIGHT'], modelOrder: false, cycleBreaking: 'GREEDY', edgeLabels: true,
  reverseHierarchy: true, spacing: spacing(1)
};

/**
 * The layout profile for a diagram type.
 * @returns {{ type: string, family: string, directions: string[], modelOrder: boolean,
 *   cycleBreaking: string, edgeLabels: boolean, reverseHierarchy: boolean, spacing: object,
 *   layerOf: (node: object, degree: { in: number, out: number }) => ('first'|'last'|null) }}
 */
export function profileFor(graphType) {
  const type = normalizeDiagramType(graphType);
  const p = PROFILES[type] || DEFAULT_PROFILE;
  return Object.assign({ type, layerOf: () => null }, DEFAULT_PROFILE, p, { type });
}

/**
 * ELK layered thoroughness (crossing-minimisation restarts) by graph size and density. Measured
 * with elkjs 0.9.3: 48 restarts on a 40-state, 84-transition state machine took 5 s per run, while
 * the crossing count stops improving after about ten; package hierarchies (INCLUDE_CHILDREN)
 * multiply the cost of every restart.
 */
export function thoroughnessFor(nodeCount, edgeCount = nodeCount, hierarchical = false) {
  const size = nodeCount + edgeCount;
  let t;
  if (size <= 40) t = 16;
  else if (size <= 150) t = 10;
  else if (size <= 500) t = 7;
  else if (size <= 1500) t = 4;
  else t = 2;
  return hierarchical ? Math.min(t, 5) : t;
}

export { nodeText };
