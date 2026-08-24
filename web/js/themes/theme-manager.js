/**
 * OpenHeart Theme Manager
 * Unified Theme Engine with Pub/Sub Lifecycle Management and Cytoscape Stylesheet Compiler.
 */
import { LightTheme } from './light.js';
import { DarkTheme } from './dark.js';

export const DEFAULT_MANIFEST = {
  version: "2.5.0",
  name: "OpenHeart Universal Diagram Architecture Manifest",
  description: "Declarative catalog of all UML 2.5 projections, compiler pipeline graphs, relationship terminologies, and node classifier schemas.",
  categories: [
    {
      id: "uml_structural",
      title: "UML 2.5 Structural Projections",
      badge: "7 TYPES",
      diagrams: [
        { id: "class", number: "01", name: "Class Diagram", phase: "Phase 9", file_json: "class.json", file_puml: "class.puml", layout_engine: "package_tree" },
        { id: "package", number: "02", name: "Package Diagram", phase: "Phase 9", file_json: "package.json", file_puml: "package.puml", layout_engine: "hierarchical" },
        { id: "component", number: "03", name: "Component Diagram", phase: "Phase 9", file_json: "component.json", file_puml: "component.puml", layout_engine: "hierarchical" },
        { id: "composite", number: "04", name: "Composite Structure", phase: "Phase 9", file_json: "composite.json", file_puml: "composite.puml", layout_engine: "hierarchical" },
        { id: "object", number: "05", name: "Object Diagram", phase: "Phase 9", file_json: "object.json", file_puml: "object.puml", layout_engine: "hierarchical" },
        { id: "deployment", number: "06", name: "Deployment Diagram", phase: "Phase 9", file_json: "deployment.json", file_puml: "deployment.puml", layout_engine: "hierarchical" },
        { id: "profile", number: "07", name: "Profile Diagram", phase: "Phase 9", file_json: "profile.json", file_puml: "profile.puml", layout_engine: "hierarchical" }
      ]
    },
    {
      id: "uml_behavioral",
      title: "UML 2.5 Behavioral Projections",
      badge: "7 TYPES",
      diagrams: [
        { id: "sequence", number: "08", name: "Sequence Diagram", phase: "Phase 9", file_json: "sequence.json", file_puml: "sequence.puml", layout_engine: "sequence" },
        { id: "statemachine", number: "09", name: "State Machine", phase: "Phase 9", file_json: "statemachine.json", file_puml: "statemachine.puml", layout_engine: "hierarchical" },
        { id: "activity", number: "10", name: "Activity Diagram", phase: "Phase 9", file_json: "activity.json", file_puml: "activity.puml", layout_engine: "hierarchical" },
        { id: "usecase", number: "11", name: "Use Case Diagram", phase: "Phase 9", file_json: "usecase.json", file_puml: "usecase.puml", layout_engine: "usecase" },
        { id: "communication", number: "12", name: "Communication Diagram", phase: "Phase 9", file_json: "communication.json", file_puml: "communication.puml", layout_engine: "hierarchical" },
        { id: "interaction", number: "13", name: "Interaction Overview", phase: "Phase 9", file_json: "interaction.json", file_puml: "interaction.puml", layout_engine: "hierarchical" },
        { id: "timing", number: "14", name: "Timing Diagram", phase: "Phase 9", file_json: "timing.json", file_puml: "timing.puml", layout_engine: "timing" }
      ]
    },
    {
      id: "compiler_pipeline",
      title: "Compiler Pipeline IRs",
      badge: "5 GRAPHS",
      diagrams: [
        { id: "cfg", number: "15", name: "Control Flow (CFG)", phase: "Phase 4", file_json: "cfg.json", file_puml: "cfg.puml", layout_engine: "hierarchical" },
        { id: "dfg", number: "16", name: "Data Flow (DFG)", phase: "Phase 5", file_json: "dfg.json", file_puml: "dfg.puml", layout_engine: "hierarchical" },
        { id: "cdg", number: "17", name: "Control Dep (CDG)", phase: "Phase 4", file_json: "cdg.json", file_puml: "cdg.puml", layout_engine: "hierarchical" },
        { id: "callgraph", number: "18", name: "Call Graph (CG)", phase: "Phase 6", file_json: "callgraph.json", file_puml: "callgraph.puml", layout_engine: "hierarchical" },
        { id: "robdd", number: "19", name: "ROBDD Saturation", phase: "Phase 8", file_json: "robdd.json", file_puml: "robdd.puml", layout_engine: "hierarchical" }
      ]
    }
  ],
  relationship_types: {
    generalization: { label: "Generalization (--|>)", icon: "▷", color_light: "#7C3AED", color_dark: "#A78BFA", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "hollow", arrow: "--|>", width: 2.5, arrow_scale: 2.0 },
    realization: { label: "Realization (..|>)", icon: "▷", color_light: "#2563EB", color_dark: "#60A5FA", line_style: "dashed", target_arrow_shape: "triangle", target_arrow_fill: "hollow", arrow: "..|>", width: 2.4, arrow_scale: 2.0 },
    composition: { label: "Composition (*--)", icon: "♦", color_light: "#DC2626", color_dark: "#F87171", line_style: "solid", source_arrow_shape: "diamond", source_arrow_fill: "filled", target_arrow_shape: "none", arrow: "*--", width: 2.6, arrow_scale: 2.2 },
    aggregation: { label: "Aggregation (o--)", icon: "◇", color_light: "#059669", color_dark: "#34D399", line_style: "solid", source_arrow_shape: "diamond", source_arrow_fill: "hollow", target_arrow_shape: "none", arrow: "o--", width: 2.4, arrow_scale: 2.2 },
    association: { label: "Association (-->)", icon: "→", color_light: "#0284C7", color_dark: "#38BDF8", line_style: "solid", target_arrow_shape: "vee", arrow: "-->", width: 2.2, arrow_scale: 1.8 },
    dependency: { label: "Dependency (..>)", icon: "⇢", color_light: "#D97706", color_dark: "#FBBF24", line_style: "dashed", target_arrow_shape: "vee", arrow: "..>", width: 2.0, arrow_scale: 1.8 },
    containment: { label: "Containment (+--)", icon: "⊕", color_light: "#6B7280", color_dark: "#9CA3AF", line_style: "dotted", target_arrow_shape: "circle", target_arrow_fill: "filled", arrow: "+--", width: 2.0, arrow_scale: 1.4 },
    message: { label: "Sequence Message (->)", icon: "▶", color_light: "#4F46E5", color_dark: "#818CF8", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "filled", arrow: "->", width: 2.4, arrow_scale: 1.8 },
    transition: { label: "State Transition (-->)", icon: "➔", color_light: "#0891B2", color_dark: "#22D3EE", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "filled", arrow: "-->", width: 2.4, arrow_scale: 1.8 },
    control_flow: { label: "Control Flow (-->)", icon: "▶", color_light: "#059669", color_dark: "#10B981", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "filled", arrow: "-->", width: 2.4, arrow_scale: 1.8 },
    manifestation: { label: "Manifestation (..>)", icon: "📦", color_light: "#EA580C", color_dark: "#FB923C", line_style: "dashed", target_arrow_shape: "vee", arrow: "..>", width: 2.2, arrow_scale: 1.8 },
    low_branch: { label: "Low Branch / 0 (..>)", icon: "⤍", color_light: "#DC2626", color_dark: "#EF4444", line_style: "dashed", target_arrow_shape: "vee", arrow: "..>", width: 2.2, arrow_scale: 1.8 },
    high_branch: { label: "High Branch / 1 (-->)", icon: "➜", color_light: "#059669", color_dark: "#10B981", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "filled", arrow: "-->", width: 2.5, arrow_scale: 1.8 },
    data_flow: { label: "Data Flow Def-Use (-->)", icon: "⇢", color_light: "#0284C7", color_dark: "#38BDF8", line_style: "solid", target_arrow_shape: "vee", arrow: "-->", width: 2.2, arrow_scale: 1.8 },
    extension: { label: "Profile Extension (--|>)", icon: "▲", color_light: "#9333EA", color_dark: "#C084FC", line_style: "solid", target_arrow_shape: "triangle", target_arrow_fill: "filled", arrow: "--|>", width: 2.5, arrow_scale: 2.0 },
    assembly_connector: { label: "Assembly Port (-(0-)", icon: "⚯", color_light: "#D97706", color_dark: "#FACC15", line_style: "solid", target_arrow_shape: "circle", target_arrow_fill: "hollow", arrow: "-->", width: 2.4, arrow_scale: 1.6 }
  },
  node_kinds: {
    class: { label: "Class", badge: "C", color: "#3B82F6" },
    interface: { label: "Interface", badge: "I", color: "#10B981" },
    abstract: { label: "Abstract Class", badge: "A", color: "#8B5CF6" },
    enum: { label: "Enumeration", badge: "E", color: "#F59E0B" },
    package: { label: "Package", badge: "📁", color: "#FACC15" },
    state: { label: "State", badge: "S", color: "#06B6D4" },
    action: { label: "Action", badge: "⚡", color: "#10B981" },
    component: { label: "Component", badge: "⚙", color: "#3B82F6" },
    device: { label: "Device", badge: "🖥", color: "#6366F1" },
    artifact: { label: "Artifact", badge: "📦", color: "#EA580C" },
    participant: { label: "Participant", badge: "👤", color: "#6366F1" },
    actor: { label: "Actor", badge: "👤", color: "#FACC15" },
    usecase: { label: "Use Case", badge: "U", color: "#38BDF8" },
    object: { label: "Object Instance", badge: "O", color: "#3B82F6" },
    bb: { label: "Basic Block", badge: "B", color: "#10B981" },
    bdd_gate: { label: "BDD Gate", badge: "◆", color: "#C084FC" },
    bdd_terminal: { label: "BDD Terminal", badge: "●", color: "#10B981" },
    part: { label: "Composite Part", badge: "P", color: "#38BDF8" },
    composite_classifier: { label: "Composite Classifier", badge: "C", color: "#38BDF8" },
    metaclass: { label: "Metaclass", badge: "M", color: "#FACC15" },
    stereotype: { label: "Stereotype", badge: "«S»", color: "#C084FC" },
    timing_track: { label: "Timing Track", badge: "T", color: "#38BDF8" },
    interaction_use: { label: "Interaction Use", badge: "ref", color: "#6366F1" },
    data_node: { label: "Data Node", badge: "D", color: "#38BDF8" }
  }
};

let themeListeners = [];
let cachedManifest = DEFAULT_MANIFEST;

export async function fetchManifest() {
  try {
    const res = await fetch('diagrams/manifest.json');
    if (res.ok) {
      const data = await res.json();
      if (data && data.categories) {
        cachedManifest = data;
      }
    }
  } catch (e) {
    console.warn('[THEME] Using embedded default manifest:', e);
  }
  return cachedManifest || DEFAULT_MANIFEST;
}

export function setManifest(manifest) {
  cachedManifest = manifest || DEFAULT_MANIFEST;
}

export function getCachedManifest() {
  return cachedManifest || DEFAULT_MANIFEST;
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

  if (typeof localStorage !== 'undefined') {
    localStorage.setItem('openheart_theme', themeName);
  }

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
  let saved = null;
  if (typeof localStorage !== 'undefined') {
    saved = localStorage.getItem('openheart_theme');
  }
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 18,
        'text-margin-y': -14,
        'text-background-color': pkgs.tier0.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '6px 14px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 2.0,
        'text-border-color': pkgs.tier0.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -13,
        'text-background-color': pkgs.tier1.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 12px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier1.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '12px',
        'font-weight': 800,
        'color': pkgs.tier1.textColor,
        'padding': '38px',
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -13,
        'text-background-color': pkgs.tier2.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 12px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier2.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '11.5px',
        'font-weight': 800,
        'color': pkgs.tier2.textColor,
        'padding': '32px',
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 14,
        'text-margin-y': -12,
        'text-background-color': pkgs.tier3.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '4px 10px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 1.8,
        'text-border-color': pkgs.tier3.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '11px',
        'font-weight': 800,
        'color': pkgs.tier3.textColor,
        'padding': '28px',
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 14,
        'text-margin-y': -12,
        'text-background-color': pkgs.tier4.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '4px 10px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 2.0,
        'text-border-color': pkgs.tier4.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '11px',
        'font-weight': 800,
        'color': pkgs.tier4.textColor,
        'padding': '24px',
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
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 16,
        'text-margin-y': -13,
        'text-background-color': pkgs.fallback.textBg,
        'text-background-opacity': 1.0,
        'text-background-padding': '5px 12px',
        'text-background-shape': 'roundrectangle',
        'text-border-width': 1.5,
        'text-border-color': pkgs.fallback.textBorder,
        'text-border-opacity': 1.0,
        'font-family': 'JetBrains Mono, monospace',
        'font-size': '11px',
        'font-weight': 800,
        'color': pkgs.fallback.textColor,
        'padding': '36px',
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
      selector: 'edge.layer-hidden',
      style: {
        'display': 'none'
      }
    }
  ];
}
