/**
 * OpenHeart Deterministic UML Spatial Layout Engine
 * Computes exact collision-free bounding boxes and non-overlapping coordinate grids.
 * Mathematically guarantees ZERO overlap between packages, classes, and compiler nodes.
 */

export function computeDeterministicLayout(elements, graphType = 'class') {
  const nodeMap = new Map();
  const packageMap = new Map();
  const standaloneNodes = [];
  const edges = [];

  // 1. Separate nodes, packages, and edges
  elements.forEach(el => {
    if (el.data.source && el.data.target) {
      edges.push(el);
    } else if (el.data.isPackage) {
      packageMap.set(el.data.id, {
        element: el,
        children: []
      });
      nodeMap.set(el.data.id, el);
    } else {
      nodeMap.set(el.data.id, el);
      if (el.data.parent && packageMap.has(el.data.parent)) {
        packageMap.get(el.data.parent).children.push(el);
      } else {
        standaloneNodes.push(el);
      }
    }
  });

  // If this is a hierarchical graph (CFG, ROBDD, Call Graph, State Machine, Sequence, Activity)
  if (['cfg', 'robdd', 'dfg', 'cdg', 'callgraph', 'statemachine', 'sequence', 'activity'].includes(graphType)) {
    return layoutHierarchicalGraph(nodeMap, edges, elements);
  }

  // 2. Compute Layout for Class / Package / Component / Composite diagrams
  const PACKAGES_PER_ROW = 3;
  const PKG_GUTTER_X = 140;
  const PKG_GUTTER_Y = 160;
  const CARD_WIDTH = 280;
  const CARD_GAP_X = 50;
  const CARD_GAP_Y = 40;
  const PKG_PAD_TOP = 80;
  const PKG_PAD_BOTTOM = 40;
  const PKG_PAD_SIDE = 40;

  let pkgIndex = 0;
  let rowMaxHeight = 0;
  let currentOriginX = 0;
  let currentOriginY = 0;

  packageMap.forEach((pkgData, pkgId) => {
    const children = pkgData.children;
    const colCount = Math.min(2, Math.max(1, children.length));
    const rowCount = Math.ceil(children.length / colCount);

    // Calculate max child dimensions
    let maxChildHeight = 90;
    children.forEach(child => {
      const h = child.data.height || 100;
      if (h > maxChildHeight) maxChildHeight = h;
    });

    const innerWidth = colCount * CARD_WIDTH + (colCount - 1) * CARD_GAP_X;
    const innerHeight = rowCount * maxChildHeight + (rowCount - 1) * CARD_GAP_Y;

    const pkgWidth = innerWidth + PKG_PAD_SIDE * 2;
    const pkgHeight = innerHeight + PKG_PAD_TOP + PKG_PAD_BOTTOM;

    // Grid placement for package
    const colIndex = pkgIndex % PACKAGES_PER_ROW;
    if (colIndex === 0 && pkgIndex > 0) {
      currentOriginX = 0;
      currentOriginY += rowMaxHeight + PKG_GUTTER_Y;
      rowMaxHeight = 0;
    }

    const pkgX = currentOriginX;
    const pkgY = currentOriginY;

    if (pkgHeight > rowMaxHeight) {
      rowMaxHeight = pkgHeight;
    }

    // Set package position & size
    pkgData.element.data.width = pkgWidth;
    pkgData.element.data.height = pkgHeight;
    pkgData.element.position = {
      x: pkgX + pkgWidth / 2,
      y: pkgY + pkgHeight / 2
    };

    // Position children inside package
    children.forEach((child, cIdx) => {
      const cCol = cIdx % colCount;
      const cRow = Math.floor(cIdx / colCount);

      const childX = pkgX + PKG_PAD_SIDE + cCol * (CARD_WIDTH + CARD_GAP_X) + CARD_WIDTH / 2;
      const childY = pkgY + PKG_PAD_TOP + cRow * (maxChildHeight + CARD_GAP_Y) + (child.data.height || maxChildHeight) / 2;

      child.data.width = CARD_WIDTH;
      child.position = {
        x: childX,
        y: childY
      };
    });

    currentOriginX += pkgWidth + PKG_GUTTER_X;
    pkgIndex++;
  });

  // 3. Position any standalone nodes in a bottom shelf
  if (standaloneNodes.length > 0) {
    let shelfX = 0;
    const shelfY = currentOriginY + rowMaxHeight + PKG_GUTTER_Y;
    standaloneNodes.forEach((node, sIdx) => {
      const w = node.data.width || CARD_WIDTH;
      const h = node.data.height || 100;
      node.position = {
        x: shelfX + w / 2,
        y: shelfY + h / 2
      };
      shelfX += w + CARD_GAP_X;
    });
  }

  return elements;
}

function layoutHierarchicalGraph(nodeMap, edges, elements) {
  // Topological rank assignment
  const inDegree = new Map();
  const adj = new Map();
  const nodes = [];

  nodeMap.forEach((node, id) => {
    nodes.push(node);
    inDegree.set(id, 0);
    adj.set(id, []);
  });

  edges.forEach(edge => {
    const src = edge.data.source;
    const tgt = edge.data.target;
    if (adj.has(src) && inDegree.has(tgt)) {
      adj.get(src).push(tgt);
      inDegree.set(tgt, inDegree.get(tgt) + 1);
    }
  });

  // Kahn rank assignment
  const queue = [];
  const ranks = new Map();

  nodeMap.forEach((node, id) => {
    if (inDegree.get(id) === 0) {
      queue.push(id);
      ranks.set(id, 0);
    }
  });

  if (queue.length === 0 && nodes.length > 0) {
    queue.push(nodes[0].data.id);
    ranks.set(nodes[0].data.id, 0);
  }

  while (queue.length > 0) {
    const u = queue.shift();
    const currRank = ranks.get(u) || 0;

    const neighbors = adj.get(u) || [];
    neighbors.forEach(v => {
      const existingRank = ranks.get(v) || 0;
      if (currRank + 1 > existingRank) {
        ranks.set(v, currRank + 1);
      }
      inDegree.set(v, inDegree.get(v) - 1);
      if (inDegree.get(v) === 0) {
        queue.push(v);
      }
    });
  }

  // Handle any disconnected or cyclic nodes
  nodes.forEach(n => {
    if (!ranks.has(n.data.id)) {
      ranks.set(n.data.id, 0);
    }
  });

  // Group by rank
  const rankGroups = new Map();
  ranks.forEach((r, id) => {
    if (!rankGroups.has(r)) rankGroups.set(r, []);
    rankGroups.get(r).push(nodeMap.get(id));
  });

  const LEVEL_GAP_Y = 160;
  const NODE_GAP_X = 80;
  let currentY = 0;

  const sortedRanks = Array.from(rankGroups.keys()).sort((a, b) => a - b);
  sortedRanks.forEach(rank => {
    const group = rankGroups.get(rank);
    let totalWidth = 0;
    group.forEach(n => {
      totalWidth += (n.data.width || 240) + NODE_GAP_X;
    });
    totalWidth -= NODE_GAP_X;

    let currentX = -totalWidth / 2;
    let maxH = 60;

    group.forEach(n => {
      const w = n.data.width || 240;
      const h = n.data.height || 60;
      if (h > maxH) maxH = h;

      n.position = {
        x: currentX + w / 2,
        y: currentY + h / 2
      };
      currentX += w + NODE_GAP_X;
    });

    currentY += maxH + LEVEL_GAP_Y;
  });

  return elements;
}
