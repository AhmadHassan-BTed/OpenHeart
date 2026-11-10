/**
 * OpenHeart sequence and timing layouts.
 *
 * A sequence diagram is not a graph-drawing problem: time runs down the page and every
 * participant owns one vertical lifeline. The layout therefore works on two independent axes.
 *
 *  - Columns (x). Participants are ordered actors first, then by first appearance in the
 *    message sequence, then participants that never exchange a message. A deterministic
 *    adjacent-swap local search then shortens messages: it minimises the sum over messages of
 *    |col(source) - col(target)|, never moves a non-actor left of an actor, and only accepts
 *    strict improvements, so it terminates and keeps first-appearance order on ties.
 *    Lifeline centres are then the longest path over difference constraints
 *    (x[j] - x[i] >= c, always i < j), solved in one forward pass: adjacent cards keep a gap,
 *    every message is wide enough for its label, and a self message leaves room for its loop.
 *  - Rows (y). Each message gets its own row in chronological order (array order, or numeric
 *    sequence prefixes such as "1:", "2.1" when every label carries one). Self messages and
 *    multi-line labels get extra height.
 *
 * Every participant card is re-rendered as a head with a dashed lifeline running the full
 * diagram height, so the node box covers the lifeline and every message attaches inside it.
 * Messages between two lifelines are stored as data.layoutRoute (ROUTE FORMAT in engine.js): a
 * straight horizontal arrow at the message's row.
 *
 * Self messages (a participant calling itself) are drawn inside the lifeline card at their rows:
 * Cytoscape always draws an edge whose source is its target as a loop around the node centre,
 * which on a lifeline the full diagram height lands mid-diagram. The participant carries
 * data.selfMessages = [{ y, label }] (y from the card top) and data.lifelineImage (the card
 * image's real size; it is wider than the node box when loops and labels need room), and each
 * self-message edge gets data.layoutRoute = { style: 'embedded' } and data.layoutEmbedded = true
 * so the canvas hides its loop (the stylesheet hides layoutEmbedded edges from creation).
 * The column after a lifeline with self messages is pushed right far enough for the widest
 * loop and label.
 *
 * Timing diagrams stack their tracks top to bottom in input order.
 */

import { edgeLabelSize, lineCount, nodeSize } from './geometry.js';
import { renderCardSvg, cardFromNodeData } from '../graph-loader.js';
import { selfMessageReach } from '../uml-card-renderer.js';
import { isDarkMode } from '../themes/index.js';

const LIFELINE_KINDS = new Set(['participant', 'actor']);
const ADJACENT_GAP = 64;
const LABEL_CLEARANCE = 32;
/** Clear space between a self message's label and the next lifeline's dashed line. */
const SELF_LOOP_CLEARANCE = 24;
const FIRST_ROW_GAP = 48;
const ROW_ADVANCE = 40;
const SELF_ROW_ADVANCE = 60;
const EXTRA_LINE_ADVANCE = 13;
const LABEL_RAISE = 11;
const BOTTOM_GAP = 48;
const TRACK_GAP = 48;

function isEdge(el) {
  return Boolean(el && el.data && el.data.source !== undefined && el.data.target !== undefined);
}

function parentIdSet(elements) {
  const parents = new Set();
  elements.forEach(el => {
    if (el && el.data && !isEdge(el) && el.data.parent) parents.add(String(el.data.parent));
  });
  return parents;
}

/** Nodes that can be laid out as stand-alone cards: not packages, not compounds, not notices. */
function leafCards(elements) {
  const parents = parentIdSet(elements);
  return elements.filter(el => el && el.data && !isEdge(el) && !el.data.isPackage && !el.data.isNotice &&
    !parents.has(String(el.data.id)));
}

const SEQUENCE_PREFIX = /^\s*(\d+(?:\.\d+)*)(?:\s*[:.)]|\s|$)/;

function sequencePrefix(label) {
  if (typeof label !== 'string') return null;
  const m = label.match(SEQUENCE_PREFIX);
  return m ? m[1].split('.').map(Number) : null;
}

function comparePrefixes(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

function lastLine(text) {
  if (typeof text !== 'string' || !text.trim()) return null;
  const lines = text.split('\n').map(s => s.trim()).filter(Boolean);
  return lines.length ? lines[lines.length - 1] : null;
}

/**
 * Card spec for a participant's lifeline. Engine cards (cardSource 'ir' with a lifeline kind)
 * are rebuilt from their data; any other card (PlantUML-sourced, or an engine card of another
 * kind) is converted to a lifeline card of its own name.
 */
function lifelineCard(data) {
  if (data.cardSource === 'ir' && LIFELINE_KINDS.has(data.kind)) {
    return { card: cardFromNodeData(data), converted: false };
  }
  const kind = data.kind === 'actor' ? 'actor' : 'participant';
  const name = (typeof data.name === 'string' && data.name.trim()) ? data.name.trim()
    : (lastLine(data.textLabel) || String(data.id));
  return {
    card: {
      id: data.id,
      kind,
      name,
      label: name,
      stereotype: `<<${kind}>>`,
      fields: [],
      methods: [],
      instructions: [],
      leafPackage: false,
      nestLevel: data.nestLevel || 0
    },
    converted: true
  };
}

/**
 * Orders participants into columns: actors first, then by first appearance in the message
 * sequence, then participants without messages in input order; then shortens messages with an
 * adjacent-swap local search.
 * @returns {number[]} participant indices, left to right
 */
function orderColumns(participants, messages) {
  const n = participants.length;
  const first = new Array(n).fill(Infinity);
  messages.forEach((m, k) => {
    if (first[m.s] === Infinity) first[m.s] = 2 * k;
    if (first[m.t] === Infinity) first[m.t] = 2 * k + 1;
  });
  const order = participants.map((p, i) => i).sort((a, b) => {
    const pa = participants[a];
    const pb = participants[b];
    if (pa.isActor !== pb.isActor) return pa.isActor ? -1 : 1;
    const ha = first[a] !== Infinity;
    const hb = first[b] !== Infinity;
    if (ha !== hb) return ha ? -1 : 1;
    if (ha && first[a] !== first[b]) return first[a] - first[b];
    return a - b;
  });
  if (n < 3 || messages.length === 0) return order;

  // Weighted neighbour lists: for each participant, the other participant of every non-self
  // message and how many messages they exchange.
  const neighbours = Array.from({ length: n }, () => new Map());
  messages.forEach(m => {
    if (m.s === m.t) return;
    neighbours[m.s].set(m.t, (neighbours[m.s].get(m.t) || 0) + 1);
    neighbours[m.t].set(m.s, (neighbours[m.t].get(m.s) || 0) + 1);
  });
  const col = new Int32Array(n);
  order.forEach((p, k) => { col[p] = k; });

  // Change in total span when the participants at columns k and k + 1 trade places.
  const swapDelta = (a, b, k) => {
    let delta = 0;
    neighbours[a].forEach((count, other) => {
      if (other === b) return;
      delta += (col[other] > k ? -1 : 1) * count; // a moves one column right
    });
    neighbours[b].forEach((count, other) => {
      if (other === a) return;
      delta += (col[other] < k + 1 ? -1 : 1) * count; // b moves one column left
    });
    return delta;
  };

  const maxPasses = 8 * n;
  for (let pass = 0; pass < maxPasses; pass++) {
    let improved = false;
    for (let k = 0; k < n - 1; k++) {
      const a = order[k];
      const b = order[k + 1];
      // A non-actor may never move left of an actor.
      if (participants[a].isActor && !participants[b].isActor) continue;
      if (swapDelta(a, b, k) < 0) {
        order[k] = b;
        order[k + 1] = a;
        col[b] = k;
        col[a] = k + 1;
        improved = true;
      }
    }
    if (!improved) break;
  }
  return order;
}

/**
 * Lays out a sequence diagram: participants as lifelines in a row, one row per message in time
 * order. Participant cards are re-rendered with a dashed lifeline the full diagram height
 * (data.lifelineHeight, data.svgDataUri, data.width, data.height, position); every message
 * between participants gets data.layoutRoute. Other edges get none.
 * @param {object[]} elements Cytoscape element objects for a sequence view (mutated)
 * @param {{ isDark?: boolean }} [options]
 * @returns {object[]} the same array
 */
export function layoutSequence(elements, options = {}) {
  if (!Array.isArray(elements) || elements.length === 0) return elements;
  const isDark = options.isDark !== undefined ? Boolean(options.isDark) : isDarkMode();

  const nodes = leafCards(elements);
  if (nodes.length === 0) return elements;
  const participants = nodes.map(el => {
    const { card, converted } = lifelineCard(el.data);
    const head = renderCardSvg(Object.assign({}, card, { lifelineHeight: 0 }), isDark);
    return {
      el,
      card,
      converted,
      isActor: card.kind === 'actor',
      w: head.width,
      headH: head.height
    };
  });
  const indexById = new Map();
  participants.forEach((p, i) => indexById.set(String(p.el.data.id), i));

  // Messages in chronological order.
  const edges = elements.filter(isEdge);
  const messages = [];
  edges.forEach((el, arrayIndex) => {
    const s = indexById.get(String(el.data.source));
    const t = indexById.get(String(el.data.target));
    delete el.data.layoutEmbedded;
    if (s === undefined || t === undefined) {
      delete el.data.layoutRoute;
      return;
    }
    const label = typeof el.data.label === 'string' ? el.data.label : '';
    const size = edgeLabelSize(label);
    messages.push({
      el,
      s,
      t,
      arrayIndex,
      prefix: sequencePrefix(label),
      labelW: size ? size.w : 0,
      lines: Math.max(1, lineCount(label))
    });
  });
  if (messages.length > 0 && messages.every(m => m.prefix)) {
    messages.sort((a, b) => comparePrefixes(a.prefix, b.prefix) || a.arrayIndex - b.arrayIndex);
  }

  // Columns.
  const order = orderColumns(participants, messages);
  const n = order.length;
  const col = new Int32Array(participants.length);
  order.forEach((p, k) => { col[p] = k; });
  const widths = order.map(p => participants[p].w);

  // Difference constraints x[j] - x[i] >= c, grouped by their right column j.
  const incoming = Array.from({ length: n }, () => []);
  messages.forEach(m => {
    const cs = col[m.s];
    const ct = col[m.t];
    if (cs === ct) {
      // The loop and its label are drawn to the right of the lifeline, below the heads; the next
      // lifeline's dashed line must clear them.
      const reach = selfMessageReach(m.el.data.label);
      if (cs + 1 < n) incoming[cs + 1].push({ i: cs, c: reach + SELF_LOOP_CLEARANCE });
      return;
    }
    const i = Math.min(cs, ct);
    const j = Math.max(cs, ct);
    incoming[j].push({ i, c: m.labelW + LABEL_CLEARANCE });
  });
  const xs = new Float64Array(n);
  xs[0] = widths[0] / 2;
  for (let j = 1; j < n; j++) {
    let x = xs[j - 1] + (widths[j - 1] + widths[j]) / 2 + ADJACENT_GAP;
    for (const { i, c } of incoming[j]) x = Math.max(x, xs[i] + c);
    xs[j] = x;
  }

  // Rows. Extra lines of a label are added above its own row, so it clears the previous message.
  const headH = participants.reduce((m, p) => Math.max(m, p.headH || 60), 0) || 60;
  const rows = new Float64Array(messages.length);
  let y = headH + FIRST_ROW_GAP;
  messages.forEach((m, k) => {
    if (k > 0) {
      const prev = messages[k - 1];
      // A self message's label is centred on its loop, so extra label lines also reach below it.
      y += prev.s === prev.t ? SELF_ROW_ADVANCE + (prev.lines - 1) * EXTRA_LINE_ADVANCE / 2 : ROW_ADVANCE;
    }
    y += (m.lines - 1) * EXTRA_LINE_ADVANCE;
    rows[k] = y;
  });
  const totalHeight = Math.max(headH + 2 * FIRST_ROW_GAP, messages.length ? rows[messages.length - 1] + BOTTOM_GAP : 0);

  // Self messages per participant, at their rows (y from the card top, which is the diagram top).
  const selfByParticipant = new Map();
  messages.forEach((m, k) => {
    if (m.s !== m.t) return;
    if (!selfByParticipant.has(m.s)) selfByParticipant.set(m.s, []);
    selfByParticipant.get(m.s).push({ y: rows[k], label: typeof m.el.data.label === 'string' ? m.el.data.label : '' });
  });

  // Lifeline cards.
  participants.forEach((p, i) => {
    const data = p.el.data;
    const selfMessages = selfByParticipant.get(i) || [];
    const card = Object.assign({}, p.card, { lifelineHeight: totalHeight, selfMessages });
    const svg = renderCardSvg(card, isDark);
    data.lifelineHeight = totalHeight;
    data.svgDataUri = svg.dataUri;
    data.width = svg.width;
    data.height = svg.height;
    if (selfMessages.length) {
      data.selfMessages = selfMessages;
      // The image is wider than the node box (by overflow on each side) to fit loops and labels.
      data.lifelineImage = { w: svg.imageWidth || svg.width, h: svg.imageHeight || svg.height, overflow: svg.overflow || 0 };
    } else {
      delete data.selfMessages;
      delete data.lifelineImage;
    }
    if (p.converted) {
      // The card is now a lifeline card built from this data, so theme restyling (which
      // re-renders cardSource 'ir' cards from their data) keeps the lifeline.
      data.kind = card.kind;
      data.name = card.name;
      data.stereotype = card.stereotype;
      data.textLabel = `${card.stereotype}\n${card.name}`;
      data.cardSource = 'ir';
    }
    p.el.position = { x: xs[col[i]], y: totalHeight / 2 };
  });

  // Message routes.
  messages.forEach((m, k) => {
    const rowY = rows[k];
    const xsrc = xs[col[m.s]];
    const xtgt = xs[col[m.t]];
    const labelRaise = LABEL_RAISE + (m.lines - 1) * EXTRA_LINE_ADVANCE / 2;
    if (m.s === m.t) {
      // Drawn inside the lifeline card (data.selfMessages); the canvas hides the edge's loop.
      // layoutEmbedded is a flat flag the stylesheet hides from the moment the canvas is created.
      m.el.data.layoutRoute = { style: 'embedded' };
      m.el.data.layoutEmbedded = true;
    } else {
      m.el.data.layoutRoute = {
        style: 'straight',
        points: [{ x: xsrc, y: rowY }, { x: xtgt, y: rowY }],
        label: { x: (xsrc + xtgt) / 2, y: rowY - labelRaise }
      };
    }
  });

  return elements;
}

/**
 * Lays out a timing diagram: tracks (non-package cards) stacked top to bottom in input order,
 * left aligned, with a fixed vertical gap. Edges get no layoutRoute.
 * @param {object[]} elements Cytoscape element objects for a timing view (mutated)
 * @param {object} [options] unused; accepted for a uniform strategy signature
 * @returns {object[]} the same array
 */
export function layoutTiming(elements, options = {}) {
  if (!Array.isArray(elements) || elements.length === 0) return elements;
  let top = 0;
  leafCards(elements).forEach(el => {
    const { w, h } = nodeSize(el.data);
    el.position = { x: w / 2, y: top + h / 2 };
    top += h + TRACK_GAP;
  });
  elements.forEach(el => {
    if (isEdge(el)) {
      delete el.data.layoutRoute;
      delete el.data.layoutEmbedded;
    }
  });
  return elements;
}
