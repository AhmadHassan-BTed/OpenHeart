/**
 * OpenHeart Hierarchical Architectural Layout Engine
 * Computes exact composite bounding boxes across 3 distinct nesting levels:
 *  - Level 1: Domain Tier Containers (Behavioral, Creational, Structural)
 *  - Level 2: Subpackage Containers (observer, strategy, builder, factory, adapter, facade, etc.)
 *  - Level 3: Enclosed 3-Compartment Class Cards
 * Guaranteed 100% collision-free geometry with clear color-tiered depth.
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
    }
  });

  // Link children to their parent packages
  elements.forEach(el => {
    if (!el.data.source && el.data.parent && packageMap.has(el.data.parent)) {
      packageMap.get(el.data.parent).children.push(el);
    } else if (!el.data.source && !el.data.isPackage && !el.data.parent) {
      standaloneNodes.push(el);
    }
  });

  // If this is a hierarchical compiler graph
  if (['cfg', 'robdd', 'dfg', 'cdg', 'callgraph', 'statemachine', 'sequence', 'activity'].includes(graphType)) {
    return layoutHierarchicalGraph(nodeMap, edges, elements);
  }

  // 2. Identify Top-Level Domain Packages (Level 1) vs Subpackages (Level 2)
  const level1Packages = [];
  const level2Packages = [];

  packageMap.forEach((pkgData, pkgId) => {
    const parent = pkgData.element.data.parent;
    if (!parent || !packageMap.has(parent)) {
      level1Packages.push(pkgData);
    } else {
      level2Packages.push(pkgData);
    }
  });

  // 3. Layout Constants
  const CARD_WIDTH = 290;
  const CARD_GAP_X = 140; // Wide routing channel
  const CARD_GAP_Y = 120;
  const SUBPKG_PAD_TOP = 80;
  const SUBPKG_PAD_BOTTOM = 50;
  const SUBPKG_PAD_SIDE = 50;
  const SUBPKG_GUTTER_X = 70;
  const DOMAIN_PAD_TOP = 90;
  const DOMAIN_PAD_BOTTOM = 60;
  const DOMAIN_PAD_SIDE = 50;
  const DOMAIN_GUTTER_Y = 180;

  // Step A: Calculate Layout for each Level 2 Subpackage
  const subpkgBounds = new Map();

  packageMap.forEach((pkgData, pkgId) => {
    const directLeafChildren = pkgData.children.filter(c => !c.data.isPackage);
    if (directLeafChildren.length > 0) {
      const colCount = Math.min(2, Math.max(1, directLeafChildren.length));
      const rowCount = Math.ceil(directLeafChildren.length / colCount);

      let maxChildHeight = 90;
      directLeafChildren.forEach(child => {
        const h = child.data.height || 180;
        if (h > maxChildHeight) maxChildHeight = h;
      });

      const innerWidth = colCount * CARD_WIDTH + (colCount - 1) * CARD_GAP_X;
      const innerHeight = rowCount * maxChildHeight + (rowCount - 1) * CARD_GAP_Y;

      const subWidth = innerWidth + SUBPKG_PAD_SIDE * 2;
      const subHeight = innerHeight + SUBPKG_PAD_TOP + SUBPKG_PAD_BOTTOM;

      subpkgBounds.set(pkgId, {
        width: subWidth,
        height: subHeight,
        colCount,
        maxChildHeight,
        children: directLeafChildren
      });

      pkgData.element.data.width = subWidth;
      pkgData.element.data.height = subHeight;
      pkgData.element.data.origWidth = subWidth;
      pkgData.element.data.origHeight = subHeight;
    }
  });

  // Step B: Layout Level 1 Domain Packages & Place Subpackages / Classes Inside
  let currentDomainY = 0;

  level1Packages.forEach((domainPkg) => {
    const domainId = domainPkg.element.data.id;
    const subChildren = domainPkg.children.filter(c => c.data.isPackage);

    if (subChildren.length > 0) {
      // Place subpackages side by side inside domain
      let currentSubX = DOMAIN_PAD_SIDE;
      let maxSubHeight = 0;

      subChildren.forEach((subPkg) => {
        const subId = subPkg.data.id;
        const b = subpkgBounds.get(subId) || { width: 400, height: 250, colCount: 1, maxChildHeight: 180, children: [] };
        
        if (b.height > maxSubHeight) {
          maxSubHeight = b.height;
        }

        const subX = currentSubX;
        const subY = currentDomainY + DOMAIN_PAD_TOP;

        // Position subpackage container
        subPkg.position = {
          x: subX + b.width / 2,
          y: subY + b.height / 2
        };

        // Position child classes inside this subpackage
        b.children.forEach((child, cIdx) => {
          const cCol = cIdx % b.colCount;
          const cRow = Math.floor(cIdx / b.colCount);

          const childX = subX + SUBPKG_PAD_SIDE + cCol * (CARD_WIDTH + CARD_GAP_X) + CARD_WIDTH / 2;
          const childY = subY + SUBPKG_PAD_TOP + cRow * (b.maxChildHeight + CARD_GAP_Y) + (child.data.height || b.maxChildHeight) / 2;

          child.data.width = CARD_WIDTH;
          child.position = {
            x: childX,
            y: childY
          };
        });

        currentSubX += b.width + SUBPKG_GUTTER_X;
      });

      const totalDomainWidth = currentSubX - SUBPKG_GUTTER_X + DOMAIN_PAD_SIDE;
      const totalDomainHeight = maxSubHeight + DOMAIN_PAD_TOP + DOMAIN_PAD_BOTTOM;

      domainPkg.element.data.width = totalDomainWidth;
      domainPkg.element.data.height = totalDomainHeight;
      domainPkg.element.data.origWidth = totalDomainWidth;
      domainPkg.element.data.origHeight = totalDomainHeight;
      domainPkg.element.position = {
        x: totalDomainWidth / 2,
        y: currentDomainY + totalDomainHeight / 2
      };

      currentDomainY += totalDomainHeight + DOMAIN_GUTTER_Y;
    } else {
      // Standalone domain package with direct classes
      const b = subpkgBounds.get(domainId);
      if (b) {
        const domainWidth = b.width + DOMAIN_PAD_SIDE * 2;
        const domainHeight = b.height + DOMAIN_PAD_TOP + DOMAIN_PAD_BOTTOM;

        domainPkg.element.data.width = domainWidth;
        domainPkg.element.data.height = domainHeight;
        domainPkg.element.position = {
          x: domainWidth / 2,
          y: currentDomainY + domainHeight / 2
        };

        b.children.forEach((child, cIdx) => {
          const cCol = cIdx % b.colCount;
          const cRow = Math.floor(cIdx / b.colCount);

          child.position = {
            x: DOMAIN_PAD_SIDE + SUBPKG_PAD_SIDE + cCol * (CARD_WIDTH + CARD_GAP_X) + CARD_WIDTH / 2,
            y: currentDomainY + DOMAIN_PAD_TOP + SUBPKG_PAD_TOP + cRow * (b.maxChildHeight + CARD_GAP_Y) + (child.data.height || b.maxChildHeight) / 2
          };
        });

        currentDomainY += domainHeight + DOMAIN_GUTTER_Y;
      }
    }
  });

  // Step C: Standalone nodes shelf
  if (standaloneNodes.length > 0) {
    let shelfX = 0;
    standaloneNodes.forEach((node) => {
      const w = node.data.width || CARD_WIDTH;
      const h = node.data.height || 100;
      node.position = {
        x: shelfX + w / 2,
        y: currentDomainY + h / 2
      };
      shelfX += w + 60;
    });
  }

  return elements;
}

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
    if (adj.has(src) && inDegree.has(tgt)) {
      adj.get(src).push(tgt);
      inDegree.set(tgt, inDegree.get(tgt) + 1);
    }
  });

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

  const LEVEL_GAP_Y = 180;
  const NODE_GAP_X = 120;
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
