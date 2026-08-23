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

export class InteractiveGraphCanvas {
  constructor(containerId = 'interactive-canvas') {
    this.containerId = containerId;
    this.cy = null;
    this.currentGraphType = 'class';
    this.selectedNode = null;
    this.onNodeSelectedCallback = null;
    this.onNodeHoverCallback = null;
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
      try {
        const response = await fetch(`diagrams/${graphType}.puml`);
        if (response.ok) {
          const pumlText = await response.text();
          elements = parsePumlToCytoscape(pumlText, graphType);
        }
      } catch (err) {
        console.warn(`[OpenHeart Canvas] Failed to fetch diagrams/${graphType}.puml:`, err);
      }
    }

    if (!elements || elements.length === 0) {
      elements = [
        { data: { id: 'root', label: `<<diagram>>\n${graphType.toUpperCase()}\n──────────────────────\nCompiled Live from Source`, kind: 'entry', width: 260, height: 80, file: 'VideoConversionFacade.java', lines: [1] } }
      ];
    }

    // Compute exact collision-free coordinates across all 3 tiers
    const layoutElements = computeDeterministicLayout(elements, graphType);

    this.cy = cytoscape({
      container: container,
      elements: layoutElements,
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

      // ── Level 1: Behavioral Domain Tier Container (Soft Violet Layer) ──
      {
        selector: 'node.pkg-domain-tier.pkg-behavioral, node[?isDomainTier][category = "pkg-behavioral"]',
        style: {
          'background-color': '#FAF5FF',
          'background-opacity': 1.0,
          'border-width': 2.5,
          'border-color': '#C084FC',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '16px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 24,
          'text-margin-y': 20,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '13px',
          'font-weight': 800,
          'letter-spacing': '0.04em',
          'color': '#6B21A8',
          'padding': '40px',
          'z-index': 1
        }
      },

      // ── Level 1: Creational Domain Tier Container (Soft Emerald Layer) ──
      {
        selector: 'node.pkg-domain-tier.pkg-creational, node[?isDomainTier][category = "pkg-creational"]',
        style: {
          'background-color': '#F0FDF4',
          'background-opacity': 1.0,
          'border-width': 2.5,
          'border-color': '#86EFAC',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '16px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 24,
          'text-margin-y': 20,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '13px',
          'font-weight': 800,
          'letter-spacing': '0.04em',
          'color': '#065F46',
          'padding': '40px',
          'z-index': 1
        }
      },

      // ── Level 1: Structural Domain Tier Container (Soft Sky Blue Layer) ──
      {
        selector: 'node.pkg-domain-tier.pkg-structural, node[?isDomainTier][category = "pkg-structural"]',
        style: {
          'background-color': '#F0F9FF',
          'background-opacity': 1.0,
          'border-width': 2.5,
          'border-color': '#7DD3FC',
          'border-style': 'solid',
          'shape': 'roundrectangle',
          'border-radius': '16px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 24,
          'text-margin-y': 20,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '13px',
          'font-weight': 800,
          'letter-spacing': '0.04em',
          'color': '#075985',
          'padding': '40px',
          'z-index': 1
        }
      },

      // ── Level 2: Behavioral Subpackage Container (Richer Violet Tint) ──
      {
        selector: 'node.pkg-subpackage.pkg-behavioral, node[!isDomainTier].pkg-behavioral',
        style: {
          'background-color': '#F3E8FF',
          'background-opacity': 1.0,
          'border-width': 2.0,
          'border-color': '#9333EA',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 18,
          'text-margin-y': 14,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11.5px',
          'font-weight': 700,
          'color': '#581C87',
          'padding': '30px',
          'z-index': 3
        }
      },

      // ── Level 2: Creational Subpackage Container (Richer Emerald Tint) ──
      {
        selector: 'node.pkg-subpackage.pkg-creational, node[!isDomainTier].pkg-creational',
        style: {
          'background-color': '#DCFCE7',
          'background-opacity': 1.0,
          'border-width': 2.0,
          'border-color': '#059669',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 18,
          'text-margin-y': 14,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11.5px',
          'font-weight': 700,
          'color': '#064E3B',
          'padding': '30px',
          'z-index': 3
        }
      },

      // ── Level 2: Structural Subpackage Container (Richer Sky Blue Tint) ──
      {
        selector: 'node.pkg-subpackage.pkg-structural, node[!isDomainTier].pkg-structural',
        style: {
          'background-color': '#E0F2FE',
          'background-opacity': 1.0,
          'border-width': 2.0,
          'border-color': '#0284C7',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 18,
          'text-margin-y': 14,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11.5px',
          'font-weight': 700,
          'color': '#0C4A6E',
          'padding': '30px',
          'z-index': 3
        }
      },

      // ── General Package Container Fallback ──
      {
        selector: 'node.compound-package, node[?isPackage]',
        style: {
          'background-color': '#F8FAFC',
          'background-opacity': 1.0,
          'border-width': 2.0,
          'border-color': '#64748B',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '12px',
          'text-valign': 'top',
          'text-halign': 'left',
          'text-margin-x': 18,
          'text-margin-y': 14,
          'font-family': 'JetBrains Mono, -apple-system, sans-serif',
          'font-size': '11px',
          'font-weight': 700,
          'color': '#334155',
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

      // ── Default Edge (Orthogonal Clean Taxi Routing) ──
      {
        selector: 'edge',
        style: {
          'width': 1.5,
          'line-color': '#64748B',
          'target-arrow-color': '#64748B',
          'target-arrow-shape': 'triangle',
          'arrow-scale': 1.0,
          'curve-style': 'taxi',
          'taxi-direction': 'auto',
          'taxi-turn': '28px',
          'taxi-turn-min-distance': '12px',
          'label': 'data(label)',
          'font-family': 'JetBrains Mono, monospace',
          'font-size': '10px',
          'font-weight': 600,
          'color': '#334155',
          'text-background-color': '#FFFFFF',
          'text-background-opacity': 1.0,
          'text-background-padding': '4px',
          'text-border-width': 1,
          'text-border-color': '#CBD5E1',
          'text-border-opacity': 1,
          'z-index': 5
        }
      },

      // ── UML Generalization (--|>) : Solid line + Hollow Triangle Head ──
      {
        selector: 'edge[uml_kind = "generalization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-color': '#1E293B',
          'target-arrow-color': '#1E293B',
          'line-style': 'solid'
        }
      },

      // ── UML Realization (..|>) : Dashed line + Hollow Triangle Head ──
      {
        selector: 'edge[uml_kind = "realization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-style': 'dashed',
          'line-color': '#1E293B',
          'target-arrow-color': '#1E293B'
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
          'target-arrow-shape': 'none'
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
          'target-arrow-shape': 'none'
        }
      },

      // ── UML Dependency (..>) : Dashed line + Vee Arrowhead ──
      {
        selector: 'edge[uml_kind = "dependency"]',
        style: {
          'target-arrow-shape': 'vee',
          'line-style': 'dashed',
          'line-color': '#64748B',
          'target-arrow-color': '#64748B'
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
        const panSensitivity = 0.45;
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

    this.cy.batch(() => {
      if (isCollapsed) {
        // Expand (Open)
        this.collapsedPackages.delete(pkgId);
        pkgNode.removeClass('package-collapsed');
        pkgNode.data('width', pkgNode.data('origWidth') || 650);
        pkgNode.data('height', pkgNode.data('origHeight') || 400);
        pkgNode.data('label', `package [${pkgId.replace(/^pkg_/, '').replace(/_/g, '.')}]`);
        children.style('display', 'element');
        children.connectedEdges().style('display', 'element');
      } else {
        // Collapse (Close)
        this.collapsedPackages.add(pkgId);
        pkgNode.addClass('package-collapsed');
        pkgNode.data('label', `[+] package [${pkgId.replace(/^pkg_/, '').replace(/_/g, '.')}] (${children.length} classes)`);
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
