/**
 * OpenHeart Theme Manager
 * Unified Theme Engine with Pub/Sub Lifecycle Management and Cytoscape Stylesheet Compiler.
 * The diagram catalog comes only from diagrams/manifest.json; there is no embedded copy to
 * drift out of date. When the manifest cannot be loaded, fetchManifest reports why.
 */
import { LightTheme } from './light.js';
import { DarkTheme } from './dark.js';
import { localStore } from '../storage.js';
import { PACKAGE_PADDING, PACKAGE_FALLBACK_PADDING, PACKAGE_LABEL, PACKAGE_FALLBACK_LABEL } from '../layout/geometry.js';

/**
 * Package box padding and label geometry, shared with the layout engine (layout/geometry.js)
 * so the space a layout reserves and the box the canvas draws are the same. Labels sit centred
 * above the box: with text-halign left Cytoscape draws a compound's label outside the box, to
 * its left, where it collides with neighbouring packages.
 */
function packageGeometryStyle(padding, label) {
  return {
    'text-valign': 'top',
    'text-halign': 'center',
    'text-margin-x': 0,
    'text-margin-y': -label.marginY,
    'text-background-padding': `${label.padY}px ${label.padX}px`,
    'text-border-width': label.border,
    'font-size': `${label.fontSize}px`,
    'padding': `${padding}px`
  };
}

function packageTierGeometry(tier) {
  return packageGeometryStyle(PACKAGE_PADDING[tier], PACKAGE_LABEL[tier]);
}

let themeListeners = [];
let cachedManifest = null;
let manifestError = null;

function isValidManifest(data) {
  return Boolean(data && Array.isArray(data.categories) && data.categories.every(cat =>
    cat && typeof cat.title === 'string' && Array.isArray(cat.diagrams) &&
    cat.diagrams.every(d => d && typeof d.id === 'string' && /^[a-z_]{1,32}$/.test(d.id) && typeof d.name === 'string')));
}

/**
 * @returns {Promise<object|null>} the manifest, or null (see getManifestError()).
 */
export async function fetchManifest() {
  try {
    const res = await fetch('diagrams/manifest.json', { cache: 'no-cache' });
    if (!res.ok) {
      manifestError = `diagrams/manifest.json returned HTTP ${res.status}.`;
      return cachedManifest;
    }
    const data = await res.json();
    if (!isValidManifest(data)) {
      manifestError = 'diagrams/manifest.json does not have the expected categories/diagrams structure.';
      return cachedManifest;
    }
    cachedManifest = data;
    manifestError = null;
  } catch (e) {
    manifestError = `diagrams/manifest.json could not be loaded: ${(e && e.message) || e}.`;
  }
  return cachedManifest;
}

export function getManifestError() {
  return manifestError;
}

export function setManifest(manifest) {
  cachedManifest = isValidManifest(manifest) ? manifest : null;
}

export function getCachedManifest() {
  return cachedManifest;
}

function compileDynamicEdgeStyles(manifest, isDark, edges) {
  const dynamicEdgeStyles = [];
  const relTypes = (manifest && manifest.relationship_types) ? manifest.relationship_types : {
    generalization: { label: 'Generalization', color_light: '#7C3AED', color_dark: '#A78BFA', target_arrow_shape: 'triangle', target_arrow_fill: 'hollow', line_style: 'solid', width: 2.5, arrow_scale: 2.0 },
    realization: { label: 'Realization', color_light: '#2563EB', color_dark: '#60A5FA', target_arrow_shape: 'triangle', target_arrow_fill: 'hollow', line_style: 'dashed', width: 2.4, arrow_scale: 2.0 },
    composition: { label: 'Composition', color_light: '#DC2626', color_dark: '#F87171', source_arrow_shape: 'diamond', source_arrow_fill: 'filled', target_arrow_shape: 'none', line_style: 'solid', width: 2.6, arrow_scale: 2.2 },
    aggregation: { label: 'Aggregation', color_light: '#059669', color_dark: '#34D399', source_arrow_shape: 'diamond', source_arrow_fill: 'hollow', target_arrow_shape: 'none', line_style: 'solid', width: 2.4, arrow_scale: 2.2 },
    association: { label: 'Association', color_light: '#0284C7', color_dark: '#38BDF8', target_arrow_shape: 'vee', line_style: 'solid', width: 2.2, arrow_scale: 1.8 },
    dependency: { label: 'Dependency', color_light: '#D97706', color_dark: '#FBBF24', target_arrow_shape: 'vee', line_style: 'dashed', width: 2.0, arrow_scale: 1.8 }
  };

  Object.entries(relTypes).forEach(([kind, conf]) => {
    const color = isDark ? (conf.color_dark || '#38BDF8') : (conf.color_light || '#0284C7');
    const styleObj = {
      'target-arrow-shape': conf.target_arrow_shape || 'vee',
      'line-style': conf.line_style || 'solid',
      'line-color': color,
      'target-arrow-color': color,
      'width': conf.width || 2.2,
      'arrow-scale': conf.arrow_scale || 1.8,
      'z-index': 999
    };
    if (conf.target_arrow_fill) styleObj['target-arrow-fill'] = conf.target_arrow_fill;
    if (conf.source_arrow_shape) {
      styleObj['source-arrow-shape'] = conf.source_arrow_shape;
      styleObj['source-arrow-color'] = color;
    }
    if (conf.source_arrow_fill) styleObj['source-arrow-fill'] = conf.source_arrow_fill;
    if (conf.line_style === 'dashed') styleObj['line-dash-pattern'] = [6, 4];
    if (conf.line_style === 'dotted') styleObj['line-dash-pattern'] = [2, 3];

    // Manifest keys become selector text; only plain identifiers are accepted.
    if (!/^[A-Za-z_][A-Za-z0-9_]{0,47}$/.test(kind)) return;
    dynamicEdgeStyles.push({
      selector: `edge[uml_kind = "${kind}"], edge.edge-${kind}`,
      style: styleObj
    });
  });

  return dynamicEdgeStyles;
}

export function isDarkMode() {
  if (typeof document === 'undefined' || !document.body) return false;
  return document.body.classList.contains('dark-theme') ||
         document.documentElement.getAttribute('data-theme') === 'dark';
}

export function getCurrentTheme() {
  return isDarkMode() ? DarkTheme : LightTheme;
}

export function getTheme(isDark = null) {
  if (isDark === null) {
    return getCurrentTheme();
  }
  return isDark ? DarkTheme : LightTheme;
}

export function onThemeChange(callback) {
  if (typeof callback === 'function') {
    themeListeners.push(callback);
  }
  return () => {
    themeListeners = themeListeners.filter(cb => cb !== callback);
  };
}

export function applyTheme(themeName) {
  const isDark = themeName === 'dark';
  if (typeof document !== 'undefined' && document.body) {
    if (isDark) {
      document.body.classList.add('dark-theme');
      document.documentElement.setAttribute('data-theme', 'dark');
    } else {
      document.body.classList.remove('dark-theme');
      document.documentElement.removeAttribute('data-theme');
    }
  }

  localStore.set('openheart_theme', isDark ? 'dark' : 'light');

  const themeObj = isDark ? DarkTheme : LightTheme;
  themeListeners.forEach(cb => {
    try {
      cb(themeObj, isDark);
    } catch (e) {
      console.error('[THEME] Error notifying listener:', e);
    }
  });

  return themeObj;
}

export function toggleTheme() {
  const next = isDarkMode() ? 'light' : 'dark';
  return applyTheme(next);
}

export function initTheme() {
  let saved = localStore.get('openheart_theme', null);
  if (saved !== 'dark' && saved !== 'light') saved = null;
  if (!saved && typeof window !== 'undefined' && window.matchMedia) {
    saved = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return applyTheme(saved || 'light');
}

export function buildCytoscapeStylesheet(theme = null, manifest = null) {
  if (!theme) {
    theme = getCurrentTheme();
  }
  const isDark = theme.isDark || isDarkMode();

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
      selector: 'node.compound-package.nest-level-0, node:parent[?isPackage].nest-level-0, node[?isPackage][!svgDataUri].nest-level-0, node[?isPackage][!svgDataUri][nestLevel = 0]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.tier0.bg,
        'background-opacity': pkgs.tier0.opacity,
        'border-width': pkgs.tier0.borderWidth,
        'border-color': pkgs.tier0.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier0.borderRadius,
        'text-background-color': pkgs.tier0.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.tier0.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'letter-spacing': `${PACKAGE_LABEL[0].letterSpacing}em`,
        'color': pkgs.tier0.textColor,
        ...packageTierGeometry(0),
        'z-index': 1
      }
    },

    // ── Level 1: Subpackage Container ──
    {
      selector: 'node.compound-package.nest-level-1, node:parent[?isPackage].nest-level-1, node[?isPackage][!svgDataUri].nest-level-1, node[?isPackage][!svgDataUri][nestLevel = 1]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.tier1.bg,
        'background-opacity': pkgs.tier1.opacity,
        'border-width': pkgs.tier1.borderWidth,
        'border-color': pkgs.tier1.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier1.borderRadius,
        'text-background-color': pkgs.tier1.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.tier1.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'color': pkgs.tier1.textColor,
        ...packageTierGeometry(1),
        'z-index': 2
      }
    },

    // ── Level 2: Subpackage Container ──
    {
      selector: 'node.compound-package.nest-level-2, node:parent[?isPackage].nest-level-2, node[?isPackage][!svgDataUri].nest-level-2, node[?isPackage][!svgDataUri][nestLevel = 2]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.tier2.bg,
        'background-opacity': pkgs.tier2.opacity,
        'border-width': pkgs.tier2.borderWidth,
        'border-color': pkgs.tier2.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier2.borderRadius,
        'text-background-color': pkgs.tier2.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.tier2.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'color': pkgs.tier2.textColor,
        ...packageTierGeometry(2),
        'z-index': 3
      }
    },

    // ── Level 3: Leaf Subpackage Container ──
    {
      selector: 'node.compound-package.nest-level-3, node:parent[?isPackage].nest-level-3, node[?isPackage][!svgDataUri].nest-level-3, node[?isPackage][!svgDataUri][nestLevel = 3]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.tier3.bg,
        'background-opacity': pkgs.tier3.opacity,
        'border-width': pkgs.tier3.borderWidth,
        'border-color': pkgs.tier3.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier3.borderRadius,
        'text-background-color': pkgs.tier3.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.tier3.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'color': pkgs.tier3.textColor,
        ...packageTierGeometry(3),
        'z-index': 4
      }
    },

    // ── Level 4+: Deepest Innermost Package Container ──
    {
      selector: 'node.compound-package.nest-level-4, node.compound-package.nest-level-5, node:parent[?isPackage].nest-level-4, node[?isPackage][!svgDataUri].nest-level-4, node[?isPackage][!svgDataUri][nestLevel >= 4]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.tier4.bg,
        'background-opacity': pkgs.tier4.opacity,
        'border-width': pkgs.tier4.borderWidth,
        'border-color': pkgs.tier4.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.tier4.borderRadius,
        'text-background-color': pkgs.tier4.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.tier4.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'color': pkgs.tier4.textColor,
        ...packageTierGeometry(4),
        'z-index': 5
      }
    },

    // ── General Package Container Fallback ──
    {
      selector: 'node.compound-package, node:parent[?isPackage], node[?isPackage][!svgDataUri]',
      style: {
        'label': 'data(label)',
        'background-color': pkgs.fallback.bg,
        'background-opacity': pkgs.fallback.opacity,
        'border-width': pkgs.fallback.borderWidth,
        'border-color': pkgs.fallback.borderColor,
        'border-style': 'solid',
        'shape': 'roundrectangle',
        'border-radius': pkgs.fallback.borderRadius,
        'text-background-color': pkgs.fallback.textBg,
        'text-background-opacity': 1.0,
        'text-background-shape': 'roundrectangle',
        'text-border-color': pkgs.fallback.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-weight': 800,
        'color': pkgs.fallback.textColor,
        ...packageGeometryStyle(PACKAGE_FALLBACK_PADDING, PACKAGE_FALLBACK_LABEL),
        'z-index': 1
      }
    },

    // ── SVG Vector Cards (Zero Outer Border, Zero Extra Padding, Zero Duplicate Label) ──
    {
      selector: 'node[?svgDataUri], node.leaf-package, node.class-card, node.state-card, node.action-card',
      style: {
        'background-image': 'data(svgDataUri)',
        'background-fit': 'contain',
        'background-clip': 'node',
        'background-color': 'transparent',
        'background-opacity': 0,
        'border-width': 0,
        'border-color': 'transparent',
        'padding': 0,
        'width': 'data(width)',
        'height': 'data(height)',
        'shape': 'roundrectangle',
        'label': '',
        'z-index': 10
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

    // ── Dynamically Compiled Relationship Styles from Manifest ──
    ...compileDynamicEdgeStyles(manifest || cachedManifest, isDark, edges),

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
      selector: 'edge.layer-hidden, .collapsed-hidden',
      style: {
        'display': 'none'
      }
    },

    // ── Edges drawn inside a card (sequence self messages, layout/sequence.js) ──
    // Hidden from the start: Cytoscape would otherwise compute their loop around the lifeline
    // centre while the canvas is created, which warns about invalid endpoints.
    {
      selector: 'edge[?layoutEmbedded]',
      style: {
        'display': 'none'
      }
    },

    // ── Self-relations draw as loops (taxi routing cannot draw a loop) ──
    {
      selector: 'edge:loop',
      style: {
        'curve-style': 'bezier',
        'loop-direction': '-45deg',
        'loop-sweep': '-60deg',
        'control-point-step-size': 60
      }
    },

    // ── Level of detail: plain labelled shapes below the zoom threshold ──
    {
      selector: 'node.lod-low',
      style: {
        'background-image': 'none',
        'background-opacity': 1,
        'background-color': isDark ? '#1E293B' : '#E2E8F0',
        'border-width': 2,
        'border-color': isDark ? '#64748B' : '#94A3B8',
        'label': 'data(name)',
        'text-valign': 'center',
        'text-halign': 'center',
        'font-family': 'JetBrains Mono, monospace',
        'font-size': 26,
        'font-weight': 700,
        'color': isDark ? '#F8FAFC' : '#0F172A',
        'min-zoomed-font-size': 5,
        'text-wrap': 'ellipsis',
        'text-max-width': 260
      }
    },

    // ── Level of detail: a sequence lifeline keeps its card (head, dashed line, self messages) ──
    // A plain shape would fill the whole lifeline column, the full diagram height.
    {
      selector: 'node.lod-low[lifelineHeight]',
      style: {
        'background-image': 'data(svgDataUri)',
        'background-opacity': 0,
        'border-width': 0,
        'label': ''
      }
    },

    // ── Explicit empty state (view not available from the active source) ──
    {
      selector: 'node.empty-state',
      style: {
        'shape': 'roundrectangle',
        'width': 'data(width)',
        'height': 'data(height)',
        'background-color': isDark ? '#111827' : '#FFFFFF',
        'background-opacity': 1,
        'border-width': 1.5,
        'border-style': 'dashed',
        'border-color': isDark ? '#475569' : '#94A3B8',
        'label': 'data(label)',
        'text-wrap': 'wrap',
        'text-max-width': 420,
        'text-valign': 'center',
        'text-halign': 'center',
        'font-family': 'Inter, -apple-system, sans-serif',
        'font-size': 12,
        'line-height': 1.45,
        'color': isDark ? '#CBD5E1' : '#334155',
        'events': 'no'
      }
    }
  ];
}
