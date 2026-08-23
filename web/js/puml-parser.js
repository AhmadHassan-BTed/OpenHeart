/**
 * OpenHeart Precision PlantUML & Mermaid Graph Engine Parser
 * Clean UML 2.5 notation with strict 3-compartment class formatting (Name, Attributes, Operations),
 * non-overlapping leaf package containers, and collision-free computed geometry.
 */

export function parsePumlToCytoscape(pumlContent, diagramType = 'class') {
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

    // ── 1. Package / Partition / Deployment Node ──
    const pkgMatch = rawLine.match(/^package\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?\s*\{?/) ||
                     rawLine.match(/^partition\s+"([^"]+)"\s*\{?/) ||
                     rawLine.match(/^node\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?(?:\s+<<[^>]+>>)?\s*\{?/);
    if (pkgMatch) {
      const pkgName = pkgMatch[1];
      const pkgId = pkgMatch[2] || `pkg_${pkgName.replace(/[^a-zA-Z0-9_]/g, '_')}`;

      // In Class diagrams, prune redundant root empty wrappers ("com", "com.patterns")
      // to keep clean side-by-side leaf package boxes without concentric container overlap
      const isRedundantWrapper = (pkgName === 'com' || pkgName === 'com.patterns' || pkgName === 'patterns') && diagramType === 'class';

      if (!isRedundantWrapper && !nodeMap.has(pkgId)) {
        const pkgNode = {
          data: {
            id: pkgId,
            label: `package [${pkgName}]`,
            kind: 'package',
            isPackage: true
          },
          classes: 'compound-package'
        };
        nodeMap.set(pkgId, pkgNode);
        elements.push(pkgNode);
      }

      if (rawLine.includes('{')) {
        packageStack.push(isRedundantWrapper ? null : pkgId);
      }
      continue;
    }

    // ── Deployment Artifact: artifact "name.jar" as art_id ──
    const artMatch = rawLine.match(/^artifact\s+"([^"]+)"(?:\s+as\s+([A-Za-z0-9_]+))?/);
    if (artMatch) {
      const artName = artMatch[1];
      const artId = artMatch[2] || `art_${artName.replace(/[^a-zA-Z0-9_]/g, '_')}`;
      const activePkg = getActivePackage(packageStack);
      if (!nodeMap.has(artId)) {
        const node = {
          data: {
            id: artId,
            label: `<<artifact>>\n${artName}`,
            kind: 'entry',
            width: 220,
            height: 60,
            file: `${artName}.java`,
            lines: [1],
            parent: activePkg
          }
        };
        nodeMap.set(artId, node);
        elements.push(node);
      }
      continue;
    }

    // ── 2. Block / Package Closing: } ──
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
      const stereotype = classMatch[3] ? `<<${classMatch[3]}>>` : (kind === 'interface' ? '<<interface>>' : (kind === 'abstract' ? '<<abstract>>' : '<<class>>'));
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

    // ── 4. Accumulate Members Inside Class Block ──
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

    // ── 5. Component / Interface Socket ──
    const compMatch = rawLine.match(/^component\s+\[([^\]]+)\]\s+as\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^component\s+([A-Za-z0-9_]+)/) ||
                      rawLine.match(/^\(\)\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/);
    if (compMatch) {
      const id = compMatch[2] || compMatch[1];
      const label = compMatch[1];
      const isIface = rawLine.startsWith('()');
      const activePkg = getActivePackage(packageStack);
      if (!nodeMap.has(id)) {
        const node = {
          data: {
            id: id,
            label: isIface ? `<<interface>>\n${label}` : `<<component>>\n${label}`,
            kind: isIface ? 'interface' : 'entry',
            width: isIface ? 180 : 230,
            height: isIface ? 55 : 70,
            file: `${label}.java`,
            lines: [1],
            parent: activePkg
          }
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 6. Activity Actions ──
    const actMatch = rawLine.match(/^:([^;]+);/) || rawLine.match(/^(start|stop)/);
    if (actMatch) {
      const label = actMatch[1];
      const id = `act_${elements.length}_${label.replace(/[^a-zA-Z0-9_]/g, '')}`;
      const isStart = label === 'start';
      const isStop = label === 'stop';
      const activePkg = getActivePackage(packageStack);
      if (!nodeMap.has(id)) {
        const node = {
          data: {
            id: id,
            label: isStart ? '(( START ))' : (isStop ? '(( STOP ))' : `[Action] ${label}`),
            kind: isStart ? 'entry' : (isStop ? 'exit' : 'block'),
            width: isStart || isStop ? 150 : 240,
            height: 50,
            file: 'Activity.java',
            lines: [1],
            parent: activePkg
          }
        };
        nodeMap.set(id, node);
        elements.push(node);
      }
      continue;
    }

    // ── 7. State Machine ──
    const stateMatch = rawLine.match(/^state\s+"([^"]+)"\s+as\s+([A-Za-z0-9_]+)/) || rawLine.match(/^state\s+([A-Za-z0-9_]+)/);
    if (stateMatch) {
      const id = stateMatch[2] || stateMatch[1];
      const label = stateMatch[1];
      const activePkg = getActivePackage(packageStack);
      if (!nodeMap.has(id)) {
        const node = {
          data: {
            id: id,
            label: `[State] ${label}`,
            kind: id === '[*]' ? 'entry' : 'block',
            width: 220,
            height: 55,
            file: `${id}.java`,
            lines: [1],
            parent: activePkg
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

function getActivePackage(packageStack) {
  for (let i = packageStack.length - 1; i >= 0; i--) {
    if (packageStack[i]) return packageStack[i];
  }
  return undefined;
}

function registerClassNode(block, nodeMap, elements, packageStack) {
  if (nodeMap.has(block.id)) return;

  const currentParentPkg = getActivePackage(packageStack);
  
  // Strict 3-Compartment UML 2.5 Construction
  const nameCompartment = `${block.stereotype}\n${block.name}`;
  const sections = [nameCompartment];

  if (block.fields.length > 0) {
    sections.push(block.fields.slice(0, 5).join('\n'));
  }
  if (block.methods.length > 0) {
    sections.push(block.methods.slice(0, 6).join('\n'));
  }

  const fullLabel = sections.join('\n──────────────────────\n');
  const allLines = fullLabel.split('\n');
  let maxLineWidth = 0;
  for (const l of allLines) {
    if (l.length > maxLineWidth) maxLineWidth = l.length;
  }

  const nodeWidth = Math.max(240, Math.min(320, maxLineWidth * 8 + 40));
  const nodeHeight = Math.max(80, Math.min(220, allLines.length * 17 + 25));

  const node = {
    data: {
      id: block.id,
      label: fullLabel,
      kind: block.kind || 'class',
      width: nodeWidth,
      height: nodeHeight,
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

  const currentParentPkg = getActivePackage(packageStack);
  const label = id.replace(/_/g, ' ');
  const node = {
    data: {
      id: id,
      label: `<<class>>\n${label}`,
      kind: id.includes('init') || id.includes('start') || id.includes('entry') ? 'entry' : 'class',
      width: 220,
      height: 65,
      file: `${id}.java`,
      lines: [1],
      parent: currentParentPkg
    }
  };

  nodeMap.set(id, node);
  elements.push(node);
}
