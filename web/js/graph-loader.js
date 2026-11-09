/**
 * OpenHeart Typed Graph IR Ingestion Engine
 * Converts a validated Graph IR (see graph-ir-validate.js) into Cytoscape elements with
 * vector card images. Source provenance is copied as-is: a node without a file or source
 * anchor in the IR has none here (no file name or line number is ever invented).
 */

import {
  generateUmlClassCardSvg,
  generatePackageFolderSvg,
  generateStateNodeSvg,
  generateActionNodeSvg,
  generateComponentNodeSvg,
  generateDeploymentNodeSvg,
  generateSequenceLifelineSvg,
  generateUseCaseSvg,
  generateObjectCardSvg,
  generateCfgBlockSvg,
  generateBddGateSvg,
  generateCompositeCardSvg,
  generateProfileCardSvg,
  generateTimingTrackSvg,
  generateInteractionFrameSvg
} from './uml-card-renderer.js';
import { isDarkMode } from './themes/index.js';

function formatMember(m, fallbackVisibility, isMethod) {
  if (typeof m === 'string') return m;
  if (!m || typeof m !== 'object') return '';
  const vis = m.visibility || fallbackVisibility;
  if (m.signature) return `${vis} ${m.signature}`;
  if (isMethod) return `${vis} ${m.name ? `${m.name}()` : '?()'}`;
  return `${vis} ${m.name || '?'}`;
}

/**
 * Renders the card image for a node. `card` carries only presentation inputs:
 * { kind, name, label, stereotype, fields, methods, instructions, leafPackage, nestLevel, id }
 */
export function renderCardSvg(card, isDark) {
  const kind = card.kind || 'class';
  if (card.leafPackage) {
    return generatePackageFolderSvg({ name: card.name, nestLevel: card.nestLevel || 0, width: 240, height: 100, isDark });
  }
  switch (kind) {
    case 'bb':
      return generateCfgBlockSvg({ id: card.id, label: card.label, instructions: card.instructions || [], width: 280, isDark });
    case 'bdd_gate':
    case 'bdd_terminal':
      return generateBddGateSvg({ varName: card.name, isTerminal: kind === 'bdd_terminal', terminalValue: card.id === '1' ? 1 : 0, isDark });
    case 'state': {
      let entry = null;
      let doAct = null;
      let exit = null;
      (card.instructions || []).forEach(inst => {
        if (typeof inst !== 'string') return;
        if (inst.startsWith('entry /')) entry = inst.replace('entry /', '').trim();
        if (inst.startsWith('do /')) doAct = inst.replace('do /', '').trim();
        if (inst.startsWith('exit /')) exit = inst.replace('exit /', '').trim();
      });
      return generateStateNodeSvg({ name: card.name, entryAction: entry, doActivity: doAct, exitAction: exit, width: 240, isDark });
    }
    case 'action':
      if (card.stereotype === '<<interaction_use>>') {
        return generateInteractionFrameSvg({ name: card.label || card.name, instructions: card.instructions || [], width: 280, isDark });
      }
      return generateActionNodeSvg({
        name: card.label || card.name,
        isStart: card.name === 'start' || card.name === '[*]',
        isStop: card.name === 'stop',
        width: 230,
        isDark
      });
    case 'timing_track':
      return generateTimingTrackSvg({ name: card.label || card.name, instructions: card.instructions || [], width: 600, isDark });
    case 'part':
    case 'composite_classifier':
      return generateCompositeCardSvg({ name: card.label || card.name, fields: card.fields || [], methods: card.methods || [], width: 270, isDark });
    case 'metaclass':
    case 'stereotype':
      return generateProfileCardSvg({ name: card.name, stereotype: card.stereotype || `<<${kind}>>`, fields: card.fields || [], width: 270, isDark });
    case 'component':
      return generateComponentNodeSvg({ name: card.name, width: 230, isDark });
    case 'device':
    case 'artifact':
      return generateDeploymentNodeSvg({ name: card.name, isArtifact: kind === 'artifact', width: 230, isDark });
    case 'participant':
    case 'actor':
      return generateSequenceLifelineSvg({ name: card.name, isActor: kind === 'actor', width: 200, isDark });
    case 'usecase':
      return generateUseCaseSvg({ name: card.label || card.name, width: 220, isDark });
    case 'object':
    case 'data_node':
      return generateObjectCardSvg({
        name: card.label || card.name,
        fields: (card.fields || []).map(f => (typeof f === 'string' ? f : (f.signature || f.name || ''))),
        width: 260,
        isDark
      });
    default:
      return generateUmlClassCardSvg({
        name: card.name,
        kind,
        stereotype: card.stereotype || `<<${kind}>>`,
        fields: (card.fields || []).map(f => formatMember(f, '-', false)),
        methods: (card.methods || []).map(m => formatMember(m, '+', true)),
        width: 290,
        isDark
      });
  }
}

/**
 * Rebuilds the card inputs from Cytoscape node data (used for in-place theme restyling).
 */
export function cardFromNodeData(data) {
  return {
    id: data.id,
    kind: data.kind,
    name: data.leafPackage ? (data.rawName || data.name) : data.name,
    label: data.cardLabel || data.name,
    stereotype: data.stereotype,
    fields: data.fields || [],
    methods: data.methods || [],
    instructions: data.instructions || [],
    leafPackage: Boolean(data.isLeafPackage),
    nestLevel: data.nestLevel || 0
  };
}

export function loadGraphIrToCytoscape(graphIr) {
  if (!graphIr || !Array.isArray(graphIr.nodes)) return [];

  const isDark = isDarkMode();
  const elements = [];
  const nodeIds = new Set();
  const parentIds = new Set(graphIr.nodes.map(n => n && n.parent).filter(Boolean));

  graphIr.nodes.forEach(node => {
    if (!node || node.id === undefined || node.id === null) return;
    const id = String(node.id);
    if (nodeIds.has(id)) return; // duplicates are reported by the validator before loading
    nodeIds.add(id);

    if (node.is_package) {
      const shortName = node.name || (node.label ? String(node.label).replace(/^package\s*\[?/, '').replace(/\]?$/, '') : id);
      const isDomainTier = Boolean(node.is_domain_tier || node.nest_level === 0);
      const displayLabel = isDomainTier ? `DOMAIN: ${String(shortName).toUpperCase()}` : `package [${shortName}]`;

      if (parentIds.has(id)) {
        elements.push({
          data: {
            id,
            label: displayLabel,
            rawName: node.name || shortName,
            name: node.name || shortName,
            textLabel: displayLabel,
            kind: 'package',
            isPackage: true,
            isDomainTier,
            parent: node.parent || undefined,
            nestLevel: node.nest_level || 0,
            file: null,
            lines: []
          },
          classes: `compound-package ${isDomainTier ? 'pkg-domain-tier' : 'pkg-subpackage'} nest-level-${Math.min(5, node.nest_level || 0)}`
        });
      } else {
        const card = { id, kind: 'package', name: shortName, leafPackage: true, nestLevel: node.nest_level || 0 };
        const svgData = renderCardSvg(card, isDark);
        elements.push({
          data: {
            id,
            label: '',
            rawName: node.name || shortName,
            name: node.name || shortName,
            textLabel: displayLabel,
            kind: 'package',
            isPackage: false,
            isLeafPackage: true,
            isDomainTier,
            parent: node.parent || undefined,
            nestLevel: node.nest_level || 0,
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri,
            cardSource: 'ir',
            file: null,
            lines: []
          },
          classes: `leaf-package nest-level-${Math.min(5, node.nest_level || 0)}`
        });
      }
      return;
    }

    const card = {
      id,
      kind: node.kind || 'class',
      name: node.name || id,
      label: node.label || node.name || id,
      stereotype: node.stereotype || `<<${node.kind || 'class'}>>`,
      fields: node.fields || [],
      methods: node.methods || [],
      instructions: node.instructions || [],
      leafPackage: false,
      nestLevel: node.nest_level || 0
    };
    const svgData = renderCardSvg(card, isDark);
    const anchor = node.anchor && typeof node.anchor === 'object' ? node.anchor : null;
    const file = typeof node.file === 'string' && node.file ? node.file : (anchor && typeof anchor.path === 'string' ? anchor.path : null);
    let lines = Array.isArray(node.lines) ? node.lines.filter(l => Number.isInteger(l) && l > 0) : [];
    if (lines.length === 0 && anchor && Number.isInteger(anchor.start_line)) lines = [anchor.start_line];

    elements.push({
      data: {
        id,
        name: node.name || id,
        qualifiedName: node.qualified_name || null,
        label: '',
        cardLabel: node.label || null,
        textLabel: `${card.stereotype}\n${card.name}`,
        kind: card.kind,
        stereotype: card.stereotype,
        fields: card.fields,
        methods: card.methods,
        instructions: card.instructions,
        extends: node.extends || undefined,
        implements: node.implements || undefined,
        width: svgData.width,
        height: svgData.height,
        svgDataUri: svgData.dataUri,
        cardSource: 'ir',
        file,
        anchor,
        language: node.language || null,
        raw_url: node.raw_url || undefined,
        lines,
        parent: node.parent || undefined,
        outer: node.outer || undefined,
        nestLevel: node.nest_level || 0,
        cyclomatic: node.cyclomatic !== undefined ? node.cyclomatic : undefined,
        idomRank: node.idom_rank !== undefined ? node.idom_rank : undefined,
        bddSatCount: node.sat_count !== undefined ? node.sat_count : undefined,
        branchPredicate: typeof node.predicate === 'string' ? node.predicate : undefined
      },
      classes: `class-card kind-${String(card.kind).replace(/[^A-Za-z0-9_-]/g, '_')}`
    });
  });

  (graphIr.edges || []).forEach((edge, idx) => {
    if (!edge) return;
    const source = edge.source !== undefined ? String(edge.source) : null;
    const target = edge.target !== undefined ? String(edge.target) : null;
    if (!source || !target || !nodeIds.has(source) || !nodeIds.has(target)) return;
    const edgeKind = String(edge.kind || edge.uml_kind || 'association');
    elements.push({
      data: {
        id: edge.id !== undefined ? String(edge.id) : `edge_${idx}_${source}_${target}`,
        source,
        target,
        uml_kind: edgeKind,
        label: typeof edge.label === 'string' ? edge.label : '',
        arrow: edge.arrow || '-->',
        resolution: edge.resolution || undefined
      },
      classes: `edge-${edgeKind.replace(/[^A-Za-z0-9_-]/g, '_')}`
    });
  });

  return elements;
}
