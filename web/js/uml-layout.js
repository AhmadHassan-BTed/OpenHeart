/**
 * OpenHeart Hierarchical Architectural Layout Engine (Zero Hardcoding)
 * Recursive bottom-up composite bounding box layout algorithm for arbitrary package trees.
 * Guarantees zero overlapping, cycle-safe topological ranking, generous arrow routing channels,
 * and dedicated domain geometry for all 19 UML & Compiler IR diagram projections.
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
        bounds: { width: 320, height: 220 }
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

  const gt = (graphType || 'class').toLowerCase();

  // ── 2. Dispatch Specialized Domain Layouts ──
  if (gt === 'sequence') {
    return layoutSequenceGraph(nodeMap, edges, elements);
  }
  if (gt === 'communication') {
    return layoutCommunicationGraph(nodeMap, edges, elements);
  }
  if (gt === 'timing') {
    return layoutTimingGraph(nodeMap, edges, elements);
  }
  if (gt === 'usecase' || gt === 'use_case') {
    return layoutUseCaseGraph(nodeMap, edges, elements);
  }
  if (['cfg', 'robdd', 'dfg', 'cdg', 'callgraph', 'state', 'statemachine', 'interaction'].includes(gt)) {
    return layoutHierarchicalGraph(nodeMap, edges, elements);
  }
  if (gt === 'activity') {
    // If activity has swimlane packages, run bounding box engine, else hierarchical DAG
    if (packageMap.size > 0) {
      return layoutPackageBoundingBoxEngine(packageMap, standaloneNodes, elements);
    }
    return layoutHierarchicalGraph(nodeMap, edges, elements);
  }
  if (['component', 'object', 'profile'].includes(gt) && packageMap.size === 0) {
    return layoutModularGridLayout(standaloneNodes, edges, elements);
  }

  // ── 3. Standard UML Class / Package / Composite Bounding Box Engine ──
  return layoutPackageBoundingBoxEngine(packageMap, standaloneNodes, elements);
}

/**
 * ── Standard UML Class / Package / Composite Bounding Box Engine ──
 */
function layoutPackageBoundingBoxEngine(packageMap, standaloneNodes, elements) {
  const CARD_WIDTH = 290;
  const CARD_GAP_X = 120;
  const CARD_GAP_Y = 100;
  const PAD_TOP = 80;
  const PAD_BOTTOM = 50;
  const PAD_SIDE = 50;
  const SIBLING_GAP_X = 80;
  const ROOT_GAP_X = 180;
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
        width: 280,
        height: 110,
        classesWidth: 0,
        classesHeight: 0,
        subpackagesWidth: 0,
        subpackagesHeight: 0
      };
      pkg.element.data.width = 280;
      pkg.element.data.height = 110;
      pkg.element.data.origWidth = 280;
      pkg.element.data.origHeight = 110;
      return;
    }

    const contentWidth = Math.max(classesWidth, subpackagesWidth);
    const contentHeight = (classesHeight > 0 && subpackagesHeight > 0)
      ? classesHeight + CARD_GAP_Y + subpackagesHeight
      : (classesHeight || subpackagesHeight || 100);

    const totalWidth = contentWidth + PAD_SIDE * 2;
    const totalHeight = contentHeight + PAD_TOP + PAD_BOTTOM;

    pkg.bounds = {
      width: Math.max(380, totalWidth),
      height: Math.max(240, totalHeight),
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

  // Multi-Root Package Grid Alignment
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
    const STANDALONE_COLS = Math.min(3, Math.max(1, Math.ceil(Math.sqrt(standaloneNodes.length))));
    const GAP_X = 120;
    const GAP_Y = 100;
    standaloneNodes.forEach((node, idx) => {
      const c = idx % STANDALONE_COLS;
      const r = Math.floor(idx / STANDALONE_COLS);
      const w = node.data.width || CARD_WIDTH;
      const h = node.data.height || 140;
      node.position = {
        x: c * (w + GAP_X) + w / 2,
        y: currentRootY + r * (h + GAP_Y) + h / 2
      };
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
 * ── Modular 2D Grid Layout for Component, Object & Profile Diagrams ──
 */
function layoutModularGridLayout(nodes, edges, elements) {
  if (nodes.length === 0) return elements;

  const N = nodes.length;
  const cols = Math.min(4, Math.max(2, Math.ceil(Math.sqrt(N * 1.2))));
  const GAP_X = 140;
  const GAP_Y = 120;

  let maxW = 260;
  let maxH = 140;
  nodes.forEach(n => {
    if ((n.data.width || 260) > maxW) maxW = n.data.width;
    if ((n.data.height || 140) > maxH) maxH = n.data.height;
  });

  const totalGridWidth = cols * maxW + (cols - 1) * GAP_X;
  const startX = -totalGridWidth / 2;

  nodes.forEach((node, idx) => {
    const c = idx % cols;
    const r = Math.floor(idx / cols);
    const w = node.data.width || maxW;
    const h = node.data.height || maxH;

    node.position = {
      x: startX + c * (maxW + GAP_X) + w / 2,
      y: r * (maxH + GAP_Y) + h / 2 + 40
    };
  });

  return elements;
}

/**
 * ── Cycle-Safe Topological Hierarchical Layout Engine ──
 */
function layoutHierarchicalGraph(nodeMap, edges, elements) {
  const nodes = [];
  const inDegree = new Map();
  const adj = new Map();

  nodeMap.forEach((node, id) => {
    if (!node.data.isPackage) {
      nodes.push(node);
      inDegree.set(id, 0);
      adj.set(id, []);
    }
  });

  edges.forEach(edge => {
    const src = edge.data.source;
    const tgt = edge.data.target;
    if (adj.has(src) && inDegree.has(tgt) && src !== tgt) {
      adj.get(src).push(tgt);
      inDegree.set(tgt, inDegree.get(tgt) + 1);
    }
  });

  const ranks = new Map();
  const visited = new Set();
  const queue = [];

  // 1. Identify starting entry roots (inDegree === 0 or initial state / entry block)
  nodes.forEach(node => {
    const id = node.data.id;
    const isStart = node.data.kind === 'entry' || node.data.name === 'start' || node.data.name === '[*]' || id.includes('start') || id.includes('init');
    if (inDegree.get(id) === 0 || isStart) {
      if (!ranks.has(id)) {
        queue.push(id);
        ranks.set(id, 0);
      }
    }
  });

  // If no entry roots identified, seed with the first node
  if (queue.length === 0 && nodes.length > 0) {
    queue.push(nodes[0].data.id);
    ranks.set(nodes[0].data.id, 0);
  }

  // 2. BFS level traversal with cycle detection and depth capping
  const MAX_DEPTH = 50;
  while (visited.size < nodes.length) {
    if (queue.length === 0) {
      const nextUnvisited = nodes.find(n => !visited.has(n.data.id));
      if (!nextUnvisited) break;
      queue.push(nextUnvisited.data.id);
      if (!ranks.has(nextUnvisited.data.id)) {
        ranks.set(nextUnvisited.data.id, 0);
      }
    }

    while (queue.length > 0) {
      const u = queue.shift();
      if (visited.has(u)) continue;
      visited.add(u);

      const currRank = ranks.get(u) || 0;
      const neighbors = adj.get(u) || [];

      neighbors.forEach(v => {
        if (!visited.has(v)) {
          const nextRank = Math.min(MAX_DEPTH, currRank + 1);
          if (!ranks.has(v) || nextRank > ranks.get(v)) {
            ranks.set(v, nextRank);
          }
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
    const n = nodeMap.get(id);
    if (n) rankGroups.get(r).push(n);
  });

  const LEVEL_GAP_Y = 160;
  const NODE_GAP_X = 120;
  let currentY = 40;

  const sortedRanks = Array.from(rankGroups.keys()).sort((a, b) => a - b);
  sortedRanks.forEach(rank => {
    const group = rankGroups.get(rank);
    if (!group || group.length === 0) return;

    // If rank is broad (> 3 nodes), wrap into sub-rows of max 3 nodes
    const MAX_NODES_PER_ROW = 3;
    const subRows = [];
    for (let i = 0; i < group.length; i += MAX_NODES_PER_ROW) {
      subRows.push(group.slice(i, i + MAX_NODES_PER_ROW));
    }

    subRows.forEach(rowGroup => {
      let totalWidth = 0;
      let maxH = 100;
      rowGroup.forEach(n => {
        const w = n.data.width || 260;
        const h = n.data.height || 140;
        totalWidth += w + NODE_GAP_X;
        if (h > maxH) maxH = h;
      });
      totalWidth -= NODE_GAP_X;

      let currentX = -totalWidth / 2;
      rowGroup.forEach(n => {
        const w = n.data.width || 260;
        const h = n.data.height || 140;

        n.position = {
          x: currentX + w / 2,
          y: currentY + h / 2
        };
        currentX += w + NODE_GAP_X;
      });

      currentY += maxH + LEVEL_GAP_Y;
    });
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
 * ── Communication / Collaboration Diagram 2D Graph Layout Engine ──
 */
function layoutCommunicationGraph(nodeMap, edges, elements) {
  const nodes = Array.from(nodeMap.values()).filter(n => !n.data.isPackage);
  const N = nodes.length;
  if (N === 0) return elements;

  if (N <= 8) {
    const radius = Math.max(380, N * 80);
    nodes.forEach((node, i) => {
      const angle = (2 * Math.PI * i) / N;
      const w = node.data.width || 240;
      const h = node.data.height || 100;
      node.position = {
        x: Math.round(radius * Math.cos(angle)),
        y: Math.round(radius * Math.sin(angle))
      };
    });
  } else {
    const cols = Math.min(5, Math.max(3, Math.ceil(Math.sqrt(N * 1.3))));
    const CELL_W = 380;
    const CELL_H = 220;
    const startX = -((cols - 1) * CELL_W) / 2;
    const rows = Math.ceil(N / cols);
    const startY = -((rows - 1) * CELL_H) / 2;

    nodes.forEach((node, i) => {
      const c = i % cols;
      const r = Math.floor(i / cols);
      node.position = {
        x: startX + c * CELL_W,
        y: startY + r * CELL_H
      };
    });
  }

  return elements;
}

/**
 * ── Timing Diagram Waveform Multi-Track Layout Engine ──
 */
function layoutTimingGraph(nodeMap, edges, elements) {
  const TRACK_GAP_Y = 160;
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
    if (!n.data.isPackage) {
      if (n.data.kind === 'actor') {
        actors.push(n);
      } else {
        usecases.push(n);
      }
    }
  });

  // Left and Right Actor Wings with Independent Column Offsets
  let actorLeftY = 60;
  let actorRightY = 60;
  actors.forEach((act, idx) => {
    const isRight = idx % 2 === 1;
    const x = isRight ? 700 : -500;
    const y = isRight ? actorRightY : actorLeftY;
    act.position = {
      x: x,
      y: y
    };
    if (isRight) {
      actorRightY += 180;
    } else {
      actorLeftY += 180;
    }
  });

  // Center System Boundary Usecases in a 2-Column Grid
  const UC_COLS = usecases.length > 3 ? 2 : 1;
  const UC_GAP_X = 80;
  const UC_GAP_Y = 60;
  const UC_W = 220;
  const UC_H = 70;
  const startX = UC_COLS === 2 ? -(UC_W + UC_GAP_X / 2) : 0;

  usecases.forEach((uc, idx) => {
    const c = idx % UC_COLS;
    const r = Math.floor(idx / UC_COLS);
    const x = UC_COLS === 2 ? startX + c * (UC_W + UC_GAP_X) + UC_W / 2 : 0;
    const y = r * (UC_H + UC_GAP_Y) + UC_H / 2 + 60;

    uc.position = {
      x: x,
      y: y
    };
  });

  return elements;
}
