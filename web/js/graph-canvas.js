/**
 * OpenHeart Precision Interactive Graph Canvas (Deterministic UML Edition)
 * Strictly enforces UML 2.5 standard symbolic notation:
 *  - 3-Level Distinct Color Hierarchy:
 *      Level 1: Domain Tier Container (Solid border, soft pastel backdrop)
 *      Level 2: Subpackage Container (Dashed border, saturated pastel backdrop)
 *      Level 3: Enclosed 3-Compartment Class Cards (Crisp pure white with drop shadow)
 *  - Deterministic Preset Layout Engine (100% Collision-Free Guarantee)
 *  - Orthogonal Taxi Wiring for Clean, Non-Entangled Routing
 */

import { parsePumlToCytoscape } from './puml-parser.js';
import { computeDeterministicLayout } from './uml-layout.js';
import { loadGraphIrToCytoscape } from './graph-loader.js';
import { generatePackageFolderSvg } from './uml-card-renderer.js';
import { getTheme, onThemeChange, buildCytoscapeStylesheet } from './themes/index.js';

export class InteractiveGraphCanvas {
  constructor(containerId = 'interactive-canvas') {
    this.containerId = containerId;
    this.cy = null;
    this.currentGraphType = 'class';
    this.selectedNode = null;
    this.onNodeSelectedCallback = null;
    this.onNodeHoverCallback = null;
    this.onRenderCompleteCallback = null;
    this.panSensitivity = parseFloat(localStorage.getItem('openheart_pan_sensitivity') || '0.10');
    this.activeHoverId = null;
    this.hoverTimeout = null;
    this.collapsedPackages = new Set();
    this.onLayersUpdateCallback = null;
    this.hiddenEdgeKinds = new Set();
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.renderGraph(this.currentGraphType);

    onThemeChange((theme, isDark) => {
      this.setTheme(isDark);
    });
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
    localStorage.setItem('openheart_pan_sensitivity', val.toString());
  }

  getNodeDataById(nodeId) {
    if (!this.cy || !nodeId) return null;
    const node = this.cy.getElementById(nodeId);
    return node && node.length > 0 ? node.data() : null;
  }

  getNodeDataByFile(fileName) {
    if (!this.cy || !fileName) return null;
    const nodes = this.cy.nodes();
    const cleanFile = fileName.replace(/\.java$/, '').replace(/\.kt$/, '').toLowerCase();
    for (let i = 0; i < nodes.length; i++) {
      const d = nodes[i].data();
      if (!d) continue;
      const dFile = (d.file || '').replace(/\.java$/, '').replace(/\.kt$/, '').toLowerCase();
      const dId = (d.id || '').toLowerCase();
      const dName = (d.name || '').toLowerCase();
      if (dFile === cleanFile || dId === cleanFile || dName === cleanFile || dFile.includes(cleanFile)) {
        return d;
      }
    }
    return null;
  }

  async renderCustomGraphIr(graphIr) {
    if (!graphIr) return;
    this.customGraphIr = graphIr;
    const elements = loadGraphIrToCytoscape(graphIr);
    await this.renderGraph(graphIr.diagram_type || 'class', elements);
  }

  async renderGraph(graphType, customElements = null) {
    this.currentGraphType = graphType;
    const container = document.getElementById(this.containerId);
    if (!container) return;

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

    let elements = customElements;
    if (!elements) {
      // ── 1. Official Direct Ingestion: Strongly-Typed Graph IR from Rust Compiler ──
      try {
        const jsonRes = await fetch(`diagrams/${graphType}.json`);
        if (jsonRes.ok) {
          const graphIr = await jsonRes.json();
          elements = loadGraphIrToCytoscape(graphIr);
          console.log(`[OpenHeart Pipeline] Loaded Official Direct Graph IR for ${graphType}: ${elements.length} elements`);
        }
      } catch (jsonErr) {
        console.warn(`[OpenHeart Pipeline] Direct JSON IR fetch failed, trying PUML fallback:`, jsonErr);
      }

      // ── 2. Fallback: Parse PUML if JSON is absent ──
      if (!elements || elements.length === 0) {
        try {
          const pumlRes = await fetch(`diagrams/${graphType}.puml`);
          if (pumlRes.ok) {
            const pumlText = await pumlRes.text();
            elements = parsePumlToCytoscape(pumlText, graphType);
          }
        } catch (pumlErr) {
          console.warn(`[OpenHeart Pipeline] PUML fallback failed:`, pumlErr);
        }
      }
    }

    if (!elements || elements.length === 0) {
      elements = [
        { data: { id: 'root', label: `<<diagram>>\n${graphType.toUpperCase()}\n──────────────────────\nCompiled Live from Source`, kind: 'entry', width: 260, height: 80, file: 'VideoConversionFacade.java', lines: [1] } }
      ];
    }

    // Compute exact collision-free coordinates across all 3 tiers
    const layoutElements = computeDeterministicLayout(elements, graphType);

    // Safety filter: Guarantee zero orphan edges so Cytoscape never crashes on external SDK symbols
    const nodeIds = new Set(layoutElements.filter(e => e.data && !e.data.source).map(e => e.data.id));
    const safeElements = layoutElements.filter(e => {
      if (e.data && e.data.source) {
        return nodeIds.has(e.data.source) && nodeIds.has(e.data.target);
      }
      return true;
    });

    this.cy = cytoscape({
      container: container,
      elements: safeElements,
      boxSelectionEnabled: false,
      autounselectify: false,
      userZoomingEnabled: false,
      userPanningEnabled: true,
      minZoom: 0.05,
      maxZoom: 4.0,
      pixelRatio: 'auto',
      textureOnViewport: true,
      hideEdgesOnViewport: false,
      motionBlur: false,
      wheelSensitivity: 0.05,
      style: this.getModernStyleSheet(),
      layout: {
        name: 'preset',
        animate: false
      }
    });

    this.attachEventListeners(container);
    this.cy.fit(undefined, 60);

    if (this.onRenderCompleteCallback) {
      this.onRenderCompleteCallback(elements);
    }

    if (this.onLayersUpdateCallback) {
      this.onLayersUpdateCallback(this.getActiveEdgeKinds());
    }
  }

  setTheme(isDark) {
    if (this.cy) {
      this.cy.style(buildCytoscapeStylesheet(getTheme(isDark)));
    }
    this.renderGraph(this.currentGraphType);
  }

  focusNodeByFile(fileName) {
    if (!this.cy) return;
    const node = this.cy.nodes().filter(n => n.data('file') === fileName)[0];
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

  getModernStyleSheet() {
    return buildCytoscapeStylesheet();
  }

  setLayersUpdateCallback(cb) {
    this.onLayersUpdateCallback = cb;
  }

  setEdgeFilter(umlKind, isVisible) {
    if (!this.cy) return;
    if (isVisible) {
      this.hiddenEdgeKinds.delete(umlKind);
    } else {
      this.hiddenEdgeKinds.add(umlKind);
    }

    this.cy.batch(() => {
      const edges = this.cy.edges(`[uml_kind = "${umlKind}"]`);
      if (isVisible) {
        edges.style('display', 'element');
      } else {
        edges.style('display', 'none');
      }
    });
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

  attachEventListeners(container) {
    if (!this.cy || !container) return;

    // ── Two-Finger Trackpad Pan vs Pinch-to-Zoom ──
    container.addEventListener('wheel', (e) => {
      e.preventDefault();

      if (e.ctrlKey || e.metaKey) {
        // Pinch gesture or Ctrl+Wheel -> Gentle, ultra-smooth zoom centered at cursor
        const zoomFactor = Math.exp(-e.deltaY * 0.003);
        const currentZoom = this.cy.zoom();
        const newZoom = Math.min(4.0, Math.max(0.05, currentZoom * zoomFactor));
        const rect = container.getBoundingClientRect();
        const renderedPos = {
          x: e.clientX - rect.left,
          y: e.clientY - rect.top
        };

        this.cy.zoom({
          level: newZoom,
          renderedPosition: renderedPos
        });
      } else {
        // Two-Finger Trackpad Gesture / Scroll -> Pan canvas in 2D
        const panSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        this.cy.panBy({
          x: -e.deltaX * panSensitivity,
          y: -e.deltaY * panSensitivity
        });
      }
    }, { passive: false });

    // ── Multi-Touch Support (2-Finger Pinch & Pan for Touchscreens) ──
    let touchStartDist = 0;
    let touchStartCenter = null;
    let touchStartZoom = 1;

    container.addEventListener('touchstart', (e) => {
      if (e.touches.length === 2) {
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
      if (e.touches.length === 2 && touchStartDist > 0) {
        e.preventDefault();
        const t1 = e.touches[0];
        const t2 = e.touches[1];
        const currentDist = Math.hypot(t2.clientX - t1.clientX, t2.clientY - t1.clientY);
        const currentCenter = {
          x: (t1.clientX + t2.clientX) / 2,
          y: (t1.clientY + t2.clientY) / 2
        };

        // 1. Pinch to Zoom with gentle damping
        const scale = 1 + (currentDist / touchStartDist - 1) * 0.5;
        const newZoom = Math.min(4.0, Math.max(0.05, touchStartZoom * scale));
        const rect = container.getBoundingClientRect();

        this.cy.zoom({
          level: newZoom,
          renderedPosition: {
            x: currentCenter.x - rect.left,
            y: currentCenter.y - rect.top
          }
        });

        // 2. Pure Two-Finger Pan
        const panSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        const deltaX = (currentCenter.x - touchStartCenter.x) * panSensitivity;
        const deltaY = (currentCenter.y - touchStartCenter.y) * panSensitivity;
        this.cy.panBy({ x: deltaX, y: deltaY });
        touchStartCenter = currentCenter;
      }
    }, { passive: false });

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
  }

  togglePackageCollapse(pkgNode) {
    const pkgId = pkgNode.id();
    const children = this.cy.nodes(`[parent = "${pkgId}"]`);
    const isCollapsed = this.collapsedPackages.has(pkgId);
    const rawName = pkgNode.data('rawName') || pkgId.replace(/^pkg_/, '').replace(/_/g, '.');
    const shortName = rawName.split('.').pop();
    const isDomainTier = pkgNode.data('isDomainTier');

    this.cy.batch(() => {
      if (isCollapsed) {
        // Expand (Open)
        this.collapsedPackages.delete(pkgId);
        pkgNode.removeClass('package-collapsed');
        pkgNode.data('width', pkgNode.data('origWidth') || 650);
        pkgNode.data('height', pkgNode.data('origHeight') || 400);
        pkgNode.data('label', isDomainTier ? `📂 [−] DOMAIN LAYER: ${rawName.toUpperCase()}` : `📂 [−] package [${shortName}]`);
        children.style('display', 'element');
        children.connectedEdges().style('display', 'element');
      } else {
        // Collapse (Close)
        this.collapsedPackages.add(pkgId);
        pkgNode.addClass('package-collapsed');
        pkgNode.data('label', isDomainTier ? `📁 [+] DOMAIN LAYER: ${rawName.toUpperCase()} (${children.length} subpackages)` : `📁 [+] package [${shortName}] (${children.length} classes)`);
        children.style('display', 'none');
        children.connectedEdges().style('display', 'none');
      }
    });
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
}
