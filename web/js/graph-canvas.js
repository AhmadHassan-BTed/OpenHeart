/**
 * OpenHeart Precision Interactive Graph Canvas
 * Strictly conforms to UML 2.5 standard symbolic notation:
 *  - 3-Compartment Class / Interface / Abstract Symbols
 *  - Folder-Tab Compound Package Containers
 *  - Orthogonal Taxi Wiring for Clean, Non-Entangled Routing
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
        { data: { id: 'root', label: `<<diagram>>\n${graphType.toUpperCase()}\n──────────────────────\nCompiled Live from Source`, kind: 'entry', width: 260, height: 80, file: 'VideoConversionFacade.java', lines: [1] } }
      ];
    }

    const isHierarchical = ['cfg', 'robdd', 'callgraph', 'statemachine', 'activity', 'sequence'].includes(graphType);

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
      layout: isHierarchical ? {
        name: 'breadthfirst',
        directed: true,
        padding: 60,
        spacingFactor: 1.45,
        animate: false
      } : {
        name: 'cose',
        nodeDimensionsIncludeLabels: true,
        padding: 90,
        nodeOverlap: 90,
        idealEdgeLength: 220,
        edgeElasticity: 0.25,
        nestingFactor: 0.1,
        gravity: 15,
        numIter: 1600,
        initialTemp: 400,
        coolingFactor: 0.95,
        minTemp: 1.0,
        nodeRepulsion: function(node) {
          return node.isParent() ? 3500000 : 1200000;
        },
        animate: false
      }
    });

    this.attachEventListeners(container);
    this.cy.fit(undefined, 40);
  }

  getModernStyleSheet() {
    return [
      // ── Base Class Node (UML 2.5 3-Compartment Card) ──
      {
        selector: 'node',
        style: {
          'background-color': '#FFFFFF',
          'border-width': 1.5,
          'border-color': '#94A3B8',
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
          'border-radius': '6px',
          'padding': '14px'
        }
      },

      // ── Interface Symbol (Blue Accent Header) ──
      {
        selector: 'node[kind = "interface"]',
        style: {
          'background-color': '#FFFFFF',
          'border-color': '#3B82F6',
          'border-width': 1.5,
          'color': '#0F172A'
        }
      },

      // ── Abstract Class Symbol (Indigo Accent Header) ──
      {
        selector: 'node[kind = "abstract"]',
        style: {
          'background-color': '#FFFFFF',
          'border-color': '#6366F1',
          'border-width': 1.5,
          'color': '#0F172A'
        }
      },

      // ── Level 0 Package Container (Outer Namespace) ──
      {
        selector: 'node.compound-package, node[?isPackage]',
        style: {
          'background-color': '#F8FAFC',
          'background-opacity': 0.85,
          'border-width': 1.5,
          'border-color': '#94A3B8',
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
          'padding': '36px'
        }
      },

      // ── Level 1 Sub-Package Container (Deeper Slate Tint) ──
      {
        selector: 'node.nest-level-1, node[nestLevel = 1]',
        style: {
          'background-color': '#F1F5F9',
          'background-opacity': 0.92,
          'border-color': '#64748B',
          'border-width': 1.5,
          'color': '#1E293B',
          'padding': '30px'
        }
      },

      // ── Level 2 Nested Sub-Module Container (Blue-Gray Tint) ──
      {
        selector: 'node.nest-level-2, node[nestLevel = 2]',
        style: {
          'background-color': '#E2E8F0',
          'background-opacity': 0.95,
          'border-color': '#475569',
          'border-width': 2.0,
          'color': '#0F172A',
          'padding': '26px'
        }
      },

      // ── Level 3+ Deep Nested Container (Rich Slate Frame) ──
      {
        selector: 'node.nest-level-3, node[nestLevel = 3]',
        style: {
          'background-color': '#CBD5E1',
          'background-opacity': 1.0,
          'border-color': '#334155',
          'border-width': 2.0,
          'color': '#020617',
          'padding': '22px'
        }
      },

      // ── Entry / Initial Node ──
      {
        selector: 'node[kind = "entry"]',
        style: {
          'background-color': '#FFFFFF',
          'border-color': '#EF4444',
          'border-width': 2.0,
          'font-weight': 700
        }
      },

      // ── Exit / Final Node ──
      {
        selector: 'node[kind = "exit"]',
        style: {
          'background-color': '#F8FAFC',
          'border-color': '#94A3B8',
          'border-width': 1.5,
          'color': '#475569'
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

      // ── Default Edge (Orthogonal Clean Taxi Routing) ──
      {
        selector: 'edge',
        style: {
          'width': 1.5,
          'line-color': '#94A3B8',
          'target-arrow-color': '#94A3B8',
          'target-arrow-shape': 'triangle',
          'arrow-scale': 0.9,
          'curve-style': 'taxi',
          'taxi-direction': 'auto',
          'taxi-turn': '24px',
          'taxi-turn-min-distance': '8px',
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
          'opacity': 0.12
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
      if (target.isParent()) return;

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
