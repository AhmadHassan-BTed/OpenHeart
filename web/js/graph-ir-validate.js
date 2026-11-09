/**
 * OpenHeart Graph IR & Diagram State Validator
 * Every IR and saved state is checked before it reaches the renderer:
 *  - structural shape (arrays, objects, string ids),
 *  - unique node and edge ids (duplicates are dropped and reported),
 *  - parents that exist and form no cycles,
 *  - edges whose endpoints exist,
 *  - allow-listed diagram types, relationship kinds, URLs and colours for saved states.
 * The result separates fatal errors (nothing is rendered) from warnings (rendered with
 * the listed repairs, which the studio surfaces to the user).
 */

export const DIAGRAM_STATE_FORMAT = 'OpenHeart-Diagram-IR';
export const DIAGRAM_STATE_VERSION = '1.1.0';

const SAFE_IDENT = /^[A-Za-z_][A-Za-z0-9_]{0,47}$/;
const SAFE_CLASSES = /^[A-Za-z0-9_\- ]{0,400}$/;
const MAX_ELEMENTS = 200000;

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function idOf(raw) {
  if (typeof raw === 'string' && raw.length > 0 && raw.length <= 1024) return raw;
  if (typeof raw === 'number' && Number.isFinite(raw)) return String(raw);
  return null;
}

function summarize(list, limit = 5) {
  const shown = list.slice(0, limit).join(', ');
  return list.length > limit ? `${shown} and ${list.length - limit} more` : shown;
}

/**
 * Accepts https URLs on raw.githubusercontent.com and same-origin relative paths.
 */
export function isAllowedSourceUrl(value) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 2048) return false;
  if (/^[a-z][a-z0-9+.-]*:/i.test(value) || value.startsWith('//')) {
    let url;
    try {
      url = new URL(value);
    } catch (_) {
      return false;
    }
    return url.protocol === 'https:' && url.hostname === 'raw.githubusercontent.com' && !url.username && !url.password;
  }
  return !value.includes('..') && !value.startsWith('\\');
}

/**
 * Validates a Graph IR document as emitted by the Rust exporters or the in-browser extractor.
 * @returns {{ ok: boolean, ir: object|null, errors: string[], warnings: string[] }}
 */
export function validateGraphIr(ir, { expectedType = null } = {}) {
  const errors = [];
  const warnings = [];
  if (!isPlainObject(ir)) {
    return { ok: false, ir: null, errors: ['Graph IR is not a JSON object.'], warnings };
  }
  if (!Array.isArray(ir.nodes)) {
    return { ok: false, ir: null, errors: ['Graph IR has no "nodes" array.'], warnings };
  }
  if (ir.edges !== undefined && !Array.isArray(ir.edges)) {
    errors.push('Graph IR "edges" is present but is not an array.');
    return { ok: false, ir: null, errors, warnings };
  }
  if (ir.nodes.length + (ir.edges ? ir.edges.length : 0) > MAX_ELEMENTS) {
    return { ok: false, ir: null, errors: [`Graph IR has more than ${MAX_ELEMENTS} elements.`], warnings };
  }
  if (expectedType && typeof ir.diagram_type === 'string') {
    const declared = ir.diagram_type.toLowerCase().replace(/_/g, '');
    const expected = expectedType.toLowerCase().replace(/_/g, '');
    const aliases = { state: 'statemachine', usecase: 'usecase', cg: 'callgraph' };
    if ((aliases[declared] || declared) !== (aliases[expected] || expected)) {
      warnings.push(`This file declares diagram_type "${ir.diagram_type}" but was loaded for the "${expectedType}" view; it is not a distinct ${expectedType} projection.`);
    }
  }

  const seen = new Set();
  const duplicates = [];
  let missingIds = 0;
  const nodes = [];
  for (const raw of ir.nodes) {
    if (!isPlainObject(raw)) { missingIds++; continue; }
    const id = idOf(raw.id);
    if (!id) { missingIds++; continue; }
    if (seen.has(id)) { duplicates.push(id); continue; }
    seen.add(id);
    nodes.push(Object.assign({}, raw, { id }));
  }
  if (missingIds) warnings.push(`${missingIds} node(s) without a usable id were dropped.`);
  if (duplicates.length) warnings.push(`${duplicates.length} duplicate node id(s) were dropped (first occurrence kept): ${summarize(duplicates)}.`);

  // Parents must exist and must not form cycles.
  const byId = new Map(nodes.map(n => [n.id, n]));
  let danglingParents = 0;
  nodes.forEach(n => {
    if (n.parent === undefined || n.parent === null || n.parent === '') { n.parent = null; return; }
    const pid = idOf(n.parent);
    if (!pid || !byId.has(pid) || pid === n.id) {
      danglingParents++;
      n.parent = null;
      return;
    }
    n.parent = pid;
  });
  let cycles = 0;
  nodes.forEach(n => {
    const visited = new Set([n.id]);
    let cur = n.parent ? byId.get(n.parent) : null;
    while (cur) {
      if (visited.has(cur.id)) {
        n.parent = null;
        cycles++;
        break;
      }
      visited.add(cur.id);
      cur = cur.parent ? byId.get(cur.parent) : null;
    }
  });
  if (danglingParents) warnings.push(`${danglingParents} node(s) referenced a missing parent; they were detached.`);
  if (cycles) warnings.push(`${cycles} parent cycle(s) were broken.`);

  const edges = [];
  const edgeIds = new Set();
  let dangling = 0;
  let renamed = 0;
  (ir.edges || []).forEach((raw, idx) => {
    if (!isPlainObject(raw)) { dangling++; return; }
    const source = idOf(raw.source);
    const target = idOf(raw.target);
    if (!source || !target || !byId.has(source) || !byId.has(target)) { dangling++; return; }
    let id = idOf(raw.id) || `edge_${idx}_${source}_${target}`;
    if (edgeIds.has(id) || seen.has(id)) {
      let n = 2;
      while (edgeIds.has(`${id}~${n}`) || seen.has(`${id}~${n}`)) n++;
      id = `${id}~${n}`;
      renamed++;
    }
    edgeIds.add(id);
    edges.push(Object.assign({}, raw, { id, source, target }));
  });
  if (dangling) warnings.push(`${dangling} edge(s) pointed at missing nodes and were dropped.`);
  if (renamed) warnings.push(`${renamed} edge(s) had duplicate ids and were renamed.`);

  return {
    ok: true,
    ir: Object.assign({}, ir, { nodes, edges }),
    errors,
    warnings
  };
}

function sanitizeElementData(data, group, allowedEdgeKinds, warnings, counters) {
  const out = {};
  for (const [key, value] of Object.entries(data)) {
    if (typeof key !== 'string' || key.length > 64 || key === '__proto__' || key === 'constructor' || key === 'prototype') continue;
    out[key] = value;
  }
  for (const key of ['raw_url', 'rawUrl']) {
    if (out[key] !== undefined && out[key] !== null && !isAllowedSourceUrl(out[key])) {
      delete out[key];
      counters.urls++;
    }
  }
  if (out.svgDataUri !== undefined && !(typeof out.svgDataUri === 'string' && out.svgDataUri.startsWith('data:image/svg+xml'))) {
    delete out.svgDataUri;
    counters.images++;
  }
  ['width', 'height', 'origWidth', 'origHeight'].forEach(k => {
    if (out[k] !== undefined && !(typeof out[k] === 'number' && Number.isFinite(out[k]) && out[k] >= 0 && out[k] < 1e6)) delete out[k];
  });
  if (group === 'edges') {
    const kind = out.uml_kind;
    if (kind !== undefined) {
      if (typeof kind !== 'string' || !SAFE_IDENT.test(kind)) {
        out.uml_kind = 'association';
        counters.kinds++;
      } else if (allowedEdgeKinds && !allowedEdgeKinds.has(kind)) {
        counters.unknownKinds.add(kind);
      }
    }
  }
  return out;
}

/**
 * Validates a saved .openheart.json diagram state.
 * @param {object} state
 * @param {{ allowedDiagramTypes: Set<string>, allowedEdgeKinds: Set<string> }} allow
 * @returns {{ ok: boolean, state: object|null, errors: string[], warnings: string[] }}
 */
export function validateDiagramState(state, { allowedDiagramTypes, allowedEdgeKinds }) {
  const errors = [];
  const warnings = [];
  if (!isPlainObject(state)) {
    return { ok: false, state: null, errors: ['The file is not a JSON object.'], warnings };
  }
  if (state.format !== undefined && state.format !== DIAGRAM_STATE_FORMAT) {
    errors.push(`Unsupported format "${String(state.format).slice(0, 40)}" (expected ${DIAGRAM_STATE_FORMAT}).`);
  }
  const version = typeof state.version === 'string' ? state.version : null;
  const major = version ? parseInt(version.split('.')[0], 10) : NaN;
  if (!version || major !== 1) {
    errors.push(`Unsupported state version "${version === null ? 'missing' : String(version).slice(0, 20)}" (this studio reads version 1.x).`);
  }
  const diagramType = typeof state.diagram_type === 'string' ? state.diagram_type : null;
  if (!diagramType || !allowedDiagramTypes.has(diagramType)) {
    errors.push(`Unknown diagram_type "${diagramType === null ? 'missing' : diagramType.slice(0, 40)}".`);
  }
  if (!Array.isArray(state.elements)) {
    errors.push('The state has no "elements" array.');
  } else if (state.elements.length > MAX_ELEMENTS) {
    errors.push(`The state has more than ${MAX_ELEMENTS} elements.`);
  }
  if (errors.length) return { ok: false, state: null, errors, warnings };

  const counters = { urls: 0, images: 0, kinds: 0, unknownKinds: new Set() };
  const nodeEls = [];
  const edgeEls = [];
  const seen = new Set();
  const duplicates = [];
  let malformed = 0;
  for (const raw of state.elements) {
    if (!isPlainObject(raw) || !isPlainObject(raw.data)) { malformed++; continue; }
    const group = raw.group === 'edges' || (raw.group === undefined && raw.data.source !== undefined) ? 'edges' : 'nodes';
    const id = idOf(raw.data.id);
    if (!id) { malformed++; continue; }
    if (seen.has(id)) { duplicates.push(id); continue; }
    seen.add(id);
    const data = sanitizeElementData(raw.data, group, allowedEdgeKinds, warnings, counters);
    data.id = id;
    const el = { group, data };
    if (group === 'nodes' && isPlainObject(raw.position) && Number.isFinite(raw.position.x) && Number.isFinite(raw.position.y)) {
      el.position = { x: raw.position.x, y: raw.position.y };
    }
    if (typeof raw.classes === 'string' && SAFE_CLASSES.test(raw.classes)) el.classes = raw.classes;
    (group === 'nodes' ? nodeEls : edgeEls).push(el);
  }
  if (malformed) warnings.push(`${malformed} malformed element(s) were dropped.`);
  if (duplicates.length) warnings.push(`${duplicates.length} duplicate element id(s) were dropped: ${summarize(duplicates)}.`);
  if (counters.urls) warnings.push(`${counters.urls} source URL(s) outside raw.githubusercontent.com or this site were removed.`);
  if (counters.images) warnings.push(`${counters.images} non-SVG card image(s) were removed.`);
  if (counters.kinds) warnings.push(`${counters.kinds} relationship kind(s) with invalid names were reset to "association".`);
  if (counters.unknownKinds.size) warnings.push(`Relationship kinds not in the manifest: ${summarize(Array.from(counters.unknownKinds))}.`);

  const nodeIds = new Set(nodeEls.map(n => n.data.id));
  let detached = 0;
  nodeEls.forEach(n => {
    if (n.data.parent !== undefined && n.data.parent !== null) {
      const pid = idOf(n.data.parent);
      if (!pid || !nodeIds.has(pid) || pid === n.data.id) {
        delete n.data.parent;
        detached++;
      } else {
        n.data.parent = pid;
      }
    }
  });
  const byId = new Map(nodeEls.map(n => [n.data.id, n]));
  let cycles = 0;
  nodeEls.forEach(n => {
    const visited = new Set([n.data.id]);
    let cur = n.data.parent ? byId.get(n.data.parent) : null;
    while (cur) {
      if (visited.has(cur.data.id)) { delete n.data.parent; cycles++; break; }
      visited.add(cur.data.id);
      cur = cur.data.parent ? byId.get(cur.data.parent) : null;
    }
  });
  if (detached) warnings.push(`${detached} node(s) referenced a missing parent and were detached.`);
  if (cycles) warnings.push(`${cycles} parent cycle(s) were broken.`);

  const validEdges = edgeEls.filter(e => {
    const s = idOf(e.data.source);
    const t = idOf(e.data.target);
    if (!s || !t || !nodeIds.has(s) || !nodeIds.has(t)) return false;
    e.data.source = s;
    e.data.target = t;
    return true;
  });
  if (validEdges.length !== edgeEls.length) warnings.push(`${edgeEls.length - validEdges.length} edge(s) with missing endpoints were dropped.`);

  const zoom = Number.isFinite(state.zoom) && state.zoom >= 0.04 && state.zoom <= 5 ? state.zoom : null;
  const pan = isPlainObject(state.pan) && Number.isFinite(state.pan.x) && Number.isFinite(state.pan.y) ? { x: state.pan.x, y: state.pan.y } : null;
  const filters = Array.isArray(state.active_filters)
    ? state.active_filters.filter(f => typeof f === 'string' && SAFE_IDENT.test(f))
    : [];
  const codebaseName = typeof state.codebase_name === 'string' ? state.codebase_name.slice(0, 200) : null;

  return {
    ok: true,
    state: {
      format: DIAGRAM_STATE_FORMAT,
      version,
      diagram_type: diagramType,
      codebase_name: codebaseName,
      zoom,
      pan,
      pan_locked: state.pan_locked === true,
      active_filters: filters,
      elements: [...nodeEls, ...validEdges]
    },
    errors,
    warnings
  };
}
