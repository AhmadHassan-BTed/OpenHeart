/**
 * OpenHeart Official Direct Typed Graph IR Ingestion Engine
 * Ingests typed JSON Graph IR directly from the Rust compiler with ZERO intermediate text parsing.
 * Supports all 19 diagram types with authentic vector card generation and styling.
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

export function loadGraphIrToCytoscape(graphIr) {
  if (!graphIr || !graphIr.nodes) return [];

  const isDark = isDarkMode();
  const elements = [];
  const nodeMap = new Map();

  // 1. Identify packages with children
  const parentIds = new Set(graphIr.nodes.map(n => n.parent).filter(Boolean));

  // 2. Ingest Nodes Directly from Typed Schema
  graphIr.nodes.forEach(node => {
    let svgData = null;

    if (node.is_package) {
      const shortName = node.name || (node.label ? node.label.replace(/^package\s*\[?/, '').replace(/\]?$/, '') : node.id);
      const isParent = parentIds.has(node.id);
      const isDomainTier = node.is_domain_tier || (node.nest_level === 0);

      const displayLabel = isDomainTier
        ? `📂 DOMAIN: ${shortName.toUpperCase()}`
        : `📁 package [${shortName}]`;

      if (isParent) {
        const pkgNode = {
          data: {
            id: node.id,
            label: displayLabel,
            rawName: node.name || shortName,
            textLabel: displayLabel,
            kind: 'package',
            isPackage: true,
            isDomainTier: isDomainTier,
            parent: node.parent || undefined,
            nestLevel: node.nest_level || 0
          },
          classes: `compound-package ${isDomainTier ? 'pkg-domain-tier' : 'pkg-subpackage'} nest-level-${Math.min(5, node.nest_level || 0)}`
        };
        nodeMap.set(node.id, pkgNode);
        elements.push(pkgNode);
      } else {
        const svgData = generatePackageFolderSvg({
          name: shortName,
          nestLevel: node.nest_level || 0,
          width: 240,
          height: 100,
          isDark
        });

        const pkgNode = {
          data: {
            id: node.id,
            label: '',
            rawName: node.name || shortName,
            textLabel: displayLabel,
            kind: 'package',
            isPackage: false,
            isLeafPackage: true,
            isDomainTier: isDomainTier,
            parent: node.parent || undefined,
            nestLevel: node.nest_level || 0,
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri
          },
          classes: `leaf-package nest-level-${Math.min(5, node.nest_level || 0)}`
        };
        nodeMap.set(node.id, pkgNode);
        elements.push(pkgNode);
      }
      return;
    }

    // Leaf Vector Card generation based on typed kind
    if (node.kind === 'bb') {
      svgData = generateCfgBlockSvg({
        id: node.id,
        label: node.label,
        instructions: node.instructions || [],
        width: 280,
        isDark
      });
    } else if (node.kind === 'bdd_gate' || node.kind === 'bdd_terminal') {
      svgData = generateBddGateSvg({
        varName: node.name,
        isTerminal: node.kind === 'bdd_terminal',
        terminalValue: node.id === '1' ? 1 : 0,
        isDark
      });
    } else if (node.kind === 'state') {
      let entry = null, doAct = null, exit = null;
      (node.instructions || []).forEach(inst => {
        if (inst.startsWith('entry /')) entry = inst.replace('entry /', '').trim();
        if (inst.startsWith('do /')) doAct = inst.replace('do /', '').trim();
        if (inst.startsWith('exit /')) exit = inst.replace('exit /', '').trim();
      });
      svgData = generateStateNodeSvg({
        name: node.name,
        entryAction: entry,
        doActivity: doAct,
        exitAction: exit,
        width: 240,
        isDark
      });
    } else if (node.kind === 'action') {
      if (node.stereotype === '<<interaction_use>>') {
        svgData = generateInteractionFrameSvg({
          name: node.label || node.name,
          instructions: node.instructions || [],
          width: 280,
          isDark
        });
      } else {
        svgData = generateActionNodeSvg({
          name: node.label || node.name,
          isStart: node.name === 'start' || node.name === '[*]',
          isStop: node.name === 'stop',
          width: 230,
          isDark
        });
      }
    } else if (node.kind === 'timing_track') {
      svgData = generateTimingTrackSvg({
        name: node.label || node.name,
        instructions: node.instructions || [],
        width: 600,
        isDark
      });
    } else if (node.kind === 'part' || node.kind === 'composite_classifier') {
      svgData = generateCompositeCardSvg({
        name: node.label || node.name,
        fields: node.fields || [],
        methods: node.methods || [],
        width: 270,
        isDark
      });
    } else if (node.kind === 'metaclass' || node.kind === 'stereotype') {
      svgData = generateProfileCardSvg({
        name: node.name,
        stereotype: node.stereotype || `<<${node.kind}>>`,
        fields: node.fields || [],
        width: 270,
        isDark
      });
    } else if (node.kind === 'component') {
      svgData = generateComponentNodeSvg({ name: node.name, width: 230, isDark });
    } else if (node.kind === 'device' || node.kind === 'artifact') {
      svgData = generateDeploymentNodeSvg({ name: node.name, isArtifact: node.kind === 'artifact', width: 230, isDark });
    } else if (node.kind === 'participant' || node.kind === 'actor') {
      svgData = generateSequenceLifelineSvg({ name: node.name, isActor: node.kind === 'actor', width: 200, isDark });
    } else if (node.kind === 'usecase') {
      svgData = generateUseCaseSvg({ name: node.label || node.name, width: 220, isDark });
    } else if (node.kind === 'object' || node.kind === 'data_node') {
      svgData = generateObjectCardSvg({
        name: node.label || node.name,
        fields: (node.fields || []).map(f => typeof f === 'string' ? f : (f.signature || f.name)),
        width: 260,
        isDark
      });
    } else {
      // UML Class / Interface / Abstract / Enum Card
      const fieldsFormatted = (node.fields || []).map(f => {
        if (typeof f === 'string') return f;
        return `${f.visibility || '-'} ${f.signature || f.name || 'field'}`;
      });
      const methodsFormatted = (node.methods || []).map(m => {
        if (typeof m === 'string') return m;
        return `${m.visibility || '+'} ${m.signature || (m.name ? `${m.name}()` : 'method()')}`;
      });

      svgData = generateUmlClassCardSvg({
        name: node.name,
        kind: node.kind || 'class',
        stereotype: node.stereotype || `<<${node.kind || 'class'}>>`,
        fields: fieldsFormatted,
        methods: methodsFormatted,
        width: 290,
        isDark
      });
    }

    const cytoscapeNode = {
      data: {
        id: node.id,
        label: '',
        textLabel: `${node.stereotype || '<<class>>'}\n${node.name}`,
        kind: node.kind || 'class',
        width: svgData.width,
        height: svgData.height,
        svgDataUri: svgData.dataUri,
        file: node.file || `${node.name}.java`,
        raw_url: node.raw_url || undefined,
        rawUrl: node.raw_url || undefined,
        lines: node.lines || [1],
        parent: node.parent || undefined,
        nestLevel: node.nest_level || 0,
        cyclomatic: node.cyclomatic || undefined,
        idomRank: node.idom_rank || undefined,
        bddSatCount: node.sat_count || undefined
      },
      classes: `class-card kind-${node.kind || 'class'}`
    };

    nodeMap.set(node.id, cytoscapeNode);
    elements.push(cytoscapeNode);
  });

  // 3. Ingest Directed Strongly-Typed Edges
  (graphIr.edges || []).forEach(edge => {
    if (!nodeMap.has(edge.source) || !nodeMap.has(edge.target)) {
      return;
    }

    const edgeKind = edge.kind || edge.uml_kind || 'association';
    elements.push({
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        uml_kind: edgeKind,
        label: edge.label || '',
        arrow: edge.arrow || '-->'
      },
      classes: `edge-${edgeKind}`
    });
  });

  return elements;
}
