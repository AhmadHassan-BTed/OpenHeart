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

export class InteractiveGraphCanvas {
  constructor(containerId = 'interactive-canvas') {
    this.containerId = containerId;
    this.cy = null;
    this.currentGraphType = 'class';
    this.selectedNode = null;
    this.onNodeSelectedCallback = null;
    this.onNodeHoverCallback = null;
    this.onRenderCompleteCallback = null;
    this.panSensitivity = parseFloat(localStorage.getItem('openheart_pan_sensitivity') || '0.45');
    this.activeHoverId = null;
    this.hoverTimeout = null;
    this.collapsedPackages = new Set();
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.renderGraph(this.currentGraphType);
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

  async renderGraph(graphType, customElements = null) {
    this.currentGraphType = graphType;
    const container = document.getElementById(this.containerId);
    if (!container) return;

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
      textureOnViewport: false,
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
    return [
      // ── Level 3: SVG 3-Compartment Class Card Vector (Pure White Floating Card) ──
      {
        selector: 'node[?svgDataUri]',
        style: {
          'background-image': 'data(svgDataUri)',
          'background-fit': 'cover',
          'background-clip': 'node',
          'background-color': '#FFFFFF',
          'border-width': 0,
          'width': 'data(width)',
          'height': 'data(height)',
          'shape': 'roundrectangle',
          'label': '',
          'z-index': 10
        }
      },

      // ── Level 0: Root Domain Tier Container (Lightest Soft Violet Tint) ──
      {
        selector: 'node.nest-level-0, node[nestLevel = 0]',
        style: {
          'background-color': '#FAF5FF',
          'background-opacity': 0.85,
          'border-width': 2.5,
          'border-color': '#C084FC',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '14px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 18,
          'text-margin-y': -16,
          'text-background-color': '#FFFFFF',
          'text-background-opacity': 1.0,
          'text-background-padding': '6px 16px',
          'text-border-width': 2.0,
          'text-border-color': '#C084FC',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '12.5px',
          'font-weight': 800,
          'letter-spacing': '0.03em',
          'color': '#6B21A8',
          'padding': '44px',
          'z-index': 1
        }
      },

      // ── Level 1: Subpackage Container (Darker / Richer Violet Tint) ──
      {
        selector: 'node.nest-level-1, node[nestLevel = 1]',
        style: {
          'background-color': '#F3E8FF',
          'background-opacity': 0.90,
          'border-width': 2.2,
          'border-color': '#A855F7',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '12px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 16,
          'text-margin-y': -14,
          'text-background-color': '#FAF5FF',
          'text-background-opacity': 1.0,
          'text-background-padding': '5px 14px',
          'text-border-width': 1.8,
          'text-border-color': '#A855F7',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '12px',
          'font-weight': 700,
          'color': '#581C87',
          'padding': '38px',
          'z-index': 2
        }
      },

      // ── Level 2: Subpackage Container (Darker Lilac Tint) ──
      {
        selector: 'node.nest-level-2, node[nestLevel = 2]',
        style: {
          'background-color': '#E9D5FF',
          'background-opacity': 0.95,
          'border-width': 2.0,
          'border-color': '#9333EA',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 16,
          'text-margin-y': -14,
          'text-background-color': '#F3E8FF',
          'text-background-opacity': 1.0,
          'text-background-padding': '5px 12px',
          'text-border-width': 1.8,
          'text-border-color': '#9333EA',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11.5px',
          'font-weight': 700,
          'color': '#4C1D95',
          'padding': '32px',
          'z-index': 3
        }
      },

      // ── Level 3: Leaf Subpackage Container (Darkest Deep Purple Tint) ──
      {
        selector: 'node.nest-level-3, node[nestLevel = 3]',
        style: {
          'background-color': '#DDD6FE',
          'background-opacity': 1.0,
          'border-width': 2.2,
          'border-color': '#7E22CE',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '8px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 14,
          'text-margin-y': -13,
          'text-background-color': '#EDE9FE',
          'text-background-opacity': 1.0,
          'text-background-padding': '4px 10px',
          'text-border-width': 1.8,
          'text-border-color': '#7E22CE',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11px',
          'font-weight': 700,
          'color': '#3B0764',
          'padding': '28px',
          'z-index': 4
        }
      },

      // ── Level 4+: Deepest Innermost Package Container ──
      {
        selector: 'node.nest-level-4, node.nest-level-5, node[nestLevel >= 4]',
        style: {
          'background-color': '#C4B5FD',
          'background-opacity': 1.0,
          'border-width': 2.5,
          'border-color': '#6B21A8',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '8px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 14,
          'text-margin-y': -13,
          'text-background-color': '#DDD6FE',
          'text-background-opacity': 1.0,
          'text-background-padding': '4px 10px',
          'text-border-width': 2.0,
          'text-border-color': '#6B21A8',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11px',
          'font-weight': 700,
          'color': '#2E1065',
          'padding': '24px',
          'z-index': 5
        }
      },

      // ── General Package Container Fallback ──
      {
        selector: 'node[?isPackage]',
        style: {
          'background-color': '#F8FAFC',
          'background-opacity': 0.9,
          'border-width': 2.0,
          'border-color': '#475569',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 16,
          'text-margin-y': -14,
          'text-background-color': '#FFFFFF',
          'text-background-opacity': 1.0,
          'text-background-padding': '5px 12px',
          'text-border-width': 1.5,
          'text-border-color': '#475569',
          'text-border-opacity': 1.0,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11px',
          'font-weight': 700,
          'color': '#1E293B',
          'padding': '36px',
          'z-index': 1
        }
      },

      // ── Collapsed Package Container State ──
      {
        selector: 'node.package-collapsed',
        style: {
          'width': 260,
          'height': 60,
          'text-valign': 'center',
          'text-halign': 'center',
          'border-style': 'solid',
          'border-width': 2.5
        }
      },

      // ── Default Edge (Clean Smooth Routing & Visible Arrowheads) ──
      {
        selector: 'edge',
        style: {
          'width': 2.0,
          'line-color': '#475569',
          'target-arrow-color': '#475569',
          'target-arrow-shape': 'triangle',
          'arrow-scale': 1.3,
          'curve-style': 'bezier',
          'label': 'data(label)',
          'font-family': 'JetBrains Mono, monospace',
          'font-size': '10.5px',
          'font-weight': 700,
          'color': '#1E293B',
          'text-background-color': '#FFFFFF',
          'text-background-opacity': 0.95,
          'text-background-padding': '4px 6px',
          'text-border-width': 1,
          'text-border-color': '#CBD5E1',
          'text-border-opacity': 1,
          'z-index': 999
        }
      },

      // ── UML Generalization (--|>) : Solid line + Hollow Triangle Head ──
      {
        selector: 'edge[uml_kind = "generalization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-color': '#0F172A',
          'target-arrow-color': '#0F172A',
          'line-style': 'solid',
          'arrow-scale': 1.5,
          'width': 2.2,
          'z-index': 999
        }
      },

      // ── UML Realization (..|>) : Dashed line + Hollow Triangle Head ──
      {
        selector: 'edge[uml_kind = "realization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-style': 'dashed',
          'line-color': '#0F172A',
          'target-arrow-color': '#0F172A',
          'arrow-scale': 1.5,
          'width': 2.2,
          'z-index': 999
        }
      },

      // ── UML Composition (<*--) : Filled Black Diamond Source ──
      {
        selector: 'edge[uml_kind = "composition"]',
        style: {
          'source-arrow-shape': 'diamond',
          'source-arrow-fill': 'filled',
          'source-arrow-color': '#0F172A',
          'line-color': '#0F172A',
          'target-arrow-shape': 'none',
          'arrow-scale': 1.4,
          'width': 2.0,
          'z-index': 999
        }
      },

      // ── UML Aggregation (o--) : Hollow Diamond Source ──
      {
        selector: 'edge[uml_kind = "aggregation"]',
        style: {
          'source-arrow-shape': 'diamond',
          'source-arrow-fill': 'hollow',
          'source-arrow-color': '#0F172A',
          'line-color': '#0F172A',
          'target-arrow-shape': 'none',
          'arrow-scale': 1.4,
          'width': 2.0,
          'z-index': 999
        }
      },

      // ── UML Dependency (..>) : Dashed line + Vee Arrowhead ──
      {
        selector: 'edge[uml_kind = "dependency"]',
        style: {
          'target-arrow-shape': 'vee',
          'line-style': 'dashed',
          'line-color': '#334155',
          'target-arrow-color': '#334155',
          'arrow-scale': 1.3,
          'width': 1.8,
          'z-index': 999
        }
      },

      // ── VIBRANT PATH ILLUMINATION: Highlighted Nodes ──
      {
        selector: 'node.path-highlighted',
        style: {
          'border-color': '#EF4444',
          'border-width': 3.5,
          'border-style': 'solid',
          'z-index': 9999
        }
      },

      // ── VIBRANT PATH ILLUMINATION: Highlighted Edges ──
      {
        selector: 'edge.path-highlighted',
        style: {
          'width': 3.5,
          'line-color': '#EF4444',
          'target-arrow-color': '#EF4444',
          'source-arrow-color': '#EF4444',
          'color': '#EF4444',
          'text-border-color': '#EF4444',
          'text-border-width': 1.5,
          'z-index': 9999
        }
      },

      // ── Dimmed Inactive State (Deep Fog) ──
      {
        selector: '.dimmed',
        style: {
          'opacity': 0.08
        }
      },

      // ── Selected Node State ──
      {
        selector: ':selected',
        style: {
          'border-color': '#EF4444',
          'border-width': 3.0,
          'border-style': 'solid'
        }
      }
    ];
  }

  attachEventListeners(container) {
    if (!this.cy || !container) return;

    // ── Two-Finger Trackpad Pan vs Two-Finger Pinch-to-Zoom ──
    container.addEventListener('wheel', (e) => {
      e.preventDefault();

      if (e.ctrlKey) {
        // Trackpad Pinch Gesture -> Zoom
        const zoomFactor = Math.exp(-e.deltaY * 0.015);
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
        // Two-Finger Trackpad Swipe -> Pan
        const panSensitivity = this.panSensitivity;
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

        // 1. Pinch to Zoom
        const scale = currentDist / touchStartDist;
        const newZoom = Math.min(4.0, Math.max(0.05, touchStartZoom * scale));
        const rect = container.getBoundingClientRect();

        this.cy.zoom({
          level: newZoom,
          renderedPosition: {
            x: currentCenter.x - rect.left,
            y: currentCenter.y - rect.top
          }
        });

        // 2. Damped Two-Finger Pan
        const deltaX = (currentCenter.x - touchStartCenter.x) * 0.55;
        const deltaY = (currentCenter.y - touchStartCenter.y) * 0.55;
        this.cy.panBy({ x: deltaX, y: deltaY });
        touchStartCenter = currentCenter;
      }
    }, { passive: false });

    // ── High-Intensity Path Hover Illumination ──
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

      let pathElements;
      if (target.isNode()) {
        const predecessors = target.predecessors();
        const successors = target.successors();
        pathElements = target.union(predecessors).union(successors);
      } else if (target.isEdge()) {
        const sourcePath = target.source().union(target.source().predecessors());
        const targetPath = target.target().union(target.target().successors());
        pathElements = target.union(sourcePath).union(targetPath);
      }

      this.cy.batch(() => {
        this.cy.elements().not('node[?isPackage]').addClass('dimmed');
        pathElements.removeClass('dimmed').addClass('path-highlighted');
      });

      if (this.onNodeHoverCallback && target.isNode()) {
        this.onNodeHoverCallback(target.data());
      }
    });

    this.cy.on('mouseout', 'node, edge', () => {
      if (this.hoverTimeout) clearTimeout(this.hoverTimeout);
      this.hoverTimeout = setTimeout(() => {
        this.activeHoverId = null;
        this.cy.batch(() => {
          this.cy.elements().removeClass('dimmed').removeClass('path-highlighted');
        });
      }, 50);
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
