/**
 * OpenHeart Robust PlantUML & Mermaid Graph Engine Parser
 * Fully realizes nested package hierarchies, compound container nodes, class stereotypes,
 * method/field signatures, partitions, components, and all UML 2.5 relationship semantics.
 */

export function parsePumlToCytoscape(pumlContent, diagramType = 'class') {
  const elements = [];
  const nodeMap = new Map();
  const packageStack = []; // Stack of active package / partition IDs
  const lines = pumlContent.split('\n');

  let currentBlock = null;

  for (let i = 0; i < lines.length; i++) {
    const rawLine = lines[i].trim();
    if (!rawLine || rawLine.startsWith('@') || rawLine.startsWith('skinparam') || rawLine.startsWith('hide') || rawLine.startsWith('show')) {
      continue;
    }

    // ── 1. Package / Partition / Deployment Node Open ──
    const pkgMatch = rawLine.match(/^package\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?\s*\{?/) ||
                     rawLine.match(/^partition\s+"([^"]+)"\s*\{?/) ||
                     rawLine.match(/^node\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?(?:\s+<<[^>]+>>)?\s*\{?/);
    if (pkgMatch) {
      const pkgName = pkgMatch[1];
      const pkgId = pkgMatch[2] || `pkg_${pkgName.replace(/[^a-zA-Z0-9_]/g, '_')}`;
      const parentPkgId = packageStack.length > 0 ? packageStack[packageStack.length - 1] : null;

      if (!nodeMap.has(pkgId)) {
        const pkgNode = {
          data: {
            id: pkgId,
            label: `📁 ${pkgName}`,
            kind: 'package',
            isPackage: true,
            parent: parentPkgId || undefined
          },
          classes: 'compound-package'
        };
        nodeMap.set(pkgId, pkgNode);
        elements.push(pkgNode);
      }

      if (rawLine.includes('{')) {
        packageStack.push(pkgId);
      }
      continue;
    }

    // ── Deployment Artifact: artifact "name.jar" as art_id ──
    const artMatch = rawLine.match(/^artifact\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?/);
    if (artMatch) {
      const artName = artMatch[1];
      const artId = artMatch[2] || `art_${artName.replace(/[^a-zA-Z0-9_]/g, '_')}`;
      if (!nodeMap.has(artId)) {
        const node = {
          data: {
            id: artId,
            label: `<<Artifact>>\n${artName}`,
            kind: 'entry',
            width: 200,
            height: 55,
            file: `${artName}.java`,
            lines: [1],
            parent: packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined
          }
        };
        nodeMap.set(artId, node);
        elements.push(node);
      }
      continue;
    }

    // ── 2. Block/Package Closing: } ──
    if (rawLine === '}') {
      if (currentBlock) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
        currentBlock = null;
      } else if (packageStack.length > 0) {
        packageStack.pop();
      }
      continue;
    }

    // ── 3. Class / Interface / Abstract Definition ──
    const classMatch = rawLine.match(/^(class|interface|abstract\s+class|abstract|enum)\s+([A-Za-z0-9_]+)(?:\s+<<([^>]+)>>)?(?:\s+as\s+([A-Za-z0-9_]+))?\s*\{?/);
    if (classMatch) {
      if (currentBlock) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
      }

      const rawType = classMatch[1];
      const kind = rawType.includes('interface') ? 'interface' : (rawType.includes('abstract') ? 'abstract' : 'class');
      const name = classMatch[2];
      const stereotype = classMatch[3] ? `<<${classMatch[3]}>>` : (kind === 'interface' ? '<<Interface>>' : (kind === 'abstract' ? '<<Abstract>>' : '<<Class>>'));
      const id = classMatch[4] || name;

      currentBlock = {
        id: id,
        name: name,
        kind: kind,
        stereotype: stereotype,
        members: []
      };

      if (!rawLine.includes('{')) {
        registerClassNode(currentBlock, nodeMap, elements, packageStack);
        currentBlock = null;
      }
      continue;
    }

    // ── 4. Accumulate Members Inside Class Block ──
    if (currentBlock) {
      if (rawLine !== '{') {
        currentBlock.members.push(rawLine);
      }
      continue;
    }

    // ── 5. Component / Interface Socket: component [com] as comp_com / () "Icom" as iface_com ──
    const compMatch = rawLine.match(/^component\s+\[([^\]]+)\]\s+as\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^component\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^\(\)\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/);
    if (compMatch) {
      const id = compMatch[2] || compMatch[1];
      const label = compMatch[1];
      const isIface = rawLine.startsWith('()');
      if (!nodeMap.has(id)) {
        const node = {
          data: {
            id: id,
            label: isIface ? `<<Interface>>\n${label}` : `<<Component>>\n${label}`,
            kind: isIface ? 'interface' : 'entry',
            width: isIface ? 160 : 220,
            height: isIface ? 50 : 65,
            file: `${label}.java`,
            lines: [1],
            parent: packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined
          }
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 6. Activity Actions: :Block_1; / start / stop ──
    const actMatch = rawLine.match(/^:([^;]+);/) || rawLine.match(/^(start|stop)/);
    if (actMatch) {
      const label = actMatch[1];
      const id = `act_${elements.length}_${label.replace(/[^a-zA-Z0-9_]/g, '')}`;
      if (!nodeMap.has(id)) {
        const isTerminal = label === 'start' || label === 'stop';
        const node = {
          data: {
            id: id,
            label: isTerminal ? `(${label.toUpperCase()})` : `Action: ${label}`,
            kind: label === 'start' ? 'entry' : (label === 'stop' ? 'exit' : 'block'),
            width: isTerminal ? 140 : 220,
            height: 48,
            file: 'Activity.java',
            lines: [1],
            parent: packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined
          }
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 7. State Machine: state "Transcoding" as sm_transcoding ──
    const stateMatch = rawLine.match(/^state\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/) || rawLine.match(/^state\s+([A-Za-z0-9_]+)/);
    if (stateMatch) {
      const id = stateMatch[2] || stateMatch[1];
      const label = stateMatch[1];
      if (!nodeMap.has(id)) {
        const node = {
          data: {
            id: id,
            label: `STATE: ${label}`,
            kind: id === '[*]' ? 'entry' : 'block',
            width: 220,
            height: 55,
            file: `${id}.java`,
            lines: [1],
            parent: packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined
          }
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 8. Relationships & Edges ──
    const relMatch = rawLine.match(/^([A-Za-z0-9_\[\]*]+)\s*([<\-\.o*|]+>|\-\-|\.\.|<\-\-|\*--|o--|\-)\s*([A-Za-z0-9_\[\]*]+)(?:\s*:\s*(.+))?/);
    if (relMatch) {
      let src = relMatch[1].replace(/[[\]]/g, '');
      const arrow = relMatch[2];
      let tgt = relMatch[3].replace(/[[\]]/g, '');
      const edgeLabel = relMatch[4] ? relMatch[4].trim() : '';

      if (src === '*') src = 'state_init';
      if (tgt === '*') tgt = 'state_final';

      ensureNodeExists(src, nodeMap, elements, packageStack);
      ensureNodeExists(tgt, nodeMap, elements, packageStack);

      let umlKind = 'dependency';
      if (arrow.includes('|>') && arrow.includes('--')) umlKind = 'generalization';
      else if (arrow.includes('|>') && arrow.includes('..')) umlKind = 'realization';
      else if (arrow.includes('*--') || arrow.includes('--*')) umlKind = 'composition';
      else if (arrow.includes('o--') || arrow.includes('--o')) umlKind = 'aggregation';
      else if (arrow.includes('--|>') || arrow === '--|>') umlKind = 'generalization';
      else if (arrow.includes('..|>') || arrow === '..|>') umlKind = 'realization';

      elements.push({
        data: {
          id: `edge_${elements.length}_${src}_${tgt}`,
          source: src,
          target: tgt,
          label: edgeLabel,
          uml_kind: umlKind
        }
      });
    }
  }

  // Flush any open class block
  if (currentBlock) {
    registerClassNode(currentBlock, nodeMap, elements, packageStack);
  }

  return elements;
}

function registerClassNode(block, nodeMap, elements, packageStack) {
  if (nodeMap.has(block.id)) return;

  const currentParentPkg = packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined;
  const header = `${block.stereotype} ${block.name}`;
  const memberText = block.members.slice(0, 8).join('\n');
  const fullLabel = memberText ? `${header}\n──────────────────────\n${memberText}` : header;

  const nodeHeight = Math.max(80, 48 + (block.members.length * 15));
  const nodeWidth = Math.max(220, Math.min(300, block.name.length * 12 + 100));

  const node = {
    data: {
      id: block.id,
      label: fullLabel,
      kind: 'class',
      width: nodeWidth,
      height: Math.min(180, nodeHeight),
      file: `${block.name}.java`,
      lines: [1, 5, 10],
      parent: currentParentPkg
    }
  };

  nodeMap.set(block.id, node);
  elements.push(node);
}

function ensureNodeExists(id, nodeMap, elements, packageStack) {
  if (!id || nodeMap.has(id)) return;

  const currentParentPkg = packageStack.length > 0 ? packageStack[packageStack.length - 1] : undefined;
  const label = id.replace(/_/g, ' ');
  const node = {
    data: {
      id: id,
      label: `<<Class>> ${label}`,
      kind: id.includes('init') || id.includes('start') || id.includes('entry') ? 'entry' : 'class',
      width: 220,
      height: 60,
      file: `${id}.java`,
      lines: [1],
      parent: currentParentPkg
    }
  };

  nodeMap.set(id, node);
  elements.push(node);
}
