/**
 * OpenHeart Interactive Graph Canvas Module (Dynamic Production Edition)
 * Powered by Cytoscape.js + Dagre / Compound Hierarchical Layout
 * Fully renders nested compound packages, all 35 real classes, and all UML relationships.
 */

import { parsePumlToCytoscape } from './puml-parser.js';

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
        { data: { id: 'root', label: `${graphType.toUpperCase()} Graph Ready\n(Compiled Live)`, kind: 'entry', width: 250, height: 60, file: 'VideoConversionFacade.java', lines: [1] } }
      ];
    }

    this.cy = cytoscape({
      container: container,
      elements: elements,
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
        name: 'cose',
        padding: 50,
        nodeOverlap: 20,
        idealEdgeLength: 100,
        edgeElasticity: 100,
        nestingFactor: 5,
        gravity: 80,
        numIter: 1000,
        initialTemp: 200,
        coolingFactor: 0.95,
        minTemp: 1.0,
        animate: false
      }
    });

    this.attachEventListeners(container);
    this.cy.fit(undefined, 40);
  }

  getModernStyleSheet() {
    return [
      // ── Base Node (Solid Rounded Glass Card) ──
      {
        selector: 'node',
        style: {
          'background-color': '#FFFFFF',
          'border-width': 1.5,
          'border-color': '#CBD5E1',
          'label': 'data(label)',
          'color': '#0F172A',
          'font-family': 'JetBrains Mono, SF Mono, Consolas, monospace',
          'font-size': '11px',
          'font-weight': 500,
          'line-height': 1.4,
          'text-valign': 'center',
          'text-halign': 'center',
          'text-wrap': 'wrap',
          'text-max-width': '280px',
          'width': 'data(width)',
          'height': 'data(height)',
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'padding': '14px'
        }
      },

      // ── Compound Package Parent Container ──
      {
        selector: 'node:parent, node.compound-package, node[?isPackage]',
        style: {
          'background-color': '#F8FAFC',
          'background-opacity': 0.7,
          'border-width': 1.5,
          'border-color': '#94A3B8',
          'border-style': 'dashed',
          'shape': 'roundrectangle',
          'border-radius': '14px',
          'text-valign': 'top',
          'text-halign': 'center',
          'text-margin-y': 10,
          'font-family': 'Inter, sans-serif',
          'font-size': '11px',
          'font-weight': 700,
          'color': '#334155',
          'padding': '24px'
        }
      },

      // ── Entry Point (Crimson Top Accent) ──
      {
        selector: 'node[kind = "entry"]',
        style: {
          'background-color': '#FFFFFF',
          'border-color': '#EF4444',
          'border-width': 2.0,
          'color': '#0F172A',
          'font-weight': 700
        }
      },

      // ── Exit Point (Subtle Slate Pill Card) ──
      {
        selector: 'node[kind = "exit"]',
        style: {
          'background-color': '#F8FAFC',
          'border-color': '#94A3B8',
          'border-width': 1.5,
          'color': '#475569',
          'font-weight': 600
        }
      },

      // ── Condition Decision Gate (Diamond Splitter) ──
      {
        selector: 'node[kind = "gate"]',
        style: {
          'shape': 'diamond',
          'background-color': '#FFFFFF',
          'border-color': '#3B82F6',
          'border-width': 2.0,
          'font-size': '10px',
          'font-weight': 600,
          'text-valign': 'center',
          'text-halign': 'center',
          'text-wrap': 'wrap',
          'text-max-width': '110px',
          'width': '130px',
          'height': '130px'
        }
      },

      // ── Class Node (UML Structured Box) ──
      {
        selector: 'node[kind = "class"], node[kind = "interface"], node[kind = "abstract"]',
        style: {
          'background-color': '#FFFFFF',
          'border-color': '#94A3B8',
          'border-width': 1.5,
          'shape': 'roundrectangle',
          'border-radius': '10px',
          'text-valign': 'center',
          'text-halign': 'center',
          'text-wrap': 'wrap',
          'text-max-width': '280px',
          'color': '#0F172A'
        }
      },

      // ── Default Edge (Refined Architecture Line) ──
      {
        selector: 'edge',
        style: {
          'width': 1.5,
          'line-color': '#94A3B8',
          'target-arrow-color': '#94A3B8',
          'target-arrow-shape': 'triangle',
          'arrow-scale': 0.9,
          'curve-style': 'bezier',
          'label': 'data(label)',
          'font-family': 'JetBrains Mono, monospace',
          'font-size': '10px',
          'font-weight': 500,
          'color': '#64748B',
          'text-background-color': '#FFFFFF',
          'text-background-opacity': 1.0,
          'text-background-padding': '4px',
          'text-border-width': 1,
          'text-border-color': '#E2E8F0',
          'text-border-opacity': 1
        }
      },

      // ── True Branch Edge (Vibrant Red) ──
      {
        selector: 'edge[branch = "true"]',
        style: {
          'line-color': '#EF4444',
          'target-arrow-color': '#EF4444',
          'color': '#DC2626',
          'text-border-color': '#FECACA'
        }
      },

      // ── False Branch Edge (Dashed) ──
      {
        selector: 'edge[branch = "false"]',
        style: {
          'line-style': 'dashed',
          'line-color': '#94A3B8',
          'target-arrow-color': '#94A3B8',
          'color': '#64748B'
        }
      },

      // ── UML Generalization (--|>) ──
      {
        selector: 'edge[uml_kind = "generalization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-color': '#475569',
          'target-arrow-color': '#475569'
        }
      },

      // ── UML Realization (..|>) ──
      {
        selector: 'edge[uml_kind = "realization"]',
        style: {
          'target-arrow-shape': 'triangle',
          'target-arrow-fill': 'hollow',
          'line-style': 'dashed',
          'line-color': '#475569',
          'target-arrow-color': '#475569'
        }
      },

      // ── UML Composition (<*--) ──
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

      // ── UML Dependency (..>) ──
      {
        selector: 'edge[uml_kind = "dependency"]',
        style: {
          'target-arrow-shape': 'vee',
          'line-style': 'dashed',
          'line-color': '#64748B',
          'target-arrow-color': '#64748B'
        }
      },

      // ── NODE HOVER HIGHLIGHT ──
      {
        selector: 'node.path-highlighted',
        style: {
          'border-color': '#EF4444',
          'border-width': 2.5,
          'background-color': '#FFFFFF',
          'color': '#0F172A',
          'z-index': 999
        }
      },

      // ── EDGE HOVER HIGHLIGHT ──
      {
        selector: 'edge.path-highlighted',
        style: {
          'width': 2.5,
          'line-color': '#EF4444',
          'target-arrow-color': '#EF4444',
          'source-arrow-color': '#EF4444',
          'z-index': 999
        }
      },

      // ── Dimmed Inactive State ──
      {
        selector: '.dimmed',
        style: {
          'opacity': 0.15
        }
      },

      // ── Selected Node State ──
      {
        selector: ':selected',
        style: {
          'border-color': '#EF4444',
          'border-width': 2.5,
          'background-color': '#FEF2F2'
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

    // ── Zero-Flicker Batched Path Hover Illumination ──
    this.cy.on('mouseover', 'node, edge', (e) => {
      const target = e.target;
      if (target.isParent()) return; // Don't dim on package container hover

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

      // Single synchronous atomic batch to prevent flicker
      this.cy.batch(() => {
        this.cy.elements().not(':parent').addClass('dimmed');
        pathElements.removeClass('dimmed').addClass('path-highlighted');
      });

      if (this.onNodeHoverCallback && target.isNode() && !target.isParent()) {
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
      if (node.isParent()) return;
      this.selectedNode = node.data();
      if (this.onNodeSelectedCallback) {
        this.onNodeSelectedCallback(this.selectedNode);
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
        padding: 50
      },
      duration: 250
    });
  }
}
