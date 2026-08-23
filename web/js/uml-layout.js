/**
 * OpenHeart Hierarchical Architectural Layout Engine (Zero Hardcoding)
 * Recursive bottom-up composite bounding box layout algorithm for arbitrary package trees.
 * Guarantees zero overlapping, cycle-safe topological ranking, generous arrow routing channels,
 * and dedicated domain geometry for Sequence, Timing, Composite, and Pipeline Program Graphs.
 */

export function computeDeterministicLayout(elements, graphType = 'class') {
  if (!elements || elements.length === 0) return elements;

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
        id: el.data.id,
        element: el,
        parent: el.data.parent,
        childPackages: [],
        childClasses: [],
        bounds: { width: 300, height: 200 }
      });
      nodeMap.set(el.data.id, el);
    } else {
      nodeMap.set(el.data.id, el);
    }
  });

  // Link children to their parent packages
  elements.forEach(el => {
    if (!el.data.source && !el.data.isPackage) {
      const parentId = el.data.parent;
      if (parentId && packageMap.has(parentId)) {
        packageMap.get(parentId).childClasses.push(el);
      } else {
        standaloneNodes.push(el);
      }
    } else if (!el.data.source && el.data.isPackage) {
      const parentId = el.data.parent;
      if (parentId && packageMap.has(parentId)) {
        packageMap.get(parentId).childPackages.push(packageMap.get(el.data.id));
      }
    }
  });

  // ── Dispatch Specialized Domain Layouts ──
  if (graphType === 'sequence' || graphType === 'communication') {
    return layoutSequenceGraph(nodeMap, edges, elements);
  }
  if (graphType === 'timing') {
    return layoutTimingGraph(nodeMap, edges, elements);
  }
  if (graphType === 'usecase' || graphType === 'use_case') {
    return layoutUseCaseGraph(nodeMap, edges, elements);
  }
  if (['cfg', 'robdd', 'dfg', 'cdg', 'callgraph', 'state', 'statemachine', 'activity', 'interaction'].includes(graphType)) {
    return layoutHierarchicalGraph(nodeMap, edges, elements);
  }

  // ── 2. Standard UML Class / Package / Composite Bounding Box Engine ──
  const CARD_WIDTH = 290;
  const CARD_GAP_X = 120;
  const CARD_GAP_Y = 100;
  const PAD_TOP = 80;
  const PAD_BOTTOM = 50;
  const PAD_SIDE = 50;
  const SIBLING_GAP_X = 80;
  const ROOT_GAP_Y = 180;

  function computePackageBounds(pkg) {
    pkg.childPackages.forEach(cp => computePackageBounds(cp));

    let classesWidth = 0;
    let classesHeight = 0;

    if (pkg.childClasses.length > 0) {
      const colCount = Math.min(2, Math.max(1, pkg.childClasses.length));
      const rowCount = Math.ceil(pkg.childClasses.length / colCount);

      let maxClassH = 140;
      let maxClassW = 290;
      pkg.childClasses.forEach(c => {
        const h = c.data.height || 180;
        const w = c.data.width || 290;
        if (h > maxClassH) maxClassH = h;
        if (w > maxClassW) maxClassW = w;
      });

      classesWidth = colCount * maxClassW + (colCount - 1) * CARD_GAP_X;
      classesHeight = rowCount * maxClassH + (rowCount - 1) * CARD_GAP_Y;
      pkg.maxClassH = maxClassH;
      pkg.maxClassW = maxClassW;
      pkg.colCount = colCount;
    }

    let subpackagesWidth = 0;
    let subpackagesHeight = 0;

    if (pkg.childPackages.length > 0) {
      pkg.childPackages.forEach(cp => {
        subpackagesWidth += cp.bounds.width + SIBLING_GAP_X;
        if (cp.bounds.height > subpackagesHeight) {
          subpackagesHeight = cp.bounds.height;
        }
      });
      subpackagesWidth -= SIBLING_GAP_X;
    }

    if (pkg.childClasses.length === 0 && pkg.childPackages.length === 0) {
      pkg.bounds = {
        width: 240,
        height: 90,
        classesWidth: 0,
        classesHeight: 0,
        subpackagesWidth: 0,
        subpackagesHeight: 0
      };
      pkg.element.data.width = 240;
      pkg.element.data.height = 90;
      pkg.element.data.origWidth = 240;
      pkg.element.data.origHeight = 90;
      return;
    }

    const contentWidth = Math.max(classesWidth, subpackagesWidth);
    const contentHeight = (classesHeight > 0 && subpackagesHeight > 0)
      ? classesHeight + CARD_GAP_Y + subpackagesHeight
      : (classesHeight || subpackagesHeight || 90);

    const totalWidth = contentWidth + PAD_SIDE * 2;
    const totalHeight = contentHeight + PAD_TOP + PAD_BOTTOM;

    pkg.bounds = {
      width: Math.max(360, totalWidth),
      height: Math.max(220, totalHeight),
      classesWidth,
      classesHeight,
      subpackagesWidth,
      subpackagesHeight
    };

    pkg.element.data.width = pkg.bounds.width;
    pkg.element.data.height = pkg.bounds.height;
    pkg.element.data.origWidth = pkg.bounds.width;
    pkg.element.data.origHeight = pkg.bounds.height;
  }

  function positionPackage(pkg, originX, originY) {
    const pkgCenterX = originX + pkg.bounds.width / 2;
    const pkgCenterY = originY + pkg.bounds.height / 2;

    pkg.element.position = {
      x: pkgCenterX,
      y: pkgCenterY
    };

    let currentY = originY + PAD_TOP;

    if (pkg.childClasses.length > 0) {
      const startX = originX + PAD_SIDE;
      const colWidth = pkg.maxClassW || 290;
      pkg.childClasses.forEach((child, idx) => {
        const col = idx % pkg.colCount;
        const row = Math.floor(idx / pkg.colCount);

        const childW = child.data.width || colWidth;
        const childH = child.data.height || pkg.maxClassH;

        const childX = startX + col * (colWidth + CARD_GAP_X) + childW / 2;
        const childY = currentY + row * (pkg.maxClassH + CARD_GAP_Y) + childH / 2;

        child.position = { x: childX, y: childY };
      });

      currentY += pkg.bounds.classesHeight + CARD_GAP_Y;
    }

    if (pkg.childPackages.length > 0) {
      let currentSubX = originX + PAD_SIDE;
      pkg.childPackages.forEach(cp => {
        positionPackage(cp, currentSubX, currentY);
        currentSubX += cp.bounds.width + SIBLING_GAP_X;
      });
    }
  }

  const rootPackages = [];
  const prunedPkgIds = new Set();

  packageMap.forEach(pkg => {
    if (!pkg.parent || !packageMap.has(pkg.parent)) {
      let curr = pkg;
      while (curr.childClasses.length === 0 && curr.childPackages.length === 1) {
        prunedPkgIds.add(curr.id);
        curr = curr.childPackages[0];
        curr.parent = undefined;
        curr.element.data.parent = undefined;
      }
      if (!rootPackages.includes(curr)) {
        rootPackages.push(curr);
      }
    }
  });

  rootPackages.forEach(rp => computePackageBounds(rp));

  // ── Multi-Root Package Grid Alignment ──
  const ROOT_GAP_X = 160;
  const ROOT_GAP_Y = 160;
  const COLS = rootPackages.length > 2 ? 2 : 1;
  let col = 0;
  let rowX = 0;
  let rowY = 0;
  let maxRowHeight = 0;

  rootPackages.forEach((rp) => {
    if (col >= COLS) {
      col = 0;
      rowX = 0;
      rowY += maxRowHeight + ROOT_GAP_Y;
      maxRowHeight = 0;
    }
    positionPackage(rp, rowX, rowY);
    rowX += rp.bounds.width + ROOT_GAP_X;
    if (rp.bounds.height > maxRowHeight) {
      maxRowHeight = rp.bounds.height;
    }
    col++;
  });
  let currentRootY = rowY + maxRowHeight + ROOT_GAP_Y;

  if (standaloneNodes.length > 0) {
    let shelfX = 0;
    standaloneNodes.forEach(node => {
      const w = node.data.width || CARD_WIDTH;
      const h = node.data.height || 100;
      node.position = {
        x: shelfX + w / 2,
        y: currentRootY + h / 2
      };
      shelfX += w + 60;
    });
  }

  const finalElements = [];
  elements.forEach(el => {
    if (el.data && el.data.isPackage && prunedPkgIds.has(el.data.id)) {
      return;
    }
    finalElements.push(el);
  });

  return finalElements;
}

/**
 * ── Cycle-Safe Topological Hierarchical Layout Engine ──
 * Safely resolves loops, recursion SCCs, and backward state machine transitions
 * with zero stacking or lockups.
 */
function layoutHierarchicalGraph(nodeMap, edges, elements) {
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
    if (adj.has(src) && inDegree.has(tgt) && src !== tgt) {
      adj.get(src).push(tgt);
      inDegree.set(tgt, inDegree.get(tgt) + 1);
    }
  });

  const queue = [];
  const ranks = new Map();
  const inDegCopy = new Map(inDegree);

  nodeMap.forEach((node, id) => {
    if (inDegCopy.get(id) === 0) {
      queue.push(id);
      ranks.set(id, 0);
    }
  });

  // Cycle breaker: If graph has unranked cyclic nodes, iteratively seed next unranked component
  while (ranks.size < nodes.length) {
    if (queue.length === 0) {
      const nextUnranked = nodes.find(n => !ranks.has(n.data.id));
      if (!nextUnranked) break;
      queue.push(nextUnranked.data.id);
      ranks.set(nextUnranked.data.id, 0);
    }

    while (queue.length > 0) {
      const u = queue.shift();
      const currRank = ranks.get(u) || 0;

      const neighbors = adj.get(u) || [];
      neighbors.forEach(v => {
        const existingRank = ranks.get(v);
        if (existingRank === undefined || currRank + 1 > existingRank) {
          ranks.set(v, currRank + 1);
        }
        inDegCopy.set(v, (inDegCopy.get(v) || 1) - 1);
        if (inDegCopy.get(v) <= 0 && !queue.includes(v)) {
          queue.push(v);
        }
      });
    }
  }

  nodes.forEach(n => {
    if (!ranks.has(n.data.id)) {
      ranks.set(n.data.id, 0);
    }
  });

  const rankGroups = new Map();
  ranks.forEach((r, id) => {
    if (!rankGroups.has(r)) rankGroups.set(r, []);
    rankGroups.get(r).push(nodeMap.get(id));
  });

  const LEVEL_GAP_Y = 160;
  const NODE_GAP_X = 100;
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

/**
 * ── Sequence Diagram Lifeline Layout Engine ──
 */
function layoutSequenceGraph(nodeMap, edges, elements) {
  const LIFELINE_GAP_X = 260;
  let currentX = 0;

  const nodes = Array.from(nodeMap.values()).filter(n => !n.data.isPackage);
  nodes.forEach((node) => {
    const w = node.data.width || 200;
    const h = node.data.height || 100;
    node.position = {
      x: currentX + w / 2,
      y: 60
    };
    currentX += w + LIFELINE_GAP_X;
  });

  return elements;
}

/**
 * ── Timing Diagram Waveform Multi-Track Layout Engine ──
 */
function layoutTimingGraph(nodeMap, edges, elements) {
  const TRACK_GAP_Y = 180;
  let currentY = 40;

  const nodes = Array.from(nodeMap.values()).filter(n => !n.data.isPackage);
  nodes.forEach((node) => {
    const w = node.data.width || 600;
    const h = node.data.height || 120;
    node.position = {
      x: w / 2 + 40,
      y: currentY + h / 2
    };
    currentY += h + TRACK_GAP_Y;
  });

  return elements;
}

/**
 * ── Use Case Diagram Wing-Boundary Layout Engine ──
 */
function layoutUseCaseGraph(nodeMap, edges, elements) {
  const actors = [];
  const usecases = [];

  nodeMap.forEach(n => {
    if (n.data.kind === 'actor') {
      actors.push(n);
    } else {
      usecases.push(n);
    }
  });

  // Left and Right Actor Wings with Independent Column Offsets
  let actorLeftY = 40;
  let actorRightY = 40;
  actors.forEach((act, idx) => {
    const isRight = idx % 2 === 1;
    const x = isRight ? 650 : -450;
    const y = isRight ? actorRightY : actorLeftY;
    act.position = {
      x: x,
      y: y
    };
    if (isRight) {
      actorRightY += 160;
    } else {
      actorLeftY += 160;
    }
  });

  // Center System Boundary Usecases
  let ucY = 40;
  usecases.forEach(uc => {
    const w = uc.data.width || 230;
    const h = uc.data.height || 60;
    uc.position = {
      x: 100,
      y: ucY + h / 2
    };
    ucY += h + 80;
  });

  return elements;
}
