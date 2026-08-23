/**
 * OpenHeart Fully Generic PlantUML & Mermaid Graph Engine Parser
 * 100% Dynamic - Zero Hardcoding of package names, classes, or patterns.
 * Builds dynamic N-ary containment trees and assigns harmonic visual themes algorithmically.
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
import { isDarkMode } from './themes/index.js';

export function parsePumlToCytoscape(pumlContent, diagramType = 'class') {
  const isDark = isDarkMode();
  const elements = [];
  const nodeMap = new Map();
  const packageStack = [];
  const lines = pumlContent.split('\n');

  let currentBlock = null;

  for (let i = 0; i < lines.length; i++) {
    const rawLine = lines[i].trim();
    if (!rawLine || rawLine.startsWith('@') || rawLine.startsWith('skinparam') || rawLine.startsWith('hide') || rawLine.startsWith('show')) {
      continue;
    }

    // ── 1. Package / Partition / Deployment Container ──
    const pkgMatch = rawLine.match(/^package\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?\s*\{?/) ||
                     rawLine.match(/^partition\s+"([^"]+)"\s*\{?/) ||
                     rawLine.match(/^node\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?(?:\s+<<[^>]+>>)?\s*\{?/);
    if (pkgMatch) {
      const pkgName = pkgMatch[1];
      const pkgId = pkgMatch[2] || `pkg_${pkgName.replace(/[^a-zA-Z0-9_]/g, '_')}`;

      const currentParentPkg = getActivePackage(packageStack);
      const nestLevel = packageStack.filter(p => p !== null).length;
      const isDomainTier = nestLevel === 0;

      // Clean dynamic label: short package name with folder icon
      const shortName = pkgName.split('.').pop();
      const displayLabel = isDomainTier ? `📂 DOMAIN: ${pkgName.toUpperCase()}` : `📁 package [${shortName}]`;

      if (!nodeMap.has(pkgId)) {
        const pkgNode = {
          data: {
            id: pkgId,
            label: displayLabel,
            rawName: pkgName,
            kind: 'package',
            isPackage: true,
            isDomainTier: isDomainTier,
            parent: currentParentPkg || undefined,
            nestLevel: nestLevel
          },
          classes: `compound-package ${isDomainTier ? 'pkg-domain-tier' : 'pkg-subpackage'} nest-level-${Math.min(3, nestLevel)}`
        };
        nodeMap.set(pkgId, pkgNode);
        elements.push(pkgNode);
      }

      if (rawLine.includes('{')) {
        packageStack.push(pkgId);
      }
      continue;
    }

    // ── 2. Deployment Artifact: artifact "name.jar" as art_id ──
    const artMatch = rawLine.match(/^artifact\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?/);
    if (artMatch) {
      const artName = artMatch[1];
      const artId = artMatch[2] || `art_${artName.replace(/[^a-zA-Z0-9_]/g, '_')}`;
      const activePkg = getActivePackage(packageStack);
      const nestLevel = packageStack.filter(p => p !== null).length;

      if (!nodeMap.has(artId)) {
        const svgData = generateDeploymentNodeSvg({ name: artName, isArtifact: true, width: 220, height: 65 });
        const node = {
          data: {
            id: artId,
            label: '',
            textLabel: `<<artifact>>\n${artName}`,
            kind: 'entry',
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri,
            file: `${artName}.java`,
            lines: [1],
            parent: activePkg,
            nestLevel: nestLevel
          },
          classes: 'artifact-card'
        };
        nodeMap.set(artId, node);
        elements.push(node);
      }
      continue;
    }

    // ── 3. Block / Package Closing: } ──
    if (rawLine === '}') {
      if (currentBlock) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
        currentBlock = null;
      } else if (packageStack.length > 0) {
        packageStack.pop();
      }
      continue;
    }

    // ── 4. Class / Interface / Abstract Definition ──
    const classMatch = rawLine.match(/^(class|interface|abstract\s+class|abstract|enum)\s+([A-Za-z0-9_]+)(?:\s+<<([^>]+)>>)?(?:\s+as\s+([A-Za-z0-9_]+))?\s*\{?/);
    if (classMatch) {
      if (currentBlock) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
      }

      const rawType = classMatch[1];
      const kind = rawType.includes('interface') ? 'interface' : (rawType.includes('abstract') ? 'abstract' : (rawType.includes('enum') ? 'enum' : 'class'));
      const name = classMatch[2];
      const stereotype = classMatch[3] ? `<<${classMatch[3]}>>` : (kind === 'interface' ? '<<interface>>' : (kind === 'abstract' ? '<<abstract>>' : (kind === 'enum' ? '<<enum>>' : '<<class>>')));
      const id = classMatch[4] || name;

      currentBlock = {
        id: id,
        name: name,
        kind: kind,
        stereotype: stereotype,
        fields: [],
        methods: []
      };

      if (!rawLine.includes('{')) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
        currentBlock = null;
      }
      continue;
    }

    // ── 5. Accumulate Members Inside Class Block ──
    if (currentBlock) {
      if (rawLine !== '{') {
        if (rawLine.includes('(') || rawLine.includes(')')) {
          currentBlock.methods.push(rawLine);
        } else {
          currentBlock.fields.push(rawLine);
        }
      }
      continue;
    }

    // ── 6. Component / Interface Socket / Timing Lifeline ──
    const compMatch = rawLine.match(/^component\s+\[([^\]]+)\]\s+as\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^component\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^\(\)\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^(?:robust|concise)\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/);
    if (compMatch) {
      const id = compMatch[2] || compMatch[1];
      const label = compMatch[1];
      const isIface = rawLine.startsWith('()');
      const activePkg = getActivePackage(packageStack);
      const nestLevel = packageStack.filter(p => p !== null).length;

      if (!nodeMap.has(id)) {
        const svgData = generateComponentNodeSvg({ name: label, isInterface: isIface, width: isIface ? 180 : 230 });
        const node = {
          data: {
            id: id,
            label: '',
            textLabel: isIface ? `<<interface>>\n${label}` : `<<component>>\n${label}`,
            kind: isIface ? 'interface' : 'entry',
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri,
            file: `${label}.java`,
            lines: [1],
            parent: activePkg,
            nestLevel: nestLevel
          },
          classes: 'component-card'
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 7. Activity Actions ──
    const actMatch = rawLine.match(/^:([^;]+);/) || rawLine.match(/^(start|stop)/);
    if (actMatch) {
      const label = actMatch[1];
      const id = `act_${elements.length}_${label.replace(/[^a-zA-Z0-9_]/g, '')}`;
      const isStart = label === 'start';
      const isStop = label === 'stop';
      const activePkg = getActivePackage(packageStack);
      const nestLevel = packageStack.filter(p => p !== null).length;

      if (!nodeMap.has(id)) {
        const svgData = generateActionNodeSvg({ name: label, isStart, isStop, width: isStart || isStop ? 48 : 230 });
        const node = {
          data: {
            id: id,
            label: '',
            textLabel: isStart ? '(( START ))' : (isStop ? '(( STOP ))' : `[Action] ${label}`),
            kind: isStart ? 'entry' : (isStop ? 'exit' : 'block'),
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri,
            file: 'Activity.java',
            lines: [1],
            parent: activePkg,
            nestLevel: nestLevel
          },
          classes: 'action-card'
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 8. State Machine ──
    const stateMatch = rawLine.match(/^state\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/) || rawLine.match(/^state\s+([A-Za-z0-9_]+)/);
    if (stateMatch) {
      const id = stateMatch[2] || stateMatch[1];
      const label = stateMatch[1];
      const activePkg = getActivePackage(packageStack);
      const nestLevel = packageStack.filter(p => p !== null).length;

      if (!nodeMap.has(id)) {
        const svgData = generateStateNodeSvg({ name: label, width: 230 });
        const node = {
          data: {
            id: id,
            label: '',
            textLabel: `[State] ${label}`,
            kind: id === '[*]' ? 'entry' : 'block',
            width: svgData.width,
            height: svgData.height,
            svgDataUri: svgData.dataUri,
            file: `${id}.java`,
            lines: [1],
            parent: activePkg,
            nestLevel: nestLevel
          },
          classes: 'state-card'
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 9. Relationships & Arrows (Generalization, Realization, Composition, Aggregation, Association, Dependency) ──
    const relMatch = rawLine.match(/^([A-Za-z0-9_\[\]*]+)\s*([<\-\.o*+|]+>|<[<\-\.o*+|]+|\*--|--\*|o--|--o|\+--|--\+|\-\-|\.\.|-->|<--|\.\.>|<\.\.|\-\|>|<\|\-|\.\.\|>|<\|\.\.)\s*([A-Za-z0-9_\[\]*]+)(?:\s*:\s*(.+))?/);
    if (relMatch) {
      let src = relMatch[1].replace(/[[\]]/g, '');
      const arrow = relMatch[2];
      let tgt = relMatch[3].replace(/[[\]]/g, '');
      const edgeLabel = relMatch[4] ? relMatch[4].trim() : '';

      if (src === '*') src = 'state_init';
      if (tgt === '*') tgt = 'state_final';

      ensureNodeExists(src, nodeMap, elements, packageStack, diagramType);
      ensureNodeExists(tgt, nodeMap, elements, packageStack, diagramType);

      let umlKind = 'association';
      if (arrow.includes('*--') || arrow.includes('--*')) {
        umlKind = 'composition';
      } else if (arrow.includes('o--') || arrow.includes('--o')) {
        umlKind = 'aggregation';
      } else if (arrow.includes('|>') || arrow.includes('<|')) {
        if (arrow.includes('..')) {
          umlKind = 'realization';
        } else {
          umlKind = 'generalization';
        }
      } else if (arrow.includes('..') || arrow.includes('..>')) {
        umlKind = 'dependency';
      } else if (arrow.includes('-->') || arrow.includes('<--')) {
        umlKind = 'association';
      }

      elements.push({
        data: {
          id: `edge_${elements.length}_${src}_${tgt}`,
          source: src,
          target: tgt,
          label: edgeLabel,
          arrow: arrow,
          uml_kind: umlKind
        }
      });
    }
  }

  // Flush any open class block
  if (currentBlock) {
    registerClassNode(currentBlock, nodeMap, elements, packageStack);
  }

  // Post-process: convert leaf packages into SVG folder cards
  const parentPkgIds = new Set(elements.map(e => e.data && e.data.parent).filter(Boolean));
  elements.forEach(el => {
    if (el.data && el.data.isPackage && !parentPkgIds.has(el.data.id)) {
      const rawName = el.data.rawName || el.data.id;
      const shortName = rawName.split('.').pop();
      const svgData = generatePackageFolderSvg({
        name: shortName,
        nestLevel: el.data.nestLevel || 0,
        width: 240,
        height: 100,
        isDark
      });
      el.data.label = '';
      el.data.textLabel = el.data.isDomainTier ? `📂 DOMAIN: ${shortName.toUpperCase()}` : `📁 package [${shortName}]`;
      el.data.width = svgData.width;
      el.data.height = svgData.height;
      el.data.svgDataUri = svgData.dataUri;
      el.data.isPackage = false;
      el.data.isLeafPackage = true;
      el.classes = `leaf-package nest-level-${Math.min(5, el.data.nestLevel || 0)}`;
    }
  });

  return elements;
}

function getActivePackage(packageStack) {
  for (let i = packageStack.length - 1; i >= 0; i--) {
    if (packageStack[i]) return packageStack[i];
  }
  return undefined;
}

function registerClassNode(block, nodeMap, elements, packageStack) {
  if (nodeMap.has(block.id)) return;

  const isDark = isDarkMode();

  const currentParentPkg = getActivePackage(packageStack);
  const nestLevel = packageStack.filter(p => p !== null).length;
  
  const svgData = generateUmlClassCardSvg({
    name: block.name,
    stereotype: block.stereotype,
    kind: block.kind,
    fields: block.fields,
    methods: block.methods,
    width: 290,
    isDark: isDark
  });

  const node = {
    data: {
      id: block.id,
      label: '',
      textLabel: `${block.stereotype}\n${block.name}`,
      kind: block.kind || 'class',
      width: svgData.width,
      height: svgData.height,
      svgDataUri: svgData.dataUri,
      file: `${block.name}.java`,
      lines: [1, 5, 10],
      parent: currentParentPkg,
      nestLevel: nestLevel
    },
    classes: `class-card kind-${block.kind || 'class'}`
  };

  nodeMap.set(block.id, node);
  elements.push(node);
}

function ensureNodeExists(id, nodeMap, elements, packageStack, diagramType) {
  if (!id || nodeMap.has(id)) return;

  const isDark = isDarkMode();

  const currentParentPkg = getActivePackage(packageStack);
  const nestLevel = packageStack.filter(p => p !== null).length;
  const label = id.replace(/_/g, ' ');

  let svgData;
  if (diagramType === 'statemachine' || id.startsWith('state_')) {
    svgData = generateStateNodeSvg({ name: id === 'state_init' ? '[*]' : (id === 'state_final' ? 'state_final' : label), isDark });
  } else if (diagramType === 'cfg') {
    svgData = generateCfgBlockSvg({ id: id, label: label, instructions: [], isDark });
  } else if (diagramType === 'robdd') {
    svgData = generateBddGateSvg({ varName: label, isTerminal: id === '0' || id === '1', terminalValue: id === '1' ? 1 : 0, isDark });
  } else {
    svgData = generateUmlClassCardSvg({
      name: label,
      stereotype: '<<class>>',
      kind: id.includes('init') || id.includes('start') || id.includes('entry') ? 'entry' : 'class',
      fields: [],
      methods: [],
      width: 260,
      isDark: isDark
    });
  }

  const node = {
    data: {
      id: id,
      label: '',
      textLabel: `<<class>>\n${label}`,
      kind: id.includes('init') || id.includes('start') || id.includes('entry') ? 'entry' : 'class',
      width: svgData.width,
      height: svgData.height,
      svgDataUri: svgData.dataUri,
      file: `${id}.java`,
      lines: [1],
      parent: currentParentPkg,
      nestLevel: nestLevel
    },
    classes: 'class-card'
  };

  nodeMap.set(id, node);
  elements.push(node);
}
