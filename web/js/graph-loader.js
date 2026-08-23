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
  generateCfgBlockSvg,
  generateBddGateSvg
} from './uml-card-renderer.js';

export function loadGraphIrToCytoscape(graphIr) {
  if (!graphIr || !graphIr.nodes) return [];

  const elements = [];
  const nodeMap = new Map();

  // 1. Ingest Nodes Directly from Typed Schema
  graphIr.nodes.forEach(node => {
    let svgData = null;

    if (node.is_package) {
      // Dynamic package folder node
      const shortName = node.name || node.label.replace(/^package\s*\[?/, '').replace(/\]?$/, '');
      const displayLabel = node.is_domain_tier
        ? `DOMAIN LAYER: ${shortName.toUpperCase()}`
        : `package [${shortName}]`;

      const pkgNode = {
        data: {
          id: node.id,
          label: displayLabel,
          rawName: node.name,
          kind: 'package',
          isPackage: true,
          isDomainTier: node.is_domain_tier,
          parent: node.parent || undefined,
          nestLevel: node.nest_level || 0
        },
        classes: `compound-package ${node.is_domain_tier ? 'pkg-domain-tier' : 'pkg-subpackage'} nest-level-${Math.min(5, node.nest_level || 0)}`
      };
      nodeMap.set(node.id, pkgNode);
      elements.push(pkgNode);
      return;
    }

    // Leaf Vector Card generation based on typed kind
    if (node.kind === 'bb') {
      svgData = generateCfgBlockSvg({
        id: node.id,
        label: node.label,
        instructions: node.instructions || [],
        width: 280
      });
    } else if (node.kind === 'bdd_gate' || node.kind === 'bdd_terminal') {
      svgData = generateBddGateSvg({
        varName: node.name,
        isTerminal: node.kind === 'bdd_terminal',
        terminalValue: node.id === '1' ? 1 : 0
      });
    } else if (node.kind === 'state') {
      svgData = generateStateNodeSvg({ name: node.name, width: 230 });
    } else if (node.kind === 'action') {
      svgData = generateActionNodeSvg({
        name: node.name,
        isStart: node.name === 'start',
        isStop: node.name === 'stop',
        width: 230
      });
    } else if (node.kind === 'component') {
      svgData = generateComponentNodeSvg({ name: node.name, width: 230 });
    } else if (node.kind === 'device' || node.kind === 'artifact') {
      svgData = generateDeploymentNodeSvg({ name: node.name, isArtifact: node.kind === 'artifact', width: 230 });
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
        width: 290
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

  // 2. Ingest Edges Directly from Typed Schema
  (graphIr.edges || []).forEach((edge, idx) => {
    elements.push({
      data: {
        id: edge.id || `edge_${idx}_${edge.source}_${edge.target}`,
        source: edge.source,
        target: edge.target,
        label: edge.label || '',
        arrow: edge.arrow || '-->',
        uml_kind: edge.kind || 'association'
      }
    });
  });

  return elements;
}
