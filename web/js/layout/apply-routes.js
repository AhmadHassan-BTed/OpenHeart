/**
 * OpenHeart layout bridge: draws the layout engine's geometry in Cytoscape.
 *
 * The engine (engine.js) leaves two kinds of geometry on the elements:
 *  - data.layoutRoute on edges (ROUTE FORMAT, see engine.js): { style, points?, label? } with
 *    absolute points in the edge's own source-to-target direction.
 *  - data.layoutBox on package (compound) nodes: { cx, cy, w, h }, the box the canvas must draw
 *    for that package, excluding the label band above it.
 * Sequence lifelines add two more (sequence.js): self-message edges carry
 * data.layoutRoute = { style: 'embedded' } because their loops are drawn inside the lifeline card
 * (the edge itself is hidden), and the lifeline carries data.lifelineImage = { w, h, overflow }
 * when its card image is wider than its node box (the image is drawn at full size, unclipped).
 *
 * Routes and boxes are applied as per-element style bypasses, which survive stylesheet replacement:
 *  - A route becomes explicit source/target endpoints plus, when it bends, a 'segments' edge whose
 *    segment points are expressed the way Cytoscape places them (edge-distances: node-position):
 *    point = s + w (t - s) + d N, with s/t the endpoint node centres and N the unit normal
 *    (-u.y, u.x) / |u| of u = t - s. The label is shifted from Cytoscape's natural anchor to the
 *    centre the engine chose.
 *  - A package box becomes 'min-width'/'min-height' with left/right and top/bottom biases.
 *    Cytoscape sizes a compound as its children's bounding box (the inner box) plus padding,
 *    enlarged to the minimum inner size with the extra split by the biases, so the inner target
 *    is the layout box shrunk by the padding on every side.
 *
 * Routes and boxes are valid only for the positions they were computed for. attachRouteInvalidation
 * watches node moves and drops a route when its two endpoints no longer move together (the
 * stylesheet's default edge style takes over), and drops a package box when its children's
 * arrangement changes. Moving a whole package with its contents keeps every route inside it.
 */

const EDGE_ROUTE_PROPERTIES = [
  'curve-style',
  'control-point-step-size',
  'source-endpoint',
  'target-endpoint',
  'edge-distances',
  'segment-weights',
  'segment-distances',
  'text-margin-x',
  'text-margin-y'
];

const COMPOUND_SIZE_PROPERTIES = [
  'min-width',
  'min-width-bias-left',
  'min-width-bias-right',
  'min-height',
  'min-height-bias-top',
  'min-height-bias-bottom'
];

const ROUTE_STALE = '_ohRouteStale';
const ROUTE_ANCHOR = '_ohRouteAnchor';
const BOX_STALE = '_ohBoxStale';
const BOX_ANCHOR = '_ohBoxAnchor';
const INVALIDATION_ATTACHED = '_ohRouteInvalidation';
const MOVE_TOLERANCE = 0.5;

function round(v, digits) {
  const f = 10 ** digits;
  return Math.round(v * f) / f;
}

function isPoint(p) {
  return Boolean(p) && Number.isFinite(p.x) && Number.isFinite(p.y);
}

function midpoint(a, b) {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

function isFiniteBox(bb) {
  return Boolean(bb) && Number.isFinite(bb.x1) && Number.isFinite(bb.y1) && Number.isFinite(bb.x2) && Number.isFinite(bb.y2);
}

/** Centre Cytoscape uses for a node's edges. Compound bounds are refreshed first. */
function nodeCentre(node) {
  if (node.isParent()) {
    try {
      node.boundingBox({ includeLabels: false, includeOverlays: false });
    } catch (_) {
      // The position below is still the best available centre.
    }
  }
  const p = node.position();
  return { x: p.x, y: p.y };
}

/** Padding Cytoscape draws inside a compound, in model pixels. */
function paddingOf(node) {
  try {
    const v = node.pstyle('padding').pfValue;
    return Number.isFinite(v) ? v : 0;
  } catch (_) {
    return 0;
  }
}

/**
 * Bounding box of a compound's children, computed the way Cytoscape sizes the compound:
 * child labels count unless 'compound-sizing-wrt-labels' is 'exclude'.
 */
function childrenBox(node) {
  let includeLabels = true;
  try {
    includeLabels = node.pstyle('compound-sizing-wrt-labels').value !== 'exclude';
  } catch (_) {
    includeLabels = true;
  }
  const bb = node.children().boundingBox({ includeLabels, includeOverlays: false, includeEdges: false });
  return isFiniteBox(bb) ? bb : null;
}

function applyCompoundSizing(node) {
  const box = node.data('layoutBox');
  if (!box || !Number.isFinite(box.cx) || !Number.isFinite(box.cy) || !(box.w > 0) || !(box.h > 0)) return;
  const bb = childrenBox(node);
  if (!bb) return;
  const pad = paddingOf(node);
  // Inner target: the drawn box minus the padding Cytoscape adds around the children.
  const left = box.cx - box.w / 2 + pad;
  const right = box.cx + box.w / 2 - pad;
  const top = box.cy - box.h / 2 + pad;
  const bottom = box.cy + box.h / 2 - pad;
  const style = {};
  const extraLeft = Math.max(0, bb.x1 - left);
  const extraRight = Math.max(0, right - bb.x2);
  if (extraLeft + extraRight > MOVE_TOLERANCE && right - left > 0) {
    const leftPct = round((100 * extraLeft) / (extraLeft + extraRight), 3);
    style['min-width'] = round(right - left, 2);
    style['min-width-bias-left'] = `${leftPct}%`;
    style['min-width-bias-right'] = `${round(100 - leftPct, 3)}%`;
  }
  const extraTop = Math.max(0, bb.y1 - top);
  const extraBottom = Math.max(0, bottom - bb.y2);
  if (extraTop + extraBottom > MOVE_TOLERANCE && bottom - top > 0) {
    const topPct = round((100 * extraTop) / (extraTop + extraBottom), 3);
    style['min-height'] = round(bottom - top, 2);
    style['min-height-bias-top'] = `${topPct}%`;
    style['min-height-bias-bottom'] = `${round(100 - topPct, 3)}%`;
  }
  if (Object.keys(style).length) node.style(style);
  node.scratch(BOX_ANCHOR, { w: bb.x2 - bb.x1, h: bb.y2 - bb.y1 });
}

/**
 * A card image wider than its node box (a sequence lifeline whose self messages are drawn in the
 * card): draw it at its real size, centred, unclipped, and widen the node's bounds so redraws
 * and viewport culling include the overflow. The node box itself (hit testing, layout) keeps
 * its width.
 */
function applyImageOverflow(node) {
  const img = node.data('lifelineImage');
  if (!img || !(img.w > 0) || !(img.h > 0)) return;
  node.style({
    'background-fit': 'none',
    'background-width': round(img.w, 2),
    'background-height': round(img.h, 2),
    'background-position-x': '50%',
    'background-position-y': '50%',
    'background-clip': 'none',
    'background-image-containment': 'over',
    'bounds-expansion': Math.max(0, round(img.overflow || 0, 2))
  });
}

function applyEdgeRoute(edge) {
  const route = edge.data('layoutRoute');
  if (!route || typeof route !== 'object') return;
  if (route.style === 'embedded') {
    // Drawn inside its node's card (a sequence self message at its row). Hiding the edge removes
    // the loop Cytoscape would draw around the node centre. No route anchor is recorded, so
    // route invalidation never re-shows it: the card and its loops move together.
    edge.style('display', 'none');
    return;
  }
  if (route.style === 'merged') {
    // A parallel message: its link's representative lists it (communication notation).
    edge.style('display', 'none');
    return;
  }
  if (route.style === 'loop-grouped') {
    // One of three or more self loops on a node: its group's representative draws them all.
    edge.style('display', 'none');
    return;
  }
  if (route.style === 'loop-group') {
    // Cytoscape grows every further loop on a node; the group draws once, labelled with its size.
    const own = edge.data('label');
    const count = Number.isFinite(route.count) ? route.count : 0;
    const text = `${typeof own === 'string' && own.trim() ? own.trim() : 'self'} (${count} self-calls)`;
    edge.style({ label: text, 'control-point-step-size': 48 });
    return;
  }
  if (edge.scratch(ROUTE_STALE) || edge.isLoop()) return;

  if (route.style === 'bezier') {
    edge.style({ 'curve-style': 'bezier', 'control-point-step-size': 14 });
    return;
  }
  if (route.style !== 'straight' && route.style !== 'orthogonal') return;

  const points = Array.isArray(route.points) ? route.points.filter(isPoint) : [];
  if (points.length < 2) {
    if (route.style === 'straight') edge.style({ 'curve-style': 'straight' });
    return;
  }

  const s = nodeCentre(edge.source());
  const t = nodeCentre(edge.target());
  const first = points[0];
  const last = points[points.length - 1];
  const interior = points.slice(1, -1);
  const bypass = {
    'source-endpoint': `${round(first.x - s.x, 2)}px ${round(first.y - s.y, 2)}px`,
    'target-endpoint': `${round(last.x - t.x, 2)}px ${round(last.y - t.y, 2)}px`
  };

  let anchor;
  if (interior.length === 0) {
    bypass['curve-style'] = 'straight';
    anchor = midpoint(first, last);
  } else {
    const ux = t.x - s.x;
    const uy = t.y - s.y;
    const len = Math.hypot(ux, uy);
    if (len < 1) return;
    const nx = -uy / len;
    const ny = ux / len;
    const weights = [];
    const distances = [];
    for (const p of interior) {
      const rx = p.x - s.x;
      const ry = p.y - s.y;
      // Weights scale with the edge length, so they keep more digits than distances.
      weights.push(round((rx * ux + ry * uy) / (len * len), 6));
      distances.push(round(rx * nx + ry * ny, 3));
    }
    bypass['curve-style'] = 'segments';
    bypass['edge-distances'] = 'node-position';
    bypass['segment-weights'] = weights.join(' ');
    bypass['segment-distances'] = distances.join(' ');
    const k = interior.length;
    anchor = k % 2 === 0 ? midpoint(interior[k / 2 - 1], interior[k / 2]) : interior[(k - 1) / 2];
  }

  if (isPoint(route.label)) {
    bypass['text-margin-x'] = round(route.label.x - anchor.x, 2);
    bypass['text-margin-y'] = round(route.label.y - anchor.y, 2);
  }

  if (typeof route.labelText === 'string') {
    // A merged link lists its messages, one per line.
    bypass.label = route.labelText;
    bypass['text-wrap'] = 'wrap';
  }
  edge.style(bypass);
  edge.scratch(ROUTE_ANCHOR, { sx: s.x, sy: s.y, tx: t.x, ty: t.y });
}

/**
 * Applies every layout route and package box on the current graph. Idempotent: calling it
 * again (for example after a theme change replaces the stylesheet) re-applies the same
 * bypasses and skips routes and boxes already invalidated by a move.
 * @param {object} cy Cytoscape core
 */
export function applyLayoutGeometry(cy) {
  if (!cy) return;

  // Package boxes first, innermost first: an outer package's natural box depends on the inner
  // packages' final sizes, and edge endpoints on packages depend on all of them. No batch here,
  // so each bypass is in effect before the next bounding box is read.
  const compounds = cy.nodes().filter(n => {
    try {
      return n.isParent() && Boolean(n.data('layoutBox')) && !n.scratch(BOX_STALE);
    } catch (_) {
      return false;
    }
  });
  const ordered = compounds.toArray().map(n => ({ n, depth: n.ancestors().length }));
  ordered.sort((a, b) => b.depth - a.depth);
  ordered.forEach(({ n }) => {
    try {
      applyCompoundSizing(n);
    } catch (err) {
      console.warn('[OpenHeart Layout] Could not size package', n.id(), err);
    }
  });

  cy.batch(() => {
    cy.nodes('[lifelineImage]').forEach(node => {
      try {
        applyImageOverflow(node);
      } catch (err) {
        console.warn('[OpenHeart Layout] Could not size card image', node.id(), err);
      }
    });
  });

  cy.batch(() => {
    cy.edges().forEach(edge => {
      try {
        applyEdgeRoute(edge);
      } catch (err) {
        console.warn('[OpenHeart Layout] Could not apply route', edge.id(), err);
      }
    });
  });
}

/**
 * Drops an edge's layout route: the stylesheet's default edge style draws it from now on.
 * @param {object} edge Cytoscape edge
 */
export function clearEdgeRoute(edge) {
  if (!edge || edge.length === 0) return;
  try {
    edge.removeStyle(EDGE_ROUTE_PROPERTIES.join(' '));
  } catch (err) {
    console.warn('[OpenHeart Layout] Could not clear route', edge.id(), err);
  }
  edge.scratch(ROUTE_STALE, true);
  edge.removeScratch(ROUTE_ANCHOR);
}

/**
 * Drops the layout box of a package and of all its ancestors, so each is drawn around its
 * children again. Used when a package's contents change (a move, a collapse or an expand).
 * @param {object} node Cytoscape node
 */
export function clearCompoundSizing(node) {
  if (!node || node.length === 0) return;
  node.union(node.ancestors()).forEach(n => {
    try {
      n.removeStyle(COMPOUND_SIZE_PROPERTIES.join(' '));
    } catch (err) {
      console.warn('[OpenHeart Layout] Could not clear package size', n.id(), err);
    }
    n.scratch(BOX_STALE, true);
    n.removeScratch(BOX_ANCHOR);
  });
}

function routeStillValid(edge) {
  const a = edge.scratch(ROUTE_ANCHOR);
  if (!a) return true;
  const s = nodeCentre(edge.source());
  const t = nodeCentre(edge.target());
  // A route stays valid while both endpoints moved by the same offset (it moves with them).
  return Math.abs((s.x - a.sx) - (t.x - a.tx)) <= MOVE_TOLERANCE &&
    Math.abs((s.y - a.sy) - (t.y - a.ty)) <= MOVE_TOLERANCE;
}

function boxStillValid(node) {
  const a = node.scratch(BOX_ANCHOR);
  if (!a) return true;
  const bb = childrenBox(node);
  if (!bb) return false;
  // The box stays valid while the children kept their arrangement (translated, not reshaped).
  return Math.abs((bb.x2 - bb.x1) - a.w) <= MOVE_TOLERANCE && Math.abs((bb.y2 - bb.y1) - a.h) <= MOVE_TOLERANCE;
}

/**
 * Watches node moves (drags, undo and redo, animations) and drops the routes and package boxes
 * they invalidate. Moves are checked once per tick, after every node of a group drag has moved,
 * so a route inside a package that moves as a whole is kept.
 * @param {object} cy Cytoscape core
 */
export function attachRouteInvalidation(cy) {
  if (!cy || cy.scratch(INVALIDATION_ATTACHED)) return;
  cy.scratch(INVALIDATION_ATTACHED, true);

  let pending = new Set();
  let scheduled = false;

  const flush = () => {
    scheduled = false;
    const ids = pending;
    pending = new Set();
    if (typeof cy.destroyed === 'function' && cy.destroyed()) return;
    try {
      let moved = cy.collection();
      ids.forEach(id => {
        const n = cy.getElementById(id);
        if (n && n.length > 0) moved = moved.union(n);
      });
      if (moved.empty()) return;
      const ancestors = moved.ancestors();
      const affected = moved.union(moved.descendants()).union(ancestors);

      affected.connectedEdges().forEach(edge => {
        if (edge.scratch(ROUTE_STALE) || !edge.scratch(ROUTE_ANCHOR)) return;
        try {
          if (!routeStillValid(edge)) clearEdgeRoute(edge);
        } catch (err) {
          clearEdgeRoute(edge);
        }
      });

      moved.filter(n => n.isParent()).union(ancestors).forEach(n => {
        if (n.scratch(BOX_STALE) || !n.scratch(BOX_ANCHOR)) return;
        try {
          if (!boxStillValid(n)) clearCompoundSizing(n);
        } catch (err) {
          clearCompoundSizing(n);
        }
      });
    } catch (err) {
      console.warn('[OpenHeart Layout] Route invalidation failed', err);
    }
  };

  cy.on('position', 'node', evt => {
    pending.add(evt.target.id());
    if (!scheduled) {
      scheduled = true;
      setTimeout(flush, 0);
    }
  });
}
