/**
 * OpenHeart Theme Manager
 * Unifies Light and Dark themes, generating Cytoscape stylesheets dynamically.
 */
import { LightTheme } from './light.js';
import { DarkTheme } from './dark.js';

export function getTheme(isDark = null) {
  if (isDark === null) {
    isDark = typeof document !== 'undefined' && document.body && (
      document.body.classList.contains('dark-theme') ||
      document.documentElement.getAttribute('data-theme') === 'dark'
    );
  }
  return isDark ? DarkTheme : LightTheme;
}

export function buildCytoscapeStylesheet(theme = null) {
  if (!theme) {
    theme = getTheme();
  }

  const { packages: pkgs, edges } = theme;

  return [
    // ── SVG 3-Compartment Class Card Vector ──
    {
      selector: 'node[?svgDataUri]',
      style: {
        'background-image': 'data(svgDataUri)',
        'background-fit': 'contain',
        'background-clip': 'node',
        'background-color': 'transparent',
        'background-opacity': 0,
        'border-width': 0,
        'padding': 0,
        'width': 'data(width)',
        'height': 'data(height)',
        'shape': 'roundrectangle',
        'label': '',
        'z-index': 10
      }
    },

    // ── Level 0: Root Domain Tier Container ──
    {
      selector: 'node[?isPackage].nest-level-0, node[?isPackage][nestLevel = 0]',
      style: {
        'background-color': pkgs.tier0.bg,
        'background-opacity': pkgs.tier0.opacity,
        'border-width': pkgs.tier0.borderWidth,
        'border-color': pkgs.tier0.borderColor,
        'border-style': pkgs.tier0.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier0.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 18,
        'text-margin-y': -16,
        'text-background-color': pkgs.tier0.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '6px 16px',
        'text-border-width': 2.0,
        'text-border-color': pkgs.tier0.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '12.5px',
        'font-weight': 800,
        'letter-spacing': '0.03em',
        'color': pkgs.tier0.textColor,
        'padding': '44px',
        'z-index': 1
      }
    },

    // ── Level 1: Subpackage Container ──
    {
      selector: 'node[?isPackage].nest-level-1, node[?isPackage][nestLevel = 1]',
      style: {
        'background-color': pkgs.tier1.bg,
        'background-opacity': pkgs.tier1.opacity,
        'border-width': pkgs.tier1.borderWidth,
        'border-color': pkgs.tier1.borderColor,
        'border-style': pkgs.tier1.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier1.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -14,
        'text-background-color': pkgs.tier1.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 14px',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier1.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '12px',
        'font-weight': 700,
        'color': pkgs.tier1.textColor,
        'padding': '38px',
        'z-index': 2
      }
    },

    // ── Level 2: Subpackage Container ──
    {
      selector: 'node[?isPackage].nest-level-2, node[?isPackage][nestLevel = 2]',
      style: {
        'background-color': pkgs.tier2.bg,
        'background-opacity': pkgs.tier2.opacity,
        'border-width': pkgs.tier2.borderWidth,
        'border-color': pkgs.tier2.borderColor,
        'border-style': pkgs.tier2.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier2.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -14,
        'text-background-color': pkgs.tier2.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 12px',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier2.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '11.5px',
        'font-weight': 700,
        'color': pkgs.tier2.textColor,
        'padding': '32px',
        'z-index': 3
      }
    },

    // ── Level 3: Leaf Subpackage Container ──
    {
      selector: 'node[?isPackage].nest-level-3, node[?isPackage][nestLevel = 3]',
      style: {
        'background-color': pkgs.tier3.bg,
        'background-opacity': pkgs.tier3.opacity,
        'border-width': pkgs.tier3.borderWidth,
        'border-color': pkgs.tier3.borderColor,
        'border-style': pkgs.tier3.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier3.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 14,
        'text-margin-y': -13,
        'text-background-color': pkgs.tier3.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '4px 10px',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier3.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '11px',
        'font-weight': 700,
        'color': pkgs.tier3.textColor,
        'padding': '28px',
        'z-index': 4
      }
    },

    // ── Level 4+: Deepest Innermost Package Container ──
    {
      selector: 'node[?isPackage].nest-level-4, node[?isPackage].nest-level-5, node[?isPackage][nestLevel >= 4]',
      style: {
        'background-color': pkgs.tier4.bg,
        'background-opacity': pkgs.tier4.opacity,
        'border-width': pkgs.tier4.borderWidth,
        'border-color': pkgs.tier4.borderColor,
        'border-style': pkgs.tier4.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier4.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 14,
        'text-margin-y': -13,
        'text-background-color': pkgs.tier4.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '4px 10px',
        'text-border-width': 2.0,
        'text-border-color': pkgs.tier4.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '11px',
        'font-weight': 700,
        'color': pkgs.tier4.textColor,
        'padding': '24px',
        'z-index': 5
      }
    },

    // ── General Package Container Fallback ──
    {
      selector: 'node[?isPackage]',
      style: {
        'background-color': pkgs.fallback.bg,
        'background-opacity': pkgs.fallback.opacity,
        'border-width': pkgs.fallback.borderWidth,
        'border-color': pkgs.fallback.borderColor,
        'border-style': pkgs.fallback.borderStyle,
        'shape': 'roundrectangle',
        'border-radius': pkgs.fallback.borderRadius,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -14,
        'text-background-color': pkgs.fallback.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 12px',
        'text-border-width': 1.5,
        'text-border-color': pkgs.fallback.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, -apple-system, sans-serif',
        'font-size': '11px',
        'font-weight': 700,
        'color': pkgs.fallback.textColor,
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

    // ── Default Edge Fallback ──
    {
      selector: 'edge',
      style: {
        'width': 2.2,
        'line-color': edges.defaultLine,
        'target-arrow-color': edges.defaultArrow,
        'target-arrow-shape': 'vee',
        'arrow-scale': 1.8,
        'curve-style': 'taxi',
        'taxi-direction': 'auto',
        'taxi-turn': '28px',
        'taxi-turn-min-distance': '12px',
        'taxi-radius': 8,
        'source-distance-from-node': 4,
        'target-distance-from-node': 4,
        'label': 'data(label)',
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '10px',
        'font-weight': 700,
        'color': edges.defaultLabelText,
        'text-background-color': edges.defaultLabelBg,
        'text-background-opacity': 0.95,
        'text-background-padding': '3px 6px',
        'text-border-width': 1,
        'text-border-color': edges.defaultLabelBorder,
        'text-border-opacity': 1,
        'z-index': 999
      }
    },

    // ── 1. UML Generalization (--|>) ──
    {
      selector: 'edge[uml_kind = "generalization"], edge.edge-generalization',
      style: {
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'line-color': edges.generalization,
        'target-arrow-color': edges.generalization,
        'line-style': 'solid',
        'width': 2.5,
        'arrow-scale': 2.0
      }
    },

    // ── 2. UML Realization / Implementation (..|>) ──
    {
      selector: 'edge[uml_kind = "realization"], edge.edge-realization',
      style: {
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'line-color': edges.realization,
        'target-arrow-color': edges.realization,
        'line-style': 'dashed',
        'line-dash-pattern': [8, 4],
        'width': 2.4,
        'arrow-scale': 2.0
      }
    },

    // ── 3. UML Composition (*--) ──
    {
      selector: 'edge[uml_kind = "composition"], edge.edge-composition',
      style: {
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'filled',
        'line-color': edges.composition,
        'source-arrow-color': edges.composition,
        'target-arrow-shape': 'none',
        'line-style': 'solid',
        'width': 2.6,
        'arrow-scale': 2.2
      }
    },

    // ── 4. UML Aggregation (o--) ──
    {
      selector: 'edge[uml_kind = "aggregation"], edge.edge-aggregation',
      style: {
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'hollow',
        'line-color': edges.aggregation,
        'source-arrow-color': edges.aggregation,
        'target-arrow-shape': 'none',
        'line-style': 'solid',
        'width': 2.4,
        'arrow-scale': 2.2
      }
    },

    // ── 5. UML Association (-->) ──
    {
      selector: 'edge[uml_kind = "association"], edge.edge-association',
      style: {
        'target-arrow-shape': 'vee',
        'line-color': edges.association,
        'target-arrow-color': edges.association,
        'line-style': 'solid',
        'width': 2.2,
        'arrow-scale': 1.8
      }
    },

    // ── 6. UML Dependency (..>) ──
    {
      selector: 'edge[uml_kind = "dependency"], edge.edge-dependency',
      style: {
        'target-arrow-shape': 'vee',
        'line-color': edges.dependency,
        'target-arrow-color': edges.dependency,
        'line-style': 'dashed',
        'line-dash-pattern': [6, 4],
        'width': 2.0,
        'arrow-scale': 1.8
      }
    },

    // ── 7. UML Package Containment (+--) ──
    {
      selector: 'edge[uml_kind = "containment"], edge.edge-containment',
      style: {
        'target-arrow-shape': 'circle',
        'target-arrow-fill': 'filled',
        'line-color': edges.containment,
        'target-arrow-color': edges.containment,
        'line-style': 'dotted',
        'width': 2.0,
        'arrow-scale': 1.4
      }
    },

    // ── 8. Sequence Message (->) ──
    {
      selector: 'edge[uml_kind = "message"], edge.edge-message',
      style: {
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'filled',
        'line-style': 'solid',
        'line-color': theme.isDark ? '#818CF8' : '#6366F1',
        'target-arrow-color': theme.isDark ? '#818CF8' : '#6366F1',
        'source-arrow-shape': 'none',
        'arrow-scale': 1.8,
        'width': 2.4,
        'z-index': 999
      }
    },

    // ── 9. State Transition (-->) ──
    {
      selector: 'edge[uml_kind = "transition"], edge.edge-transition',
      style: {
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'filled',
        'line-style': 'solid',
        'line-color': theme.isDark ? '#22D3EE' : '#06B6D4',
        'target-arrow-color': theme.isDark ? '#22D3EE' : '#06B6D4',
        'source-arrow-shape': 'none',
        'arrow-scale': 1.8,
        'width': 2.4,
        'z-index': 999
      }
    },

    // ── 10. Activity Control Flow (-->) ──
    {
      selector: 'edge[uml_kind = "control_flow"], edge.edge-control_flow',
      style: {
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'filled',
        'line-style': 'solid',
        'line-color': theme.isDark ? '#34D399' : '#10B981',
        'target-arrow-color': theme.isDark ? '#34D399' : '#10B981',
        'source-arrow-shape': 'none',
        'arrow-scale': 1.8,
        'width': 2.4,
        'z-index': 999
      }
    },

    // ── 11. Deployment Manifestation (..>) ──
    {
      selector: 'edge[uml_kind = "manifestation"], edge.edge-manifestation',
      style: {
        'target-arrow-shape': 'vee',
        'target-arrow-fill': 'filled',
        'line-style': 'dashed',
        'line-dash-pattern': [6, 4],
        'line-color': theme.isDark ? '#FB923C' : '#EA580C',
        'target-arrow-color': theme.isDark ? '#FB923C' : '#EA580C',
        'source-arrow-shape': 'none',
        'arrow-scale': 1.8,
        'width': 2.2,
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

    // ── Interactive Hover Highlight ──
    {
      selector: 'node.highlighted, node:selected',
      style: {
        'shadow-blur': 25,
        'shadow-color': theme.isDark ? '#60A5FA' : '#2563EB',
        'shadow-opacity': 0.8,
        'z-index': 9999
      }
    },
    {
      selector: 'edge.highlighted, edge:selected',
      style: {
        'width': 4.0,
        'line-color': theme.isDark ? '#60A5FA' : '#2563EB',
        'target-arrow-color': theme.isDark ? '#60A5FA' : '#2563EB',
        'source-arrow-color': theme.isDark ? '#60A5FA' : '#2563EB',
        'z-index': 9999
      }
    },
    {
      selector: 'edge.dimmed, node.dimmed, .dimmed',
      style: {
        'opacity': 0.12
      }
    },
    {
      selector: 'edge.layer-hidden',
      style: {
        'display': 'none'
      }
    }
  ];
}
