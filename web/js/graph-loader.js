/**
 * OpenHeart Official Direct Typed Graph IR Ingestion Engine
 * Ingests typed JSON Graph IR directly from the Rust compiler with ZERO intermediate text parsing.
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
  generateBddGateSvg
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
      svgData = generateStateNodeSvg({ name: node.name, width: 230, isDark });
    } else if (node.kind === 'action') {
      svgData = generateActionNodeSvg({
        name: node.name,
        isStart: node.name === 'start',
        isStop: node.name === 'stop',
        width: 230,
        isDark
      });
    } else if (node.kind === 'component') {
      svgData = generateComponentNodeSvg({ name: node.name, width: 230, isDark });
    } else if (node.kind === 'device' || node.kind === 'artifact') {
      svgData = generateDeploymentNodeSvg({ name: node.name, isArtifact: node.kind === 'artifact', width: 230, isDark });
    } else if (node.kind === 'participant' || node.kind === 'actor') {
      svgData = generateSequenceLifelineSvg({ name: node.name, isActor: node.kind === 'actor', width: 200, isDark });
    } else if (node.kind === 'usecase') {
      svgData = generateUseCaseSvg({ name: node.name, width: 220, isDark });
    } else if (node.kind === 'object') {
      svgData = generateObjectCardSvg({
        name: node.name,
        fields: (node.fields || []).map(f => f.signature || f.name),
        width: 260,
        isDark
      });
    } else {
      // UML Class / Interface / Abstract / Enum Card
      const fieldsFormatted = (node.fields || []).map(f => `${f.visibility} ${f.signature || f.name}`);
      const methodsFormatted = (node.methods || []).map(m => `${m.visibility} ${m.signature || m.name}`);

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
        lines: node.lines || [1],
        parent: node.parent || undefined,
        nestLevel: node.nest_level || 0
      },
      classes: `class-card kind-${node.kind || 'class'}`
    };

    nodeMap.set(node.id, cytoscapeNode);
    elements.push(cytoscapeNode);
  });

  // 2. Ingest Directed Strongly-Typed Edges
  graphIr.edges.forEach(edge => {
    // Prevent dangling edges if either endpoint is absent
    if (!nodeMap.has(edge.source) || !nodeMap.has(edge.target)) {
      return;
    }

    elements.push({
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        uml_kind: edge.kind,
        label: edge.label || '',
        arrow: edge.arrow || '-->'
      },
      classes: `edge-${edge.kind}`
    });
  });

  return elements;
}
