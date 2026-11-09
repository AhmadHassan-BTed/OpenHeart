/**
 * OpenHeart Precision Interactive Graph Canvas (Deterministic UML Edition)
 * Strictly enforces UML 2.5 standard symbolic notation:
 *  - 3-Level Distinct Color Hierarchy:
 *      Level 1: Domain Tier Container (Solid border, soft pastel backdrop)
 *      Level 2: Subpackage Container (Dashed border, saturated pastel backdrop)
 *      Level 3: Enclosed 3-Compartment Class Cards (Crisp pure white with drop shadow)
 *  - Deterministic preset layouts (see uml-layout.js)
 *  - Orthogonal taxi wiring
 * Diagram data comes from exactly one session source (studio-store.js): bundled sample files,
 * the Rust analysis server, or the in-browser declaration extractor. Views the active source
 * cannot provide render an explicit empty state; nothing is synthesised.
 */

import { parsePumlToCytoscape } from './puml-parser.js';
import { computeDeterministicLayout } from './uml-layout.js';
import { loadGraphIrToCytoscape, renderCardSvg, cardFromNodeData } from './graph-loader.js';
import { getTheme, onThemeChange, buildCytoscapeStylesheet, isDarkMode, getCachedManifest } from './themes/index.js';
import { MinimapNavigator } from './minimap-navigator.js';
import { validateGraphIr, DIAGRAM_STATE_FORMAT, DIAGRAM_STATE_VERSION } from './graph-ir-validate.js';
import { studioStore } from './studio-store.js';
import { localStore } from './storage.js';

const LOD_ZOOM_THRESHOLD = 0.35;
const LARGE_GRAPH_ELEMENTS = 1000;
const HIT_CELL_SIZE = 400;
const MAX_EXPORT_DIMENSION = 16000;
const MAX_EXPORT_PIXELS = 120e6;
const DEFAULT_HUD_HINT = 'Mouse wheel to Zoom · Drag to Pan · Shift+Wheel to Pan H · Hover to trace path';

function formatParameter(p) {
  if (!p) return '_';
  if (p.name && p.type) return `${p.name}: ${p.type}`;
  return p.name || p.type || '_';
}

export class InteractiveGraphCanvas {
  constructor(containerId = 'interactive-canvas') {
    this.containerId = containerId;
    this.cy = null;
    this.currentGraphType = 'class';
    this.selectedNode = null;
    this.onNodeSelectedCallback = null;
    this.onNodeHoverCallback = null;
    this.onRenderCompleteCallback = null;
    this.onAnnounceCallback = null;
    this.panSensitivity = localStore.getNumber('openheart_pan_sensitivity', 0.10);
    this.activeHoverId = null;
    this.hoverTimeout = null;
    this.collapsedPackages = new Set();
    this.onLayersUpdateCallback = null;
    this.hiddenEdgeKinds = new Set();
    this.isPanLocked = false;
    this.domListenersBound = false;
    this.minimap = new MinimapNavigator(this);
    this.undoStack = [];
    this.redoStack = [];
    this.maxHistory = 50;
    this._hudStatusTimer = null;
    this.renderSeq = 0;
    this.lastRenderMeta = null;
    this.lodActive = false;
    this.hitIndex = null;
    this.snapshotLabel = null;
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.bindContainerDomListeners(container);
    this.bindHistoryControls();
    if (this.minimap) {
      this.minimap.mount(this.containerId);
    }
    onThemeChange((theme, isDark) => {
      this.setTheme(isDark);
    });
    return this.renderGraph(this.currentGraphType);
  }

  setAnnounceCallback(cb) {
    this.onAnnounceCallback = cb;
  }

  announce(message) {
    if (this.onAnnounceCallback) this.onAnnounceCallback(message);
  }

  /**
   * True while keyboard focus is on the diagram canvas itself; single-key shortcuts
   * (Delete, Space, arrows, M, L, N) only act then (WCAG 2.1.4).
   */
  isCanvasFocused() {
    const container = document.getElementById(this.containerId);
    const active = document.activeElement;
    return Boolean(container && active && (active === container || container.contains(active)));
  }

  isFocusInCanvasCard() {
    const container = document.getElementById(this.containerId);
    const card = container ? container.closest('.canvas-card') : null;
    const active = document.activeElement;
    return Boolean(card && active && card.contains(active));
  }

  setNodeSelectCallback(cb) {
    this.onNodeSelectedCallback = cb;
  }

  setNodeHoverCallback(cb) {
    this.onNodeHoverCallback = cb;
  }

  setRenderCompleteCallback(cb) {
    this.onRenderCompleteCallback = cb;
  }

  setPanSensitivity(val) {
    this.panSensitivity = val;
    localStore.set('openheart_pan_sensitivity', val.toString());
  }

  diagramDisplayName(graphType) {
    const manifest = getCachedManifest();
    for (const cat of (manifest && manifest.categories) || []) {
      const hit = (cat.diagrams || []).find(d => d.id === graphType);
      if (hit) return hit.name;
    }
    return graphType;
  }

  /**
   * Projections the in-browser declaration extractor can derive truthfully:
   * the class model itself, and packages with dependencies aggregated from resolved relations.
   * Every other view needs data only the Rust engine computes and gets an explicit empty state.
   */
  projectInBrowserDiagram(ir, graphType) {
    const normType = (graphType || 'class').toLowerCase().replace(/_/g, '');
    if (normType === 'class') {
      return { elements: loadGraphIrToCytoscape(ir), emptyReason: null };
    }
    if (normType === 'package') {
      const classToPkg = new Map();
      ir.nodes.forEach(n => {
        if (!n.is_package && n.parent) classToPkg.set(n.id, n.parent);
      });
      const pkgNodes = ir.nodes.filter(n => n.is_package).map(p => Object.assign({}, p, { parent: null }));
      const counts = new Map();
      (ir.edges || []).forEach(e => {
        const s = classToPkg.get(e.source);
        const t = classToPkg.get(e.target);
        if (s && t && s !== t) {
          const key = `${s}\u0000${t}`;
          counts.set(key, (counts.get(key) || 0) + 1);
        }
      });
      const pkgEdges = Array.from(counts.entries()).map(([key, n]) => {
        const [s, t] = key.split('\u0000');
        return { id: `pkgdep:${s}->${t}`, source: s, target: t, kind: 'dependency', label: `${n} relation${n === 1 ? '' : 's'}`, arrow: '..>' };
      });
      return {
        elements: loadGraphIrToCytoscape({ diagram_type: 'package', nodes: pkgNodes, edges: pkgEdges }),
        emptyReason: pkgNodes.length ? null : 'No packages were recovered from the analyzed files.'
      };
    }
    return {
      elements: [],
      emptyReason: `${this.diagramDisplayName(graphType)} is not available from in-browser analysis. The in-browser extractor recovers type declarations, members and type relations only; control flow, data flow, call graphs, behaviour and deployment views require the OpenHeart Rust engine (run "openheart server" and analyze there).`
    };
  }

  async fetchGraphIrElements(url, graphType) {
    let res;
    try {
      res = await fetch(url, { cache: 'no-cache' });
    } catch (err) {
      return { elements: null, warnings: [], reason: `Could not fetch ${url}: ${(err && err.message) || err}.`, metadata: null };
    }
    if (!res.ok) return { elements: null, warnings: [], reason: `${url} returned HTTP ${res.status}.`, metadata: null };
    let json;
    try {
      json = await res.json();
    } catch (_) {
      return { elements: null, warnings: [], reason: `${url} is not valid JSON.`, metadata: null, fatal: true };
    }
    const v = validateGraphIr(json, { expectedType: graphType });
    if (!v.ok) {
      return { elements: null, warnings: v.errors, reason: `Rejected ${url}: ${v.errors.join(' ')}`, metadata: null, fatal: true };
    }
    return { elements: loadGraphIrToCytoscape(v.ir), warnings: v.warnings, reason: null, metadata: v.ir.metadata || null, ir: v.ir };
  }

  async fetchPumlElements(url, graphType) {
    try {
      const res = await fetch(url, { cache: 'no-cache' });
      if (!res.ok) return null;
      const text = await res.text();
      return { elements: parsePumlToCytoscape(text, graphType), text };
    } catch (_) {
      return null;
    }
  }

  /**
   * Resolves the elements for a view from the active session source.
   * @returns {Promise<{ elements: object[], warnings: string[], emptyReason: string|null, origin: string, metadata: object|null, ir: object|null }>}
   */
  async resolveDiagramElements(graphType) {
    const session = studioStore.get();
    const kind = session.source.kind;
    const safeType = encodeURIComponent(graphType);

    if (session.ir && (kind === 'github' || kind === 'folder' || kind === 'zip')) {
      const { elements, emptyReason } = this.projectInBrowserDiagram(session.ir, graphType);
      return { elements, warnings: [], emptyReason, origin: 'in-browser', metadata: session.ir.metadata || null, ir: graphType === 'class' ? session.ir : null };
    }

    if (kind === 'engine' && session.engine) {
      const engine = session.engine;
      if (engine.artifact_base_url) {
        const base = engine.artifact_base_url.replace(/\/+$/, '');
        const r = await this.fetchGraphIrElements(`${base}/${safeType}.json`, graphType);
        if (r.elements) return { elements: r.elements, warnings: r.warnings, emptyReason: r.elements.length ? null : `The engine produced an empty ${graphType} diagram.`, origin: 'engine', metadata: r.metadata, ir: r.ir };
        if (r.fatal) return { elements: [], warnings: r.warnings, emptyReason: r.reason, origin: 'engine', metadata: null, ir: null };
      }
      const puml = engine.puml_by_type ? engine.puml_by_type[graphType] : null;
      if (typeof puml === 'string' && puml.trim()) {
        const elements = parsePumlToCytoscape(puml, graphType);
        return {
          elements,
          warnings: [],
          emptyReason: elements.length ? null : `The engine's ${graphType} PlantUML contained no drawable elements.`,
          origin: 'engine',
          metadata: null,
          ir: null,
          pumlText: puml
        };
      }
      return {
        elements: [],
        warnings: [],
        emptyReason: `The analysis server's response for this session did not include a "${this.diagramDisplayName(graphType)}" diagram.`,
        origin: 'engine',
        metadata: null,
        ir: null
      };
    }

    const r = await this.fetchGraphIrElements(`diagrams/${safeType}.json`, graphType);
    if (r.elements && r.elements.length) {
      return { elements: r.elements, warnings: r.warnings, emptyReason: null, origin: 'bundled', metadata: r.metadata, ir: r.ir };
    }
    if (!r.fatal) {
      const puml = await this.fetchPumlElements(`diagrams/${safeType}.puml`, graphType);
      if (puml && puml.elements.length) {
        return { elements: puml.elements, warnings: r.warnings, emptyReason: null, origin: 'bundled', metadata: null, ir: null, pumlText: puml.text };
      }
    }
    return {
      elements: [],
      warnings: r.warnings,
      emptyReason: r.reason || `No bundled sample exists for "${this.diagramDisplayName(graphType)}".`,
      origin: 'bundled',
      metadata: null,
      ir: null
    };
  }

  emptyStateElements(graphType, reason) {
    const text = `${this.diagramDisplayName(graphType)}\n\n${reason}`;
    return [{
      data: {
        id: '__openheart_empty_state__',
        label: text,
        name: this.diagramDisplayName(graphType),
        kind: 'notice',
        isNotice: true,
        width: 460,
        height: 170,
        file: null,
        lines: []
      },
      classes: 'empty-state'
    }];
  }

  /**
   * Renders a view. Overlapping calls are ordered by a monotonically increasing render id:
   * only the most recent call commits, so a slow earlier fetch never replaces a later choice.
   * @param {string} graphType
   * @param {{ elements?: object[], preservePositions?: boolean, origin?: string }} options
   * @returns {Promise<boolean>} true when this call committed its render
   */
  async renderGraph(graphType, options = {}) {
    const renderId = ++this.renderSeq;
    const container = document.getElementById(this.containerId);
    if (!container) return false;
    const providedElements = Array.isArray(options.elements) ? options.elements : null;
    const preservePositions = Boolean(options.preservePositions);

    let resolved;
    if (providedElements) {
      resolved = { elements: providedElements, warnings: options.warnings || [], emptyReason: null, origin: options.origin || 'state-file', metadata: null, ir: null };
    } else {
      try {
        resolved = await this.resolveDiagramElements(graphType);
      } catch (err) {
        resolved = { elements: [], warnings: [], emptyReason: `Loading this view failed: ${(err && err.message) || err}`, origin: studioStore.sourceKind, metadata: null, ir: null };
      }
    }
    if (renderId !== this.renderSeq) return false;

    this.currentGraphType = graphType;
    // A loaded state file is a standalone snapshot; its codebase label lasts until the next render.
    this.snapshotLabel = resolved.origin === 'state-file' ? (options.snapshotLabel || null) : null;
    if (this.hoverTimeout) {
      clearTimeout(this.hoverTimeout);
      this.hoverTimeout = null;
    }
    if (this.cy) {
      this.cy.destroy();
      this.cy = null;
    }
    this.activeHoverId = null;
    this.collapsedPackages.clear();
    this.selectedNode = null;
    this.hitIndex = null;
    this.lodActive = false;

    let elements = resolved.elements;
    const isEmpty = !elements || elements.length === 0;
    const emptyReason = isEmpty ? (resolved.emptyReason || 'No diagram data is available for this view.') : null;
    if (isEmpty) elements = this.emptyStateElements(graphType, emptyReason);

    let safeElements;
    if (isEmpty) {
      safeElements = elements.map(e => Object.assign({}, e, { position: { x: 0, y: 0 } }));
    } else if (preservePositions) {
      safeElements = elements;
    } else {
      const layoutElements = computeDeterministicLayout(elements, graphType);
      const nodeIds = new Set(layoutElements.filter(e => e.data && !e.data.source).map(e => e.data.id));
      safeElements = layoutElements.filter(e => {
        if (e.data && e.data.source) return nodeIds.has(e.data.source) && nodeIds.has(e.data.target);
        return true;
      });
    }

    if (typeof cytoscape === 'undefined') {
      this.lastRenderMeta = {
        graphType,
        origin: resolved.origin,
        warnings: resolved.warnings || [],
        emptyReason: 'The diagram renderer (Cytoscape.js) is not loaded, so nothing can be drawn.',
        metadata: null,
        ir: null,
        pumlText: null,
        elementCount: 0,
        largeGraph: false
      };
      if (this.onRenderCompleteCallback) this.onRenderCompleteCallback([], this.lastRenderMeta);
      return false;
    }

    const large = safeElements.length > LARGE_GRAPH_ELEMENTS;
    this.cy = cytoscape({
      container,
      elements: safeElements,
      boxSelectionEnabled: false,
      autounselectify: false,
      userZoomingEnabled: false,
      userPanningEnabled: true,
      autoungrabify: this.isPanLocked,
      minZoom: 0.04,
      maxZoom: 5.0,
      pixelRatio: 'auto',
      textureOnViewport: large,
      hideEdgesOnViewport: large,
      motionBlur: false,
      wheelSensitivity: 0.05,
      style: this.getModernStyleSheet(),
      layout: { name: 'preset', animate: false }
    });

    if (this.isPanLocked) {
      this.cy.nodes().ungrabify();
    }

    this.attachEventListeners(container);
    if (!preservePositions || isEmpty) {
      this.cy.fit(undefined, 60);
    }
    // Every newly loaded diagram starts with a fresh history, so earlier actions can never
    // replay against different elements.
    this.undoStack = [];
    this.redoStack = [];
    this.updateUndoRedoButtons();
    this.applyEdgeFilters();
    this.updateLevelOfDetail();

    if (this.minimap) {
      this.minimap.invalidateCaches();
      this.minimap.onGraphRendered();
    }
    this.cy.on('pan zoom viewport', () => {
      if (this.minimap) this.minimap.onViewportChange();
    });
    this.cy.on('zoom', () => this.updateLevelOfDetail());

    this.lastRenderMeta = {
      graphType,
      origin: resolved.origin,
      warnings: resolved.warnings || [],
      emptyReason,
      metadata: resolved.metadata || null,
      ir: resolved.ir || null,
      pumlText: resolved.pumlText || null,
      elementCount: isEmpty ? 0 : elements.length,
      largeGraph: large
    };

    if (this.onRenderCompleteCallback) {
      this.onRenderCompleteCallback(isEmpty ? [] : elements, this.lastRenderMeta);
    }
    if (this.onLayersUpdateCallback) {
      this.onLayersUpdateCallback(this.getActiveEdgeKinds());
    }
    return true;
  }

  /**
   * Restyles the current diagram for a theme change in place: the stylesheet and card images
   * are replaced inside one batch, so positions, deletions, collapses and history survive.
   */
  setTheme(isDark) {
    if (!this.cy) return;
    const cy = this.cy;
    let pumlCards = null;
    if (this.lastRenderMeta && this.lastRenderMeta.pumlText) {
      pumlCards = new Map();
      parsePumlToCytoscape(this.lastRenderMeta.pumlText, this.currentGraphType).forEach(el => {
        if (el.data && el.data.svgDataUri) pumlCards.set(el.data.id, el.data.svgDataUri);
      });
    }
    cy.batch(() => {
      cy.style(buildCytoscapeStylesheet(getTheme(isDark)));
      cy.nodes().forEach(node => {
        const data = node.data();
        if (!data.svgDataUri) return;
        if (data.cardSource === 'ir') {
          try {
            node.data('svgDataUri', renderCardSvg(cardFromNodeData(data), isDark).dataUri);
          } catch (err) {
            console.warn('[OpenHeart Canvas] Could not restyle card', data.id, err);
          }
        } else if (pumlCards && pumlCards.has(data.id)) {
          node.data('svgDataUri', pumlCards.get(data.id));
        }
      });
    });
    if (this.minimap) {
      this.minimap.invalidateCaches();
      this.minimap.onGraphRendered();
    }
  }

  /**
   * Level of detail: below the zoom threshold cards are drawn as plain labelled shapes,
   * which keeps pan and zoom responsive on large diagrams.
   */
  updateLevelOfDetail() {
    if (!this.cy) return;
    const shouldSimplify = this.cy.zoom() < LOD_ZOOM_THRESHOLD && this.cy.nodes('[?svgDataUri]').length > 40;
    if (shouldSimplify === this.lodActive) return;
    this.lodActive = shouldSimplify;
    this.cy.batch(() => {
      const cards = this.cy.nodes('[?svgDataUri]');
      if (shouldSimplify) cards.addClass('lod-low');
      else cards.removeClass('lod-low');
    });
  }

  focusNodeById(nodeId, { inspect = true } = {}) {
    if (!this.cy || !nodeId) return false;
    const node = this.cy.getElementById(nodeId);
    if (!node || node.length === 0) return false;
    const hiddenAncestor = node.ancestors().filter(a => a.hasClass('package-collapsed'));
    if (hiddenAncestor.length) {
      hiddenAncestor.forEach(a => this.togglePackageCollapse(a, false));
    }
    this.cy.animate({ center: { eles: node }, zoom: Math.max(0.7, this.cy.zoom()), duration: 250 });
    this.cy.nodes().unselect();
    node.select();
    if (inspect && !node.data('isPackage') && !node.data('isNotice')) {
      this.selectedNode = node.data();
      if (this.onNodeSelectedCallback) this.onNodeSelectedCallback(node.data());
    }
    return true;
  }

  /**
   * Fuzzy search over node names and qualified names (subsequence match, scored by
   * contiguity, word-start hits and name length).
   */
  searchNodes(query, limit = 20) {
    if (!this.cy) return [];
    const q = String(query || '').trim().toLowerCase();
    if (!q) return [];
    const scoreText = (text) => {
      const t = text.toLowerCase();
      const direct = t.indexOf(q);
      if (direct >= 0) return 1000 - direct * 2 - (t.length - q.length) * 0.5 + (direct === 0 ? 200 : 0);
      let score = 0;
      let ti = 0;
      let streak = 0;
      for (let qi = 0; qi < q.length; qi++) {
        const ch = q[qi];
        let found = -1;
        while (ti < t.length) {
          if (t[ti] === ch) { found = ti; break; }
          ti++;
        }
        if (found < 0) return -Infinity;
        const atWordStart = found === 0 || /[^a-z0-9]/.test(t[found - 1]) || (text[found] !== t[found]);
        streak = (qi > 0 && found === ti && streak > 0) ? streak + 1 : 1;
        score += 10 + streak * 4 + (atWordStart ? 12 : 0);
        ti = found + 1;
      }
      return score - t.length * 0.2;
    };
    const results = [];
    this.cy.nodes().forEach(n => {
      const d = n.data();
      if (d.isNotice) return;
      const name = String(d.name || d.rawName || d.id);
      const qualified = String(d.qualifiedName || d.id);
      const best = Math.max(scoreText(name), scoreText(qualified) - 15);
      if (best > -Infinity) {
        results.push({ id: d.id, name, qualifiedName: qualified, kind: d.kind || 'node', isPackage: Boolean(d.isPackage || d.isLeafPackage), score: best });
      }
    });
    results.sort((a, b) => b.score - a.score || a.name.localeCompare(b.name));
    return results.slice(0, limit);
  }

  /**
   * Keyboard traversal over visible, non-package nodes in reading order (top-to-bottom,
   * left-to-right). Returns the newly focused node's data or null.
   */
  navigateNodes(step) {
    if (!this.cy) return null;
    const nodes = this.cy.nodes().filter(n => n.visible() && !n.data('isPackage') && !n.data('isNotice')).toArray();
    if (nodes.length === 0) return null;
    nodes.sort((a, b) => {
      const pa = a.position();
      const pb = b.position();
      if (Math.abs(pa.y - pb.y) > 40) return pa.y - pb.y;
      return pa.x - pb.x;
    });
    const currentId = this.selectedNode ? this.selectedNode.id : null;
    let idx = currentId ? nodes.findIndex(n => n.id() === currentId) : -1;
    idx = idx < 0 ? (step > 0 ? 0 : nodes.length - 1) : (idx + step + nodes.length) % nodes.length;
    const node = nodes[idx];
    this.cy.nodes().unselect();
    node.select();
    this.selectedNode = node.data();
    this.cy.animate({ center: { eles: node }, duration: 180 });
    return node.data();
  }

  /**
   * Plain structural snapshot of the current view for interchange exports (DOT, GraphML).
   */
  getGraphSnapshot() {
    if (!this.cy) return null;
    const nodes = [];
    const edges = [];
    this.cy.nodes().forEach(n => {
      const d = n.data();
      if (d.isNotice) return;
      nodes.push({
        id: d.id,
        name: String(d.name || d.rawName || d.id),
        kind: d.kind || 'node',
        stereotype: d.stereotype || null,
        parent: d.parent || null,
        isPackage: Boolean(d.isPackage || d.isLeafPackage),
        file: d.file || null,
        line: Array.isArray(d.lines) && d.lines.length ? d.lines[0] : null,
        qualifiedName: d.qualifiedName || null
      });
    });
    this.cy.edges().forEach(e => {
      const d = e.data();
      edges.push({ id: d.id, source: d.source, target: d.target, kind: d.uml_kind || 'association', label: d.label || '' });
    });
    return {
      diagramType: this.currentGraphType,
      codebaseName: studioStore.codebaseName,
      sourceKind: studioStore.sourceKind,
      nodes,
      edges
    };
  }

  focusNodeByFile(fileName) {
    if (!this.cy || !fileName) return;
    const baseName = fileName.replace(/\\/g, '/').split('/').pop();
    const node = this.cy.nodes().filter(n => {
      const f = n.data('file');
      const id = n.data('id');
      return f === fileName ||
             (baseName && f && (f.endsWith('/' + baseName) || f === baseName)) ||
             id === fileName ||
             id === baseName;
    })[0];

    if (node) {
      this.cy.animate({
        center: { eles: node },
        zoom: Math.max(0.7, this.cy.zoom()),
        duration: 250
      });
      this.cy.nodes().unselect();
      node.select();
      if (this.onNodeSelectedCallback) {
        this.onNodeSelectedCallback(node.data());
      }
    }
  }

  getNodeDataByFile(fileName) {
    if (!this.cy || !fileName) return null;
    const baseName = fileName.replace(/\\/g, '/').split('/').pop();
    const node = this.cy.nodes().filter(n => {
      const f = n.data('file');
      return f === fileName || (baseName && f && (f.endsWith('/' + baseName) || f === baseName));
    })[0];
    return node ? node.data() : null;
  }

  getNodeDataById(nodeId) {
    if (!this.cy || !nodeId) return null;
    const node = this.cy.getElementById(nodeId);
    return (node && node.length > 0) ? node.data() : null;
  }

  getModernStyleSheet() {
    return buildCytoscapeStylesheet();
  }

  setLayersUpdateCallback(cb) {
    this.onLayersUpdateCallback = cb;
  }

  setEdgeFilter(umlKind, isVisible) {
    if (isVisible) {
      this.hiddenEdgeKinds.delete(umlKind);
    } else {
      this.hiddenEdgeKinds.add(umlKind);
    }
    this.applyEdgeFilters();
  }

  /**
   * Re-applies the relationship-kind filters through the 'layer-hidden' class (independent
   * of package-collapse hiding). Called after every render and state load.
   */
  applyEdgeFilters() {
    if (!this.cy) return;
    const hidden = this.hiddenEdgeKinds;
    this.cy.batch(() => {
      this.cy.edges().forEach(edge => {
        const kind = edge.data('uml_kind') || 'association';
        if (hidden.has(kind)) edge.addClass('layer-hidden');
        else edge.removeClass('layer-hidden');
      });
    });
    this.hitIndex = null;
  }

  getActiveEdgeKinds() {
    if (!this.cy) return [];
    const counts = new Map();
    this.cy.edges().forEach(e => {
      const k = e.data('uml_kind') || 'association';
      counts.set(k, (counts.get(k) || 0) + 1);
    });
    return Array.from(counts.entries()).map(([kind, count]) => ({
      kind,
      count,
      visible: !this.hiddenEdgeKinds.has(kind)
    }));
  }

  bindContainerDomListeners(container) {
    if (this.domListenersBound || !container) return;
    this.domListenersBound = true;

    // ── 1. Figma-Grade Smooth Navigation Engine (60fps/120fps RAF Inertia & Easing) ──
    const navPhysics = {
      targetPanDx: 0,
      targetPanDy: 0,
      targetZoom: null,
      zoomAnchor: null,
      panRafId: null,
      zoomRafId: null
    };

    const startPanLoop = () => {
      if (navPhysics.panRafId) return;
      const step = () => {
        if (!this.cy) {
          navPhysics.panRafId = null;
          return;
        }

        const remainingX = navPhysics.targetPanDx;
        const remainingY = navPhysics.targetPanDy;

        if (Math.abs(remainingX) < 0.25 && Math.abs(remainingY) < 0.25) {
          if (remainingX !== 0 || remainingY !== 0) {
            this.cy.panBy({ x: remainingX, y: remainingY });
          }
          navPhysics.targetPanDx = 0;
          navPhysics.targetPanDy = 0;
          navPhysics.panRafId = null;
          return;
        }

        // Exponential ease-out friction (smooth, organic glide)
        const moveX = remainingX * 0.28;
        const moveY = remainingY * 0.28;
        this.cy.panBy({ x: moveX, y: moveY });
        navPhysics.targetPanDx -= moveX;
        navPhysics.targetPanDy -= moveY;

        navPhysics.panRafId = requestAnimationFrame(step);
      };
      navPhysics.panRafId = requestAnimationFrame(step);
    };

    const startZoomLoop = () => {
      if (navPhysics.zoomRafId) return;
      const step = () => {
        if (!this.cy || (typeof this.cy.destroyed === 'function' && this.cy.destroyed()) || navPhysics.targetZoom === null || !navPhysics.zoomAnchor) {
          navPhysics.zoomRafId = null;
          return;
        }

        const currentZoom = this.cy.zoom();
        const diff = navPhysics.targetZoom - currentZoom;

        if (Math.abs(diff) < 0.0008) {
          this.cy.zoom({
            level: navPhysics.targetZoom,
            renderedPosition: navPhysics.zoomAnchor
          });
          navPhysics.targetZoom = null;
          navPhysics.zoomAnchor = null;
          navPhysics.zoomRafId = null;
          return;
        }

        // Smooth ease-out zoom convergence (fast and buttery ~120-150ms glide)
        const nextZoom = currentZoom + diff * 0.35;
        this.cy.zoom({
          level: nextZoom,
          renderedPosition: navPhysics.zoomAnchor
        });

        navPhysics.zoomRafId = requestAnimationFrame(step);
      };
      navPhysics.zoomRafId = requestAnimationFrame(step);
    };

    // ── Alt+wheel gestures: suppress the browser menu bar only while the canvas has focus ──
    window.addEventListener('keydown', (e) => {
      if ((e.key === 'Alt' || e.code === 'AltLeft' || e.code === 'AltRight') && this.isCanvasFocused()) {
        e.preventDefault();
      }
    }, { capture: true });

    window.addEventListener('keyup', (e) => {
      if ((e.key === 'Alt' || e.code === 'AltLeft' || e.code === 'AltRight') && this.isCanvasFocused()) {
        e.preventDefault();
      }
    }, { capture: true });

    // Attach wheel listener to full viewport area (canvas + overlay card)
    const viewportTarget = container.closest('.canvas-card') || container;

    viewportTarget.addEventListener('wheel', (e) => {
      // Scrollable overlays (relationship layers list, search results, menus) keep native scrolling.
      const target = e.target instanceof Element ? e.target : null;
      if (target && target.closest('.layers-checklist, .canvas-search-results, .export-menu-dropdown, .canvas-notice, .canvas-floating-layers-bar')) {
        return;
      }
      e.preventDefault();
      if (!this.cy) return;

      const rect = container.getBoundingClientRect();
      const pointerPos = {
        x: Math.max(0, Math.min(rect.width, e.clientX - rect.left)),
        y: Math.max(0, Math.min(rect.height, e.clientY - rect.top))
      };

      const userSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
      const speed = userSensitivity * 10.0;

      // Extract raw delta across all OS drivers (some remap Alt+wheel to deltaX or wheelDelta)
      let rawDelta = 0;
      if (Math.abs(e.deltaY) >= Math.abs(e.deltaX) && e.deltaY !== 0) {
        rawDelta = e.deltaY;
      } else if (e.deltaX !== 0) {
        rawDelta = e.deltaX;
      } else if (e.detail) {
        rawDelta = e.detail * 40;
      } else if (e.wheelDelta) {
        rawDelta = -e.wheelDelta;
      }

      if (rawDelta === 0) return;

      // ── Condition A: Shift + Wheel or Pure Horizontal Scroll -> Horizontal Pan ──
      const isHorizontalPan = e.shiftKey || (Math.abs(e.deltaX) > Math.abs(e.deltaY) && Math.abs(e.deltaY) === 0);
      if (isHorizontalPan) {
        const rawHDelta = e.shiftKey ? (e.deltaY !== 0 ? e.deltaY : e.deltaX) : e.deltaX;
        if (rawHDelta === 0) return;
        const isDiscreteWheel = e.deltaMode !== 0 || Math.abs(rawHDelta) >= 40;
        const dx = -Math.sign(rawHDelta) * (isDiscreteWheel ? 54 : Math.abs(rawHDelta)) * speed;
        if (isDiscreteWheel) {
          navPhysics.targetPanDx += dx;
          startPanLoop();
        } else {
          this.cy.panBy({ x: dx, y: 0 });
        }
        return;
      }

      // ── Condition B: ZOOM (Mouse Wheel & Trackpad Gestures) ──
      // Cancel any ongoing pan motion so zoom centers cleanly on cursor
      navPhysics.targetPanDx = 0;
      navPhysics.targetPanDy = 0;

      // Fast-path: trackpad pinch-to-zoom (instantaneous 1:1 proportional zoom)
      const isTrackpadPinch = Math.abs(rawDelta) < 25 && (e.ctrlKey || e.metaKey);
      if (isTrackpadPinch) {
        const currentZoom = this.cy.zoom();
        const zoomFactor = Math.exp(-rawDelta * 0.008);
        const newZoom = Math.min(5.0, Math.max(0.04, currentZoom * zoomFactor));
        this.cy.zoom({
          level: newZoom,
          renderedPosition: pointerPos
        });
        navPhysics.targetZoom = null;
        return;
      }

      // Normalize wheel delta across delta modes:
      // deltaMode 0: DOM_DELTA_PIXEL
      // deltaMode 1: DOM_DELTA_LINE (~33px per line)
      // deltaMode 2: DOM_DELTA_PAGE (~600px per page)
      let normalizedDelta = rawDelta;
      if (e.deltaMode === 1) {
        normalizedDelta = rawDelta * 33;
      } else if (e.deltaMode === 2) {
        normalizedDelta = rawDelta * 600;
      }

      const currentZoom = this.cy.zoom();

      // Determine starting base zoom:
      // If a zoom animation is already in flight, accumulate targetZoom.
      // However, if the user reverses wheel direction, reset base to current actual zoom
      // so it immediately reverses without lingering opposite momentum!
      let baseZoom = (navPhysics.targetZoom !== null && !isNaN(navPhysics.targetZoom))
        ? navPhysics.targetZoom
        : currentZoom;

      if (navPhysics.targetZoom !== null) {
        const currentDiff = navPhysics.targetZoom - currentZoom;
        if ((currentDiff > 0 && normalizedDelta > 0) || (currentDiff < 0 && normalizedDelta < 0)) {
          baseZoom = currentZoom;
        }
      }

      // Smooth exponential zoom factor (~12-14% per standard 100-120px wheel notch)
      // Clamped to avoid extreme single-event jumps if an anomalous delta arrives
      const clampedDelta = Math.max(-400, Math.min(400, normalizedDelta));
      const zoomMultiplier = Math.exp(-clampedDelta * 0.0012);

      navPhysics.targetZoom = Math.min(5.0, Math.max(0.04, baseZoom * zoomMultiplier));
      navPhysics.zoomAnchor = pointerPos;
      startZoomLoop();
    }, { passive: false });

    // ── 2 & 3. Unified Pan Drag Engine: Middle-Click, Spacebar Hand Tool, and Pan Lock Active Drag ──
    let isMiddleDragging = false;
    let isSpacePressed = false;
    let isLeftHandDragging = false;
    let leftHandStartPos = { x: 0, y: 0 };
    let hasLeftHandMoved = false;

    let dragRafId = null;
    let pendingDragDx = 0;
    let pendingDragDy = 0;

    const dispatchPendingDrag = () => {
      if (dragRafId) return;
      dragRafId = requestAnimationFrame(() => {
        dragRafId = null;
        if (this.cy && (pendingDragDx !== 0 || pendingDragDy !== 0)) {
          this.cy.panBy({ x: pendingDragDx, y: pendingDragDy });
          pendingDragDx = 0;
          pendingDragDy = 0;
        }
      });
    };

    container.addEventListener('mousedown', (e) => {
      // Pointer interaction moves keyboard focus to the canvas so canvas shortcuts apply.
      if (document.activeElement !== container) {
        container.focus({ preventScroll: true });
      }
      // Middle click pan (button 1)
      if (e.button === 1) {
        e.preventDefault();
        e.stopPropagation();
        isMiddleDragging = true;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        container.classList.add('cursor-grabbing');
        navPhysics.targetPanDx = 0;
        navPhysics.targetPanDy = 0;
        return;
      }

      // Left click pan when either Pan Lock is active OR Spacebar is held
      // Operates anywhere across canvas, diagrams, box models, packages, and edges
      if (e.button === 0 && (this.isPanLocked || isSpacePressed)) {
        e.preventDefault();
        e.stopPropagation();
        isLeftHandDragging = true;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        hasLeftHandMoved = false;
        container.classList.remove('cursor-grab');
        container.classList.add('cursor-grabbing');
        navPhysics.targetPanDx = 0;
        navPhysics.targetPanDy = 0;
        return;
      }
    }, { capture: true });

    window.addEventListener('mousemove', (e) => {
      if (isMiddleDragging && this.cy) {
        pendingDragDx += e.clientX - leftHandStartPos.x;
        pendingDragDy += e.clientY - leftHandStartPos.y;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        dispatchPendingDrag();
        return;
      }

      if (isLeftHandDragging && this.cy) {
        const dx = e.clientX - leftHandStartPos.x;
        const dy = e.clientY - leftHandStartPos.y;
        if (Math.hypot(dx, dy) > 2) {
          hasLeftHandMoved = true;
        }
        pendingDragDx += dx;
        pendingDragDy += dy;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        dispatchPendingDrag();
        return;
      }
    });

    window.addEventListener('mouseup', (e) => {
      if (e.button === 1 && isMiddleDragging) {
        isMiddleDragging = false;
        container.classList.remove('cursor-grabbing');
        if (this.isPanLocked || isSpacePressed) {
          container.classList.add('cursor-grab');
        }
        return;
      }

      if (e.button === 0 && isLeftHandDragging) {
        isLeftHandDragging = false;
        container.classList.remove('cursor-grabbing');
        if (this.isPanLocked || isSpacePressed) {
          container.classList.add('cursor-grab');
        }

        // If user tapped without dragging, trigger selection/inspection on node under cursor
        if (!hasLeftHandMoved && this.cy) {
          const rect = container.getBoundingClientRect();
          const renderedPos = {
            x: e.clientX - rect.left,
            y: e.clientY - rect.top
          };
          const targetNode = this.getNodeAtRenderedPos(renderedPos);
          if (!targetNode) {
            this.selectedNode = null;
            this.cy.nodes().unselect();
          } else if (!targetNode.data('isNotice')) {
            if (targetNode.data('isPackage')) {
              this.togglePackageCollapse(targetNode);
            } else {
              this.selectedNode = targetNode.data();
              this.cy.nodes().unselect();
              targetNode.select();
              if (this.onNodeSelectedCallback) {
                this.onNodeSelectedCallback(this.selectedNode);
              }
            }
          }
        }
        return;
      }
    }, { capture: true });

    // ── 3. Figma Hand Tool (Spacebar, only while the canvas has focus) ──
    window.addEventListener('keydown', (e) => {
      if (e.code === 'Space' && !e.repeat) {
        if (!this.isCanvasFocused()) return;
        e.preventDefault();
        isSpacePressed = true;
        container.classList.add('cursor-grab');
        if (this.cy) {
          this.cy.autoungrabify(true);
        }
      }
    });

    window.addEventListener('keyup', (e) => {
      if (e.code === 'Space') {
        isSpacePressed = false;
        isLeftHandDragging = false;
        if (!this.isPanLocked) {
          container.classList.remove('cursor-grab', 'cursor-grabbing');
          if (this.cy) {
            this.cy.autoungrabify(false);
          }
        }
      }
    });

    window.addEventListener('blur', () => {
      isSpacePressed = false;
      isLeftHandDragging = false;
      isMiddleDragging = false;
      if (!this.isPanLocked) {
        container.classList.remove('cursor-grab', 'cursor-grabbing');
        if (this.cy) {
          this.cy.autoungrabify(false);
        }
      }
    });

    // ── 4. Figma Keyboard Shortcuts (Ctrl + / -, Ctrl 0, Shift 1, Arrow Keys) ──
    window.addEventListener('keydown', (e) => {
      const active = document.activeElement;
      const activeTag = active ? active.tagName : '';
      if (['INPUT', 'TEXTAREA', 'SELECT'].includes(activeTag) || active?.isContentEditable) {
        return;
      }
      const canvasFocused = this.isCanvasFocused();
      // Modifier shortcuts apply when focus is on the canvas card or nowhere in particular.
      const modifierScope = canvasFocused || this.isFocusInCanvasCard() || !active || active === document.body;
      if (!canvasFocused && !modifierScope) return;

      const isCtrl = e.ctrlKey || e.metaKey;
      const isZ = e.key === 'z' || e.key === 'Z' || e.code === 'KeyZ';
      const isY = e.key === 'y' || e.key === 'Y' || e.code === 'KeyY';

      // ── Figma Undo: Ctrl + Z / Cmd + Z (without Shift) ──
      if (isCtrl && !e.shiftKey && isZ) {
        e.preventDefault();
        this.undo();
        return;
      }

      // ── Figma Redo: Ctrl + Shift + Z / Cmd + Shift + Z OR Ctrl + Y / Cmd + Y ──
      if ((isCtrl && e.shiftKey && isZ) || (isCtrl && isY)) {
        e.preventDefault();
        this.redo();
        return;
      }

      // ── Delete / Backspace: Remove selected node with full undo/redo (canvas focus only) ──
      if (canvasFocused && (e.key === 'Delete' || e.key === 'Backspace') && (this.selectedNode || (this.cy && this.cy.nodes(':selected').length > 0))) {
        e.preventDefault();
        this.deleteSelectedNode();
        return;
      }

      // ── Keyboard node traversal: N / Shift+N, Enter to inspect, Escape to clear ──
      if (canvasFocused && !isCtrl && !e.altKey && (e.key === 'n' || e.key === 'N')) {
        e.preventDefault();
        const data = this.navigateNodes(e.shiftKey ? -1 : 1);
        if (data) this.announce(`${data.name || data.id} (${data.kind || 'node'}). Press Enter to inspect.`);
        return;
      }
      if (canvasFocused && e.key === 'Enter' && this.selectedNode) {
        e.preventDefault();
        if (this.onNodeSelectedCallback) this.onNodeSelectedCallback(this.selectedNode);
        return;
      }
      if (canvasFocused && e.key === 'Escape' && this.selectedNode && this.cy) {
        this.cy.nodes().unselect();
        this.selectedNode = null;
        this.announce('Selection cleared.');
        return;
      }

      // Ctrl/Cmd + = / + : Zoom In
      if (isCtrl && (e.key === '=' || e.key === '+')) {
        e.preventDefault();
        this.zoomIn();
        return;
      }

      // Ctrl/Cmd + - : Zoom Out
      if ((e.ctrlKey || e.metaKey) && (e.key === '-' || e.key === '_')) {
        e.preventDefault();
        this.zoomOut();
        return;
      }

      // Ctrl/Cmd + 0 : Reset Zoom to 100%
      if ((e.ctrlKey || e.metaKey) && e.key === '0') {
        e.preventDefault();
        if (this.cy) {
          this.cy.animate({
            zoom: {
              level: 1.0,
              renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
            },
            duration: 200
          });
        }
        return;
      }

      // Shift + 1 OR Ctrl + 1 : Zoom to Fit (Fit all diagram elements into screen)
      if (((e.shiftKey || e.ctrlKey || e.metaKey) && e.key === '1') || e.key === '!') {
        e.preventDefault();
        this.resetView();
        return;
      }

      // Arrow Keys: pan the canvas (canvas focus only)
      if (canvasFocused && !isCtrl && ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) {
        e.preventDefault();
        const dist = e.shiftKey ? 120 : 40;
        const dx = e.key === 'ArrowLeft' ? dist : (e.key === 'ArrowRight' ? -dist : 0);
        const dy = e.key === 'ArrowUp' ? dist : (e.key === 'ArrowDown' ? -dist : 0);
        if (this.cy) {
          this.cy.panBy({ x: dx, y: dy });
        }
      }
    });

    // ── 5. Multi-Touch Support (2-Finger Pinch & Pan for Touchscreens) ──
    let touchStartDist = 0;
    let touchStartCenter = null;
    let touchStartZoom = 1;

    container.addEventListener('touchstart', (e) => {
      if (e.touches.length === 2 && this.cy) {
        const t1 = e.touches[0];
        const t2 = e.touches[1];
        touchStartDist = Math.hypot(t2.clientX - t1.clientX, t2.clientY - t1.clientY);
        touchStartCenter = {
          x: (t1.clientX + t2.clientX) / 2,
          y: (t1.clientY + t2.clientY) / 2
        };
        touchStartZoom = this.cy.zoom();
      }
    }, { passive: true });

    container.addEventListener('touchmove', (e) => {
      if (e.touches.length === 2 && touchStartDist > 0 && this.cy) {
        e.preventDefault();
        const t1 = e.touches[0];
        const t2 = e.touches[1];
        const currentDist = Math.hypot(t2.clientX - t1.clientX, t2.clientY - t1.clientY);
        const currentCenter = {
          x: (t1.clientX + t2.clientX) / 2,
          y: (t1.clientY + t2.clientY) / 2
        };

        const scale = 1 + (currentDist / touchStartDist - 1) * 0.4;
        const newZoom = Math.min(5.0, Math.max(0.04, touchStartZoom * scale));
        const rect = container.getBoundingClientRect();

        this.cy.zoom({
          level: newZoom,
          renderedPosition: {
            x: currentCenter.x - rect.left,
            y: currentCenter.y - rect.top
          }
        });

        const userSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        const speed = userSensitivity * 10.0;
        const deltaX = (currentCenter.x - touchStartCenter.x) * speed;
        const deltaY = (currentCenter.y - touchStartCenter.y) * speed;
        this.cy.panBy({ x: deltaX, y: deltaY });
        touchStartCenter = currentCenter;
      }
    }, { passive: false });
  }

  setPanLock(locked) {
    this.isPanLocked = Boolean(locked);
    if (this.cy) {
      this.cy.autoungrabify(this.isPanLocked);
      if (this.isPanLocked) {
        this.cy.nodes().ungrabify();
      } else {
        this.cy.nodes().grabify();
      }
    }
    const container = document.getElementById(this.containerId);
    if (container) {
      if (this.isPanLocked) {
        container.classList.add('pan-locked', 'cursor-grab');
      } else {
        container.classList.remove('pan-locked', 'cursor-grab', 'cursor-grabbing');
      }
    }
    return this.isPanLocked;
  }

  togglePanLock() {
    return this.setPanLock(!this.isPanLocked);
  }

  /**
   * Uniform-grid spatial index of visible node bounding boxes, rebuilt lazily after any
   * position, add/remove, collapse or filter change. Hit tests then inspect one cell.
   */
  buildHitIndex() {
    const cells = new Map();
    const boxes = [];
    this.cy.nodes().forEach(node => {
      if (!node.visible()) return;
      const bb = node.boundingBox();
      const depth = node.ancestors().length;
      const entry = { node, bb, leaf: !node.isParent(), depth };
      boxes.push(entry);
      const cx1 = Math.floor(bb.x1 / HIT_CELL_SIZE);
      const cx2 = Math.floor(bb.x2 / HIT_CELL_SIZE);
      const cy1 = Math.floor(bb.y1 / HIT_CELL_SIZE);
      const cy2 = Math.floor(bb.y2 / HIT_CELL_SIZE);
      for (let cx = cx1; cx <= cx2; cx++) {
        for (let cyi = cy1; cyi <= cy2; cyi++) {
          const key = `${cx},${cyi}`;
          if (!cells.has(key)) cells.set(key, []);
          cells.get(key).push(entry);
        }
      }
    });
    this.hitIndex = { cells };
  }

  getNodeAtRenderedPos(renderedPos) {
    if (!this.cy || !renderedPos) return null;
    if (!this.hitIndex) this.buildHitIndex();
    const pan = this.cy.pan();
    const zoom = this.cy.zoom();
    const modelX = (renderedPos.x - pan.x) / zoom;
    const modelY = (renderedPos.y - pan.y) / zoom;
    const key = `${Math.floor(modelX / HIT_CELL_SIZE)},${Math.floor(modelY / HIT_CELL_SIZE)}`;
    const candidates = this.hitIndex.cells.get(key) || [];
    let best = null;
    for (const c of candidates) {
      const bb = c.bb;
      if (modelX < bb.x1 || modelX > bb.x2 || modelY < bb.y1 || modelY > bb.y2) continue;
      // Prefer leaf cards over compound packages, then the most deeply nested.
      if (!best || (c.leaf && !best.leaf) || (c.leaf === best.leaf && c.depth > best.depth)) best = c;
    }
    return best ? best.node : null;
  }

  getDiagramStateJson() {
    if (!this.cy) return null;
    const session = studioStore.get();
    const elements = [];
    this.cy.elements().forEach(el => {
      if (el.data('isNotice')) return;
      const isNode = el.isNode();
      const classes = el.classes().filter(c => c !== 'path-highlighted' && c !== 'lod-low').join(' ');
      elements.push({
        group: isNode ? 'nodes' : 'edges',
        data: { ...el.data() },
        position: isNode ? { ...el.position() } : undefined,
        classes
      });
    });

    return {
      format: DIAGRAM_STATE_FORMAT,
      version: DIAGRAM_STATE_VERSION,
      generator: 'OpenHeart Studio',
      diagram_type: this.currentGraphType,
      codebase_name: this.snapshotLabel || session.source.name,
      source: {
        kind: session.source.kind,
        url: session.source.url || null,
        commit: session.source.commit || null
      },
      timestamp: new Date().toISOString(),
      pan: { ...this.cy.pan() },
      zoom: this.cy.zoom(),
      pan_locked: this.isPanLocked,
      active_filters: Array.from(this.hiddenEdgeKinds),
      elements
    };
  }

  /**
   * Loads a saved state that has already passed validateDiagramState().
   */
  async loadDiagramState(validState) {
    if (!validState || !Array.isArray(validState.elements)) {
      throw new Error('Invalid OpenHeart diagram state.');
    }
    const hasPositions = validState.elements.some(e => e.group === 'nodes' && e.position);
    this.hiddenEdgeKinds = new Set(validState.active_filters || []);
    const committed = await this.renderGraph(validState.diagram_type, {
      elements: validState.elements,
      preservePositions: hasPositions,
      origin: 'state-file',
      snapshotLabel: validState.codebase_name || 'Saved state'
    });
    if (!committed || !this.cy) return false;

    // Restore collapse bookkeeping from the saved classes.
    this.cy.nodes('.package-collapsed').forEach(n => this.collapsedPackages.add(n.id()));
    if (validState.zoom !== null && validState.pan) {
      this.cy.viewport({ zoom: validState.zoom, pan: validState.pan });
    }
    this.setPanLock(validState.pan_locked);
    if (this.onLayersUpdateCallback) {
      this.onLayersUpdateCallback(this.getActiveEdgeKinds());
    }
    if (this.minimap) this.minimap.onGraphRendered();
    return true;
  }

  /**
   * Exports the full diagram as PNG, scaled down so neither side exceeds the browser canvas
   * limits. Throws when the diagram is too large to export legibly as a single image.
   * @returns {{ dataUri: string, scale: number, downscaled: boolean }}
   */
  exportPng() {
    if (!this.cy) return null;
    const bb = this.cy.elements().boundingBox();
    const w = Math.max(1, bb.w);
    const h = Math.max(1, bb.h);
    const scale = Math.min(2.0, MAX_EXPORT_DIMENSION / w, MAX_EXPORT_DIMENSION / h, Math.sqrt(MAX_EXPORT_PIXELS / (w * h)));
    if (scale < 0.08) {
      throw new Error(`The diagram is ${Math.round(w)}×${Math.round(h)} px; a single PNG would be illegible at ${(scale * 100).toFixed(1)}% scale. Collapse packages, filter relationships or export SVG instead.`);
    }
    // The scale is computed here rather than via cytoscape's maxWidth/maxHeight, which would
    // also upscale small diagrams to the maximum size.
    const dataUri = this.cy.png({
      full: true,
      scale,
      bg: isDarkMode() ? '#0b0f19' : '#ffffff'
    });
    return { dataUri, scale, downscaled: scale < 2.0 };
  }

  exportSvg() {
    if (!this.cy) return null;
    if (typeof this.cy.svg === 'function') {
      return this.cy.svg({ full: true, scale: 1.0 });
    }
    return null;
  }

  /**
   * Normalizes a field definition (from object or raw string) into a structured AST property object.
   */
  parseFieldDefinition(f) {
    if (!f) return null;

    if (typeof f === 'object') {
      const name = f.name || 'field';
      const type = f.type || f.type_name || null;
      let visibility = f.visibility || 'private';
      if (visibility === '+') visibility = 'public';
      else if (visibility === '-') visibility = 'private';
      else if (visibility === '#') visibility = 'protected';
      else if (visibility === '~') visibility = 'package';

      const isStatic = Boolean(f.is_static || f.isStatic);
      const isFinal = Boolean(f.is_final || f.isFinal || f.is_readonly || f.readonly);
      const isCollection = Boolean(f.is_collection || f.isCollection || (type && /List|Set|Map|Collection|Iterable|Array|Vec|\[\]/i.test(type)));
      const defaultValue = f.default_value || f.defaultValue || null;
      const signature = f.signature || `${visibility} ${isStatic ? 'static ' : ''}${isFinal ? 'final ' : ''}${name}${type ? `: ${type}` : ''}${defaultValue ? ` = ${defaultValue}` : ''}`.trim();

      return {
        name,
        type,
        visibility,
        is_static: isStatic,
        is_final: isFinal,
        is_collection: isCollection,
        default_value: defaultValue,
        signature
      };
    }

    if (typeof f === 'string') {
      const raw = f.trim();
      if (!raw) return null;

      const isStatic = /\{static\}|\bstatic\b/i.test(raw);
      const isFinal = /\{final\}|\{readonly\}|\bfinal\b|\bconst\b|\breadonly\b/i.test(raw);

      // Strip annotations like {static}, {abstract} first to correctly inspect visibility prefix
      const stripped = raw.replace(/\{[^}]+\}\s*/g, '').trim();

      let visibility = 'private';
      if (stripped.startsWith('+') || /\bpublic\b/i.test(stripped)) visibility = 'public';
      else if (stripped.startsWith('#') || /\bprotected\b/i.test(stripped)) visibility = 'protected';
      else if (stripped.startsWith('~') || /\bpackage\b/i.test(stripped)) visibility = 'package';
      else if (stripped.startsWith('-') || /\bprivate\b/i.test(stripped)) visibility = 'private';

      let clean = stripped
        .replace(/^[+\-#~]\s*/, '')
        .replace(/\b(public|protected|private|static|final|readonly|const|val|var|let|mut)\b/gi, '')
        .replace(/;$/, '')
        .trim();

      let defaultValue = null;
      if (clean.includes('=')) {
        const parts = clean.split('=');
        clean = parts[0].trim();
        defaultValue = parts.slice(1).join('=').trim();
      }

      let name = 'field';
      let type = null;

      if (clean.includes(':')) {
        const parts = clean.split(':');
        name = parts[0].trim().replace(/[^a-zA-Z0-9_$]/g, '');
        type = parts.slice(1).join(':').trim() || null;
      } else if (clean.includes(' ')) {
        const tokens = clean.split(/\s+/).filter(Boolean);
        if (tokens.length >= 2) {
          name = tokens[tokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
          type = tokens.slice(0, -1).join(' ');
        } else if (tokens.length === 1) {
          name = tokens[0].replace(/[^a-zA-Z0-9_$]/g, '');
        }
      } else {
        name = clean.replace(/[^a-zA-Z0-9_$]/g, '') || 'field';
      }

      const isCollection = Boolean(type && /List|Set|Map|Collection|Iterable|Array|Vec|\[\]|<[^>]+>/i.test(type));
      const signature = `${visibility} ${isStatic ? 'static ' : ''}${isFinal ? 'final ' : ''}${name}${type ? `: ${type}` : ''}${defaultValue ? ` = ${defaultValue}` : ''}`.trim();

      return {
        name,
        type,
        visibility,
        is_static: isStatic,
        is_final: isFinal,
        is_collection: isCollection,
        default_value: defaultValue,
        signature
      };
    }

    return null;
  }

  /**
   * Normalizes a method definition (from object or raw string) into a structured AST method object.
   */
  parseMethodDefinition(m) {
    if (!m) return null;

    if (typeof m === 'object') {
      const name = m.name || 'method';
      const returnType = m.return_type || m.returnType || m.type_name || null;
      let visibility = m.visibility || 'public';
      if (visibility === '+') visibility = 'public';
      else if (visibility === '-') visibility = 'private';
      else if (visibility === '#') visibility = 'protected';
      else if (visibility === '~') visibility = 'package';

      const isStatic = Boolean(m.is_static || m.isStatic);
      const isAbstract = Boolean(m.is_abstract || m.isAbstract);
      const isAsync = Boolean(m.is_async || m.isAsync);

      let parameters = [];
      if (Array.isArray(m.parameters)) {
        parameters = m.parameters.map(p => {
          if (typeof p === 'object' && p !== null) {
            return { name: p.name || null, type: p.type || p.type_name || null };
          }
          return { name: String(p), type: null };
        });
      }

      const paramSig = parameters.map(formatParameter).join(', ');
      const signature = m.signature || `${visibility} ${isStatic ? 'static ' : ''}${isAbstract ? 'abstract ' : ''}${isAsync ? 'async ' : ''}${name}(${paramSig})${returnType ? `: ${returnType}` : ''}`.trim();

      return {
        name,
        return_type: returnType,
        visibility,
        is_static: isStatic,
        is_abstract: isAbstract,
        is_async: isAsync,
        parameters,
        signature
      };
    }

    if (typeof m === 'string') {
      const raw = m.trim();
      if (!raw) return null;

      const isStatic = /\{static\}|\bstatic\b/i.test(raw);
      const isAbstract = /\{abstract\}|\babstract\b/i.test(raw);
      const isAsync = /\basync\b/i.test(raw);

      // Strip annotations like {static}, {abstract} first to correctly inspect visibility prefix
      const stripped = raw.replace(/\{[^}]+\}\s*/g, '').trim();

      let visibility = 'public';
      if (stripped.startsWith('-') || /\bprivate\b/i.test(stripped)) visibility = 'private';
      else if (stripped.startsWith('#') || /\bprotected\b/i.test(stripped)) visibility = 'protected';
      else if (stripped.startsWith('~') || /\bpackage\b/i.test(stripped)) visibility = 'package';
      else if (stripped.startsWith('+') || /\bpublic\b/i.test(stripped)) visibility = 'public';

      let clean = stripped
        .replace(/^[+\-#~]\s*/, '')
        .replace(/\b(public|protected|private|static|abstract|async|fun|fn|def|function)\b/gi, '')
        .replace(/;$/, '')
        .trim();

      const paramMatch = clean.match(/\(([^)]*)\)/);
      const paramsRaw = paramMatch ? paramMatch[1].trim() : '';
      const parameters = [];

      if (paramsRaw) {
        const parts = paramsRaw.split(',');
        for (const p of parts) {
          const trimmed = p.trim();
          if (!trimmed) continue;
          if (trimmed.includes(':')) {
            const [pName, pType] = trimmed.split(':').map(s => s.trim());
            parameters.push({ name: pName.replace(/[^a-zA-Z0-9_$]/g, '') || null, type: pType || null });
          } else {
            const tokens = trimmed.replace(/\bfinal\b|\bval\b|\bvar\b/g, '').trim().split(/\s+/);
            if (tokens.length >= 2) {
              const pType = tokens.slice(0, -1).join(' ');
              const pName = tokens[tokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
              parameters.push({ name: pName || 'arg', type: pType });
            } else if (tokens.length === 1) {
              parameters.push({ name: tokens[0].replace(/[^a-zA-Z0-9_$]/g, '') || null, type: null });
            }
          }
        }
      }

      let returnType = null;
      let name = 'method';

      const afterParenMatch = clean.match(/\)\s*:\s*([A-Za-z0-9_<>[\]?]+)/);
      if (afterParenMatch) {
        returnType = afterParenMatch[1].trim();
      }

      const beforeParen = clean.split('(')[0].trim();
      const nameTokens = beforeParen.split(/\s+/).filter(Boolean);
      if (nameTokens.length >= 2) {
        name = nameTokens[nameTokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
        if (returnType === null) {
          returnType = nameTokens.slice(0, -1).join(' ');
        }
      } else if (nameTokens.length === 1) {
        name = nameTokens[0].replace(/[^a-zA-Z0-9_$]/g, '');
      }

      const paramSig = parameters.map(formatParameter).join(', ');
      const signature = `${visibility} ${isStatic ? 'static ' : ''}${isAbstract ? 'abstract ' : ''}${isAsync ? 'async ' : ''}${name}(${paramSig})${returnType ? `: ${returnType}` : ''}`.trim();

      return {
        name,
        return_type: returnType,
        visibility,
        is_static: isStatic,
        is_abstract: isAbstract,
        is_async: isAsync,
        parameters,
        signature
      };
    }

    return null;
  }

  /**
   * Generates natural language semantic explanation for an architectural relationship.
   */
  describeRelationSemantics(srcName, tgtName, kind, rawArrow, label) {
    const k = (kind || '').toLowerCase();
    const arrow = rawArrow || '';
    const lblStr = label ? ` ('${label}')` : '';

    if (k === 'generalization' || arrow === '--|>' || arrow === '<|--') {
      return `${srcName} inherits from and specializes ${tgtName} (Generalization / Extends)`;
    }
    if (k === 'realization' || arrow === '..|>' || arrow === '<|..') {
      return `${srcName} realizes and implements contract of ${tgtName} (Realization / Implements)`;
    }
    if (k === 'composition' || arrow.includes('*--') || arrow.includes('--*')) {
      return `${srcName} strongly composes ${tgtName} with strict lifecycle ownership (Composition)`;
    }
    if (k === 'aggregation' || arrow.includes('o--') || arrow.includes('--o')) {
      return `${srcName} aggregates and references ${tgtName} (Aggregation)`;
    }
    if (k === 'dependency' || arrow.includes('..>') || arrow.includes('<..')) {
      return `${srcName} depends on and consumes ${tgtName}${lblStr} (Dependency)`;
    }
    if (k === 'association' || arrow.includes('-->') || arrow.includes('<--')) {
      return `${srcName} is associated with ${tgtName}${lblStr} (Association)`;
    }
    if (k === 'message') {
      return `${srcName} sends message '${label || 'invokes'}' to ${tgtName} (Sequence Message)`;
    }
    if (k === 'transition') {
      return `${srcName} transitions to ${tgtName}${label ? ` on event '${label}'` : ''} (State Transition)`;
    }
    if (k === 'control_flow') {
      return `${srcName} directs control flow to ${tgtName}${label ? ` condition '${label}'` : ''} (Control Flow)`;
    }
    if (k === 'data_flow') {
      return `${srcName} transmits data to ${tgtName}${lblStr} (Data Flow)`;
    }
    return `${srcName} connects to ${tgtName}${lblStr} (${kind})`;
  }

  /**
   * Exports pure semantic AST code structure without any coordinates, dimensions, SVG data URIs, or styling.
   * Tailored specifically for AI / LLM reasoning, code generation, and architectural analysis.
   */
  getCodeStructureForAi() {
    if (!this.cy) return null;

    const session = studioStore.get();
    const codebaseName = session.source.name;
    const nodes = this.cy.nodes();
    const edges = this.cy.edges();

    const nodeById = new Map();
    const packageNodes = [];
    const typeNodes = [];

    nodes.forEach(node => {
      const d = node.data() || {};
      if (d.isNotice) return;
      nodeById.set(node.id(), d);

      const isPkg = Boolean(d.isPackage || d.isLeafPackage || d.kind === 'package' || node.isParent());
      if (isPkg) {
        packageNodes.push({ id: node.id(), data: d });
      } else {
        typeNodes.push({ id: node.id(), data: d });
      }
    });

    const packageMembersMap = new Map();
    packageNodes.forEach(p => {
      const rawName = p.data.rawName || p.data.name || p.id;
      packageMembersMap.set(p.id, {
        id: p.id,
        name: rawName,
        is_domain_tier: Boolean(p.data.isDomainTier),
        members: []
      });
    });

    const relationships = [];
    const outgoingByNode = new Map();
    const incomingByNode = new Map();

    edges.forEach((edge, idx) => {
      const ed = edge.data() || {};
      const srcId = edge.source().id();
      const tgtId = edge.target().id();
      const srcData = nodeById.get(srcId) || {};
      const tgtData = nodeById.get(tgtId) || {};

      const srcName = srcData.name || srcData.label || srcId;
      const tgtName = tgtData.name || tgtData.label || tgtId;
      const umlKind = ed.uml_kind || ed.kind || 'association';
      const arrow = ed.arrow || '-->';
      const label = (ed.label || '').trim();

      const semantics = this.describeRelationSemantics(srcName, tgtName, umlKind, arrow, label);

      const relObj = {
        id: ed.id || `rel_${idx}_${srcId}_${tgtId}`,
        source: srcName,
        source_id: srcId,
        target: tgtName,
        target_id: tgtId,
        relation_type: umlKind,
        arrow: arrow,
        label: label || undefined,
        semantics: semantics
      };

      relationships.push(relObj);

      if (!outgoingByNode.has(srcId)) outgoingByNode.set(srcId, []);
      outgoingByNode.get(srcId).push({
        target: tgtName,
        relation_type: umlKind,
        semantics: semantics,
        label: label || undefined
      });

      if (!incomingByNode.has(tgtId)) incomingByNode.set(tgtId, []);
      incomingByNode.get(tgtId).push({
        source: srcName,
        relation_type: umlKind,
        semantics: semantics,
        label: label || undefined
      });
    });

    const types = [];
    let totalProperties = 0;
    let totalMethods = 0;
    let classCount = 0;
    let interfaceCount = 0;
    let enumCount = 0;
    let abstractCount = 0;

    typeNodes.forEach(t => {
      const d = t.data;
      const typeId = t.id;
      const typeName = d.name || d.label || typeId;
      const kind = (d.kind || 'class').toLowerCase();
      const stereotype = d.stereotype || `<<${kind}>>`;

      if (kind === 'interface') interfaceCount++;
      else if (kind === 'enum') enumCount++;
      else if (kind === 'abstract' || stereotype.includes('abstract')) abstractCount++;
      else classCount++;

      let packageName = d.package || null;
      if (!packageName && d.parent) {
        const parentPkg = packageMembersMap.get(d.parent);
        if (parentPkg) {
          packageName = parentPkg.name;
          if (!parentPkg.members.includes(typeName)) parentPkg.members.push(typeName);
        }
      } else if (packageName) {
        let existing = null;
        for (const [_, pkg] of packageMembersMap) {
          if (pkg.name === packageName) {
            existing = pkg;
            break;
          }
        }
        if (!existing) {
          existing = { id: packageName, name: packageName, members: [] };
          packageMembersMap.set(packageName, existing);
        }
        if (!existing.members.includes(typeName)) existing.members.push(typeName);
      }

      const extendsList = [];
      const implementsList = [];

      if (d.extends) {
        if (Array.isArray(d.extends)) extendsList.push(...d.extends);
        else extendsList.push(d.extends);
      }
      if (d.implements) {
        if (Array.isArray(d.implements)) implementsList.push(...d.implements);
        else implementsList.push(d.implements);
      }

      const outgoing = outgoingByNode.get(typeId) || [];
      outgoing.forEach(rel => {
        if (rel.relation_type === 'generalization' && !extendsList.includes(rel.target)) {
          extendsList.push(rel.target);
        } else if (rel.relation_type === 'realization' && !implementsList.includes(rel.target)) {
          implementsList.push(rel.target);
        }
      });

      const rawFields = Array.isArray(d.fields) ? d.fields : [];
      const properties = rawFields
        .map(f => this.parseFieldDefinition(f))
        .filter(Boolean);
      totalProperties += properties.length;

      const rawMethods = Array.isArray(d.methods) ? d.methods : [];
      const methods = rawMethods
        .map(m => this.parseMethodDefinition(m))
        .filter(Boolean);
      totalMethods += methods.length;

      types.push({
        id: typeId,
        name: typeName,
        kind: kind,
        stereotype: stereotype,
        package: packageName || undefined,
        qualified_name: d.qualifiedName || undefined,
        file_path: d.file || null,
        declaration_line: Array.isArray(d.lines) && d.lines.length ? d.lines[0] : null,
        extends: extendsList.length > 0 ? (extendsList.length === 1 ? extendsList[0] : extendsList) : undefined,
        implements: implementsList.length > 0 ? implementsList : undefined,
        properties: properties,
        methods: methods,
        outgoing_relations: outgoing,
        incoming_relations: incomingByNode.get(typeId) || []
      });
    });

    const packages = Array.from(packageMembersMap.values()).map(p => ({
      name: p.name,
      is_domain_tier: p.is_domain_tier || undefined,
      member_count: p.members.length,
      members: p.members
    }));

    const provenance = session.provenance || {};
    return {
      format: 'OpenHeart-Code-Structure',
      version: '1.1.0',
      generator: 'OpenHeart Studio - AI Code Structure Exporter',
      project_name: codebaseName,
      diagram_type: this.currentGraphType,
      export_timestamp: new Date().toISOString(),
      source: {
        kind: session.source.kind,
        url: session.source.url || null,
        commit: session.source.commit || null,
        engine: provenance.engine || null,
        approximate: provenance.approximate === true,
        content_hash: provenance.content_hash || null,
        coverage: provenance.coverage || null
      },
      description: 'Types, members and relationships of the current diagram view for AI/LLM context. Layout coordinates and UI formatting are excluded. When source.approximate is true the model comes from the in-browser declaration extractor over a sample of files, not from the Rust engine.',
      summary: {
        total_packages: packages.length,
        total_types: types.length,
        total_classes: classCount,
        total_interfaces: interfaceCount,
        total_abstract_classes: abstractCount,
        total_enums: enumCount,
        total_relationships: relationships.length,
        total_properties: totalProperties,
        total_methods: totalMethods
      },
      packages: packages,
      types: types,
      relationships: relationships
    };
  }

  /**
   * Generates a prompt-ready Markdown document formatted for pasting directly into LLMs (ChatGPT, Claude, Gemini, etc.).
   */
  getCodeStructureMarkdownForAi() {
    const data = this.getCodeStructureForAi();
    if (!data) return '';

    const lines = [];
    lines.push(`# Codebase Architecture & Semantic Model: ${data.project_name}`);
    lines.push(`> Exported from OpenHeart Studio for AI & LLM Contextual Reasoning (Type Contracts, AST, & Relationship Matrix)`);
    lines.push('');
    lines.push(`## Architectural Overview`);
    lines.push(`- **Project Name:** ${data.project_name}`);
    lines.push(`- **Analysis Source:** ${data.source.engine || data.source.kind}${data.source.approximate ? ' (approximate, sampled files)' : ''}`);
    if (data.source.coverage) lines.push(`- **Coverage:** ${data.source.coverage}`);
    if (data.source.content_hash) lines.push(`- **IR Content Hash:** ${data.source.content_hash}`);
    lines.push(`- **Diagram Projection:** ${data.diagram_type.toUpperCase()}`);
    lines.push(`- **Total Packages:** ${data.summary.total_packages}`);
    lines.push(`- **Total Types:** ${data.summary.total_types} (${data.summary.total_classes} Classes, ${data.summary.total_interfaces} Interfaces, ${data.summary.total_abstract_classes} Abstract Classes, ${data.summary.total_enums} Enums)`);
    lines.push(`- **Total Properties / Fields:** ${data.summary.total_properties}`);
    lines.push(`- **Total Functions / Methods:** ${data.summary.total_methods}`);
    lines.push(`- **Total Architectural Couplings:** ${data.summary.total_relationships}`);
    lines.push('');

    if (data.packages.length > 0) {
      lines.push(`## Packages & Namespaces`);
      data.packages.forEach(pkg => {
        const membersStr = pkg.members.length > 0 ? pkg.members.join(', ') : 'None';
        lines.push(`- **\`${pkg.name}\`** (${pkg.member_count} types): ${membersStr}`);
      });
      lines.push('');
    }

    lines.push(`## Type Definitions & Declarations`);
    lines.push('');

    data.types.forEach(t => {
      const kindHeader = `${t.kind || 'class'} ${t.name}`;
      lines.push(`### \`${kindHeader}\``);
      if (t.package) lines.push(`- **Package:** \`${t.package}\``);
      if (t.file_path) lines.push(`- **Source File:** \`${t.file_path}\``);
      if (t.extends) {
        const extStr = Array.isArray(t.extends) ? t.extends.join(', ') : t.extends;
        lines.push(`- **Extends / Base:** \`${extStr}\``);
      }
      if (t.implements && t.implements.length > 0) {
        lines.push(`- **Implements:** \`${t.implements.join(', ')}\``);
      }

      if (t.properties.length > 0) {
        lines.push('');
        lines.push(`#### Properties & Fields (${t.properties.length})`);
        t.properties.forEach(p => {
          const modStr = [
            p.visibility,
            p.is_static ? 'static' : '',
            p.is_final ? 'final' : '',
            p.is_collection ? 'collection' : ''
          ].filter(Boolean).join(' ');
          const valStr = p.default_value ? ` = ${p.default_value}` : '';
          lines.push(`- \`${modStr ? modStr + ' ' : ''}${p.name}${p.type ? `: ${p.type}` : ''}${valStr}\``);
        });
      }

      if (t.methods.length > 0) {
        lines.push('');
        lines.push(`#### Methods & Functions (${t.methods.length})`);
        t.methods.forEach(m => {
          const modStr = [
            m.visibility,
            m.is_static ? 'static' : '',
            m.is_abstract ? 'abstract' : '',
            m.is_async ? 'async' : ''
          ].filter(Boolean).join(' ');
          const paramStr = m.parameters.map(formatParameter).join(', ');
          lines.push(`- \`${modStr ? modStr + ' ' : ''}${m.name}(${paramStr})${m.return_type ? `: ${m.return_type}` : ''}\``);
        });
      }

      if (t.outgoing_relations.length > 0 || t.incoming_relations.length > 0) {
        lines.push('');
        lines.push(`#### Coupling & Relations`);
        t.outgoing_relations.forEach(r => {
          lines.push(`- Outgoing: ${r.semantics}`);
        });
        t.incoming_relations.forEach(r => {
          lines.push(`- Incoming: ${r.semantics}`);
        });
      }

      lines.push('');
      lines.push('---');
      lines.push('');
    });

    if (data.relationships.length > 0) {
      lines.push(`## Relationship Coupling Matrix`);
      data.relationships.forEach(r => {
        lines.push(`- **\`${r.source}\`** ${r.arrow} **\`${r.target}\`** : ${r.semantics}`);
      });
      lines.push('');
    }

    return lines.join('\n');
  }

  attachEventListeners(container) {
    if (!this.cy || !container) return;

    // ── High-Performance Local Neighborhood Hover Illumination ──
    this.cy.on('mouseover', 'node, edge', (e) => {
      const target = e.target;
      if (target.data('isPackage')) return;

      const targetId = target.id();
      if (this.activeHoverId === targetId) return;
      this.activeHoverId = targetId;

      if (this.hoverTimeout) {
        clearTimeout(this.hoverTimeout);
        this.hoverTimeout = null;
      }

      const neighborhood = target.isNode()
        ? target.closedNeighborhood()
        : target.connectedNodes().union(target);

      this.cy.batch(() => {
        target.addClass('path-highlighted');
        neighborhood.addClass('path-highlighted');
      });

      if (this.onNodeHoverCallback && target.isNode()) {
        this.onNodeHoverCallback(target.data());
      }
    });

    this.cy.on('mouseout', 'node, edge', (e) => {
      const target = e.target;
      if (this.hoverTimeout) clearTimeout(this.hoverTimeout);
      this.hoverTimeout = setTimeout(() => {
        this.activeHoverId = null;
        this.cy.batch(() => {
          this.cy.elements('.path-highlighted').removeClass('path-highlighted');
        });
      }, 30);
    });

    // ── Click to Inspect & Synchronize Monaco ──
    this.cy.on('tap', 'node', (e) => {
      const node = e.target;
      if (node.data('isNotice')) return;

      // If clicking package container, toggle collapse/expand (Opening & Closing)
      if (node.data('isPackage')) {
        this.togglePackageCollapse(node);
        return;
      }

      this.selectedNode = node.data();
      if (this.onNodeSelectedCallback) {
        this.onNodeSelectedCallback(this.selectedNode);
      }
    });

    // Tapping the background clears the selection so Delete and arrow keys stop targeting it.
    this.cy.on('tap', (e) => {
      if (e.target === this.cy) {
        this.selectedNode = null;
        this.cy.nodes().unselect();
      }
    });

    // Any geometry or membership change invalidates the hit-test index.
    this.cy.on('position add remove', () => {
      this.hitIndex = null;
    });

    // ── Node Reposition Tracking for Figma-Style Undo / Redo ──
    const dragStartPositions = new Map();

    this.cy.on('grab', 'node', (e) => {
      dragStartPositions.clear();
      // Only the grabbed node, its descendants and the current selection can move.
      const moving = e.target.union(e.target.descendants()).union(this.cy.nodes(':selected'));
      moving.union(moving.descendants()).forEach(n => {
        dragStartPositions.set(n.id(), { ...n.position() });
      });
    });

    this.cy.on('free', 'node', () => {
      if (dragStartPositions.size === 0) return;
      const movedNodes = [];
      dragStartPositions.forEach((oldPos, id) => {
        const node = this.cy.getElementById(id);
        if (node && node.length > 0) {
          const currentPos = node.position();
          if (Math.hypot(currentPos.x - oldPos.x, currentPos.y - oldPos.y) > 1.0) {
            movedNodes.push({
              id,
              oldPos: { x: oldPos.x, y: oldPos.y },
              newPos: { x: currentPos.x, y: currentPos.y }
            });
          }
        }
      });
      dragStartPositions.clear();

      if (movedNodes.length > 0 && this.minimap) {
        this.minimap.scheduleThumbnailRefresh();
      }

      if (movedNodes.length > 0) {
        const targetLabel = movedNodes.length === 1
          ? (this.cy.getElementById(movedNodes[0].id).data('label') || movedNodes[0].id)
          : `${movedNodes.length} nodes`;

        this.pushUndoAction({
          type: 'move',
          description: `Move ${targetLabel}`,
          undo: () => {
            const canAnimate = movedNodes.length <= 15;
            movedNodes.forEach(({ id, oldPos }) => {
              const n = this.cy.getElementById(id);
              if (n && n.length > 0) {
                if (canAnimate) {
                  n.animate({ position: oldPos }, { duration: 180 });
                } else {
                  n.position(oldPos);
                }
              }
            });
            if (this.minimap) this.minimap.scheduleThumbnailRefresh();
          },
          redo: () => {
            const canAnimate = movedNodes.length <= 15;
            movedNodes.forEach(({ id, newPos }) => {
              const n = this.cy.getElementById(id);
              if (n && n.length > 0) {
                if (canAnimate) {
                  n.animate({ position: newPos }, { duration: 180 });
                } else {
                  n.position(newPos);
                }
              }
            });
            if (this.minimap) this.minimap.scheduleThumbnailRefresh();
          }
        });
      }
    });
  }

  togglePackageCollapse(pkgNode, recordUndo = true) {
    if (!pkgNode || (typeof pkgNode.id !== 'function' && !pkgNode.length)) return;
    const actualNode = pkgNode.length !== undefined ? pkgNode[0] : pkgNode;
    const pkgId = actualNode.id();
    const children = actualNode.children();
    const isCollapsed = this.collapsedPackages.has(pkgId);
    const rawName = actualNode.data('rawName') || pkgId.replace(/^pkg_/, '').replace(/_/g, '.');
    const shortName = rawName.split('.').pop();
    const isDomainTier = actualNode.data('isDomainTier');
    const childPackages = children.filter(c => c.data('isPackage') || c.data('isLeafPackage')).length;
    const childTypes = children.length - childPackages;

    this.cy.batch(() => {
      if (isCollapsed) {
        // Expand: reveal descendants, except those still inside a nested collapsed package.
        this.collapsedPackages.delete(pkgId);
        actualNode.removeClass('package-collapsed');
        actualNode.data('label', isDomainTier ? `DOMAIN: ${rawName.toUpperCase()}` : `package [${shortName}]`);
        const reveal = actualNode.descendants().filter(d => {
          let p = d.parent();
          while (p && p.length && p.id() !== pkgId) {
            if (this.collapsedPackages.has(p.id())) return false;
            p = p.parent();
          }
          return true;
        });
        reveal.removeClass('collapsed-hidden');
        reveal.connectedEdges().forEach(edge => {
          if (edge.source().hasClass('collapsed-hidden') || edge.target().hasClass('collapsed-hidden')) return;
          edge.removeClass('collapsed-hidden');
        });
      } else {
        // Collapse: hide every descendant and every edge touching one.
        this.collapsedPackages.add(pkgId);
        actualNode.addClass('package-collapsed');
        const parts = [];
        if (childPackages) parts.push(`${childPackages} package${childPackages === 1 ? '' : 's'}`);
        if (childTypes) parts.push(`${childTypes} type${childTypes === 1 ? '' : 's'}`);
        const summary = parts.length ? ` (${parts.join(', ')})` : '';
        actualNode.data('label', isDomainTier ? `[+] DOMAIN: ${rawName.toUpperCase()}${summary}` : `[+] package [${shortName}]${summary}`);
        const hide = actualNode.descendants();
        hide.addClass('collapsed-hidden');
        hide.connectedEdges().addClass('collapsed-hidden');
      }
    });
    this.hitIndex = null;
    if (this.minimap) this.minimap.scheduleThumbnailRefresh();

    if (recordUndo) {
      this.pushUndoAction({
        type: 'package-collapse',
        description: isCollapsed ? `Expand ${shortName}` : `Collapse ${shortName}`,
        undo: () => {
          const p = this.cy.getElementById(pkgId);
          if (p && p.length > 0) this.togglePackageCollapse(p, false);
        },
        redo: () => {
          const p = this.cy.getElementById(pkgId);
          if (p && p.length > 0) this.togglePackageCollapse(p, false);
        }
      });
    }
  }

  zoomIn() {
    if (!this.cy) return;
    this.cy.animate({
      zoom: {
        level: this.cy.zoom() * 1.25,
        renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
      },
      duration: 150
    });
  }

  zoomOut() {
    if (!this.cy) return;
    this.cy.animate({
      zoom: {
        level: this.cy.zoom() * 0.8,
        renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
      },
      duration: 150
    });
  }

  resetView() {
    if (!this.cy) return;
    this.cy.animate({
      fit: {
        eles: this.cy.elements(),
        padding: 60
      },
      duration: 250
    });
  }

  toggleMinimap(state = null) {
    if (this.minimap) {
      return this.minimap.toggleVisibility(state);
    }
    return false;
  }

  toggleLoupe(state = null) {
    if (this.minimap) {
      return this.minimap.toggleLoupe(state);
    }
    return false;
  }

  // ── 6. Figma-Grade Undo / Redo & Manipulation History Engine ──
  bindHistoryControls() {
    const btnUndo = document.getElementById('btn-canvas-undo');
    const btnRedo = document.getElementById('btn-canvas-redo');

    if (btnUndo) {
      btnUndo.addEventListener('click', (e) => {
        e.preventDefault();
        this.undo();
      });
    }

    if (btnRedo) {
      btnRedo.addEventListener('click', (e) => {
        e.preventDefault();
        this.redo();
      });
    }

    this.updateUndoRedoButtons();
  }

  pushUndoAction(action) {
    this.undoStack.push(action);
    if (this.undoStack.length > this.maxHistory) {
      this.undoStack.shift();
    }
    this.redoStack = [];
    this.updateUndoRedoButtons();
    if (action.description && action.type !== 'move') {
      this.showCanvasNotification(action.description);
    }
  }

  undo() {
    if (this.undoStack.length === 0) {
      this.showCanvasNotification('Nothing to undo');
      return;
    }
    const action = this.undoStack.pop();
    try {
      action.undo();
      this.redoStack.push(action);
      this.showCanvasNotification(`Undo: ${action.description || 'Action'}`);
    } catch (err) {
      console.error('Error during undo:', err);
    }
    this.updateUndoRedoButtons();
  }

  redo() {
    if (this.redoStack.length === 0) {
      this.showCanvasNotification('Nothing to redo');
      return;
    }
    const action = this.redoStack.pop();
    try {
      action.redo();
      this.undoStack.push(action);
      this.showCanvasNotification(`Redo: ${action.description || 'Action'}`);
    } catch (err) {
      console.error('Error during redo:', err);
    }
    this.updateUndoRedoButtons();
  }

  updateUndoRedoButtons() {
    const btnUndo = document.getElementById('btn-canvas-undo');
    const btnRedo = document.getElementById('btn-canvas-redo');

    if (btnUndo) {
      if (this.undoStack.length > 0) {
        btnUndo.classList.remove('disabled');
        btnUndo.removeAttribute('disabled');
        const lastAction = this.undoStack[this.undoStack.length - 1];
        btnUndo.setAttribute('title', `Undo ${lastAction.description || ''} (Ctrl+Z)`);
      } else {
        btnUndo.classList.add('disabled');
        btnUndo.setAttribute('disabled', 'true');
        btnUndo.setAttribute('title', 'Undo (Ctrl+Z)');
      }
    }

    if (btnRedo) {
      if (this.redoStack.length > 0) {
        btnRedo.classList.remove('disabled');
        btnRedo.removeAttribute('disabled');
        const nextAction = this.redoStack[this.redoStack.length - 1];
        btnRedo.setAttribute('title', `Redo ${nextAction.description || ''} (Ctrl+Y or Ctrl+Shift+Z)`);
      } else {
        btnRedo.classList.add('disabled');
        btnRedo.setAttribute('disabled', 'true');
        btnRedo.setAttribute('title', 'Redo (Ctrl+Y or Ctrl+Shift+Z)');
      }
    }
  }

  showCanvasNotification(msg) {
    const statusText = document.getElementById('hud-status-text');
    const hudStatus = document.getElementById('hud-status');
    if (statusText) {
      statusText.textContent = msg;
      if (hudStatus) {
        hudStatus.classList.remove('status-pulse');
        void hudStatus.offsetWidth;
        hudStatus.classList.add('status-pulse');
      }
      clearTimeout(this._hudStatusTimer);
      this._hudStatusTimer = setTimeout(() => {
        if (statusText) {
          statusText.textContent = DEFAULT_HUD_HINT;
        }
      }, 2500);
    }
  }

  deleteSelectedNode() {
    if (!this.cy) return;
    const selectedNodes = this.cy.nodes(':selected');
    const targetNodes = selectedNodes.length > 0
      ? selectedNodes
      : (this.selectedNode ? this.cy.getElementById(this.selectedNode.id) : null);

    if (!targetNodes || targetNodes.length === 0) return;

    const validTargets = targetNodes.filter(n => !n.data('isNotice'));
    if (validTargets.length === 0) return;

    // cytoscape's remove() also removes descendants of compound nodes, so the undo record
    // must capture every descendant and every edge touching any removed node.
    const removedNodes = validTargets.union(validTargets.descendants());
    const toDelete = removedNodes.union(removedNodes.connectedEdges());
    const depthOf = new Map();
    removedNodes.forEach(n => depthOf.set(n.id(), n.ancestors().length));
    const nodeJsons = removedNodes.jsons().sort((a, b) => (depthOf.get(a.data.id) || 0) - (depthOf.get(b.data.id) || 0));
    const edgeJsons = toDelete.edges().jsons();
    const elesJson = [...nodeJsons, ...edgeJsons];
    const removedIds = elesJson.map(item => item.data.id);
    const label = validTargets.length === 1
      ? (validTargets[0].data('name') || validTargets[0].data('rawName') || validTargets[0].id())
      : `${validTargets.length} nodes`;

    toDelete.remove();
    this.selectedNode = null;
    this.hitIndex = null;
    if (this.minimap) this.minimap.scheduleThumbnailRefresh();

    this.pushUndoAction({
      type: 'delete',
      description: `Delete ${label}`,
      undo: () => {
        const added = this.cy.add(elesJson);
        this.applyEdgeFilters();
        this.hitIndex = null;
        if (this.minimap) this.minimap.scheduleThumbnailRefresh();
        added.nodes().filter(n => validTargets.some(v => v.id() === n.id())).select();
      },
      redo: () => {
        removedIds.forEach(id => {
          const el = this.cy.getElementById(id);
          if (el && el.length > 0) el.remove();
        });
        this.selectedNode = null;
        this.hitIndex = null;
        if (this.minimap) this.minimap.scheduleThumbnailRefresh();
      }
    });

    this.showCanvasNotification(`Deleted ${label}`);
  }
}
