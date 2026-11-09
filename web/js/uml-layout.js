/**
 * OpenHeart Deterministic Layout Engine
 *  - Package diagrams: recursive bottom-up bounding boxes with balanced (square-ish) card grids
 *    and shelf-packed sub-packages, so large packages do not become one tall column.
 *  - Graph diagrams (CFG, call graph, state, ...): layered (Sugiyama-style) layout with DFS cycle
 *    breaking, longest-path layering, barycenter crossing reduction and centred coordinates.
 * Overlap and crossing counts are not measured by this module; it makes no guarantee beyond the
 * construction itself (cards in a grid cell or layer slot never share space by construction).
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
  const MAX_CARD_COLUMNS = 6;
  const CARD_WIDTH = 290;
  const CARD_GAP_X = 120;
  const CARD_GAP_Y = 100;
  const PAD_TOP = 80;
  const PAD_BOTTOM = 50;
  const PAD_SIDE = 50;
  const SIBLING_GAP_X = 80;
  const ROOT_GAP_X = 180;
  const ROOT_GAP_Y = 180;

  function shelfRows(items, targetWidth, gap) {
    const rows = [];
    let row = [];
    let rowWidth = 0;
    items.forEach(item => {
      const w = item.bounds.width;
      if (row.length > 0 && rowWidth + gap + w > targetWidth) {
        rows.push(row);
        row = [];
        rowWidth = 0;
      }
      rowWidth += (row.length > 0 ? gap : 0) + w;
      row.push(item);
    });
    if (row.length) rows.push(row);
    return rows;
  }

  function computePackageBounds(pkg) {
    pkg.childPackages.forEach(cp => computePackageBounds(cp));

    let classesWidth = 0;
    let classesHeight = 0;

    if (pkg.childClasses.length > 0) {
      // Balanced grid: roughly square for many cards, capped so cards stay readable side by side.
      const colCount = Math.min(MAX_CARD_COLUMNS, Math.max(1, Math.ceil(Math.sqrt(pkg.childClasses.length * 1.5))));
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
    pkg.subRows = [];

    if (pkg.childPackages.length > 0) {
      const area = pkg.childPackages.reduce((sum, cp) => sum + cp.bounds.width * cp.bounds.height, 0);
      const widest = pkg.childPackages.reduce((m, cp) => Math.max(m, cp.bounds.width), 0);
      const target = Math.max(widest, classesWidth, Math.sqrt(area) * 1.4);
      pkg.subRows = shelfRows(pkg.childPackages, target, SIBLING_GAP_X);
      pkg.subRows.forEach((row, idx) => {
        const rowWidth = row.reduce((sum, cp) => sum + cp.bounds.width, 0) + SIBLING_GAP_X * (row.length - 1);
        const rowHeight = row.reduce((m, cp) => Math.max(m, cp.bounds.height), 0);
        subpackagesWidth = Math.max(subpackagesWidth, rowWidth);
        subpackagesHeight += rowHeight + (idx > 0 ? CARD_GAP_Y : 0);
      });
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
      (pkg.subRows || [pkg.childPackages]).forEach(row => {
        let currentSubX = originX + PAD_SIDE;
        let rowHeight = 0;
        row.forEach(cp => {
          positionPackage(cp, currentSubX, currentY);
          currentSubX += cp.bounds.width + SIBLING_GAP_X;
          rowHeight = Math.max(rowHeight, cp.bounds.height);
        });
        currentY += rowHeight + CARD_GAP_Y;
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

  // Root packages: shelf-packed rows targeting a roughly square overall footprint.
  const rootArea = rootPackages.reduce((sum, rp) => sum + rp.bounds.width * rp.bounds.height, 0);
  const widestRoot = rootPackages.reduce((m, rp) => Math.max(m, rp.bounds.width), 0);
  const rootRows = shelfRows(rootPackages, Math.max(widestRoot, Math.sqrt(rootArea) * 1.4), ROOT_GAP_X);
  let rowY = 0;
  rootRows.forEach(row => {
    let rowX = 0;
    let rowHeight = 0;
    row.forEach(rp => {
      positionPackage(rp, rowX, rowY);
      rowX += rp.bounds.width + ROOT_GAP_X;
      rowHeight = Math.max(rowHeight, rp.bounds.height);
    });
    rowY += rowHeight + ROOT_GAP_Y;
  });
  let currentRootY = rowY;

  if (standaloneNodes.length > 0) {
    const STANDALONE_COLS = Math.min(8, Math.max(1, Math.ceil(Math.sqrt(standaloneNodes.length * 1.5))));
    const GAP_X = 120;
    const GAP_Y = 100;
    // Uniform cells sized to the largest card keep differently sized cards from overlapping.
    const cellW = standaloneNodes.reduce((m, n) => Math.max(m, n.data.width || CARD_WIDTH), CARD_WIDTH);
    const rowHeights = [];
    standaloneNodes.forEach((node, idx) => {
      const r = Math.floor(idx / STANDALONE_COLS);
      rowHeights[r] = Math.max(rowHeights[r] || 0, node.data.height || 140);
    });
    const rowOffsets = [];
    let acc = 0;
    rowHeights.forEach((h, r) => {
      rowOffsets[r] = acc;
      acc += h + GAP_Y;
    });
    standaloneNodes.forEach((node, idx) => {
      const c = idx % STANDALONE_COLS;
      const r = Math.floor(idx / STANDALONE_COLS);
      const h = node.data.height || 140;
      node.position = {
        x: c * (cellW + GAP_X) + cellW / 2,
        y: currentRootY + rowOffsets[r] + h / 2
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
 * ── Layered (Sugiyama-style) Layout for directed graphs ──
 * 1. Cycle breaking: iterative DFS from entry-like nodes; back edges are reversed.
 * 2. Layering: longest path over the resulting DAG (Kahn's algorithm, linear queue).
 * 3. Crossing reduction: alternating down/up barycenter sweeps.
 * 4. Coordinates: each layer centred; very wide layers wrap into sub-rows.
 * Isolated nodes are placed in a grid below the layered drawing. All steps are O(V + E)
 * except the sweeps, which are O(iterations · E log V).
 */
function layoutHierarchicalGraph(nodeMap, edges, elements) {
  const nodes = [];
  nodeMap.forEach(node => {
    if (!node.data.isPackage) nodes.push(node);
  });
  const n = nodes.length;
  if (n === 0) return elements;

  const index = new Map();
  nodes.forEach((node, i) => index.set(node.data.id, i));
  const out = Array.from({ length: n }, () => []);
  const degree = new Int32Array(n);
  edges.forEach(edge => {
    const s = index.get(edge.data.source);
    const t = index.get(edge.data.target);
    if (s === undefined || t === undefined || s === t) return;
    out[s].push(t);
    degree[s]++;
    degree[t]++;
  });

  const isEntryLike = (node) => {
    const d = node.data;
    return d.kind === 'entry' || d.name === 'start' || d.name === '[*]' || /(^|[^a-z])(entry|start|init)([^a-z]|$)/i.test(String(d.id));
  };
  const indegreeRaw = new Int32Array(n);
  out.forEach(list => list.forEach(t => { indegreeRaw[t]++; }));
  const rootOrder = [];
  nodes.forEach((node, i) => { if (isEntryLike(node)) rootOrder.push(i); });
  nodes.forEach((node, i) => { if (!isEntryLike(node) && indegreeRaw[i] === 0) rootOrder.push(i); });
  nodes.forEach((node, i) => { if (!isEntryLike(node) && indegreeRaw[i] !== 0) rootOrder.push(i); });

  // 1. Cycle breaking
  const state = new Uint8Array(n); // 0 = new, 1 = on stack, 2 = finished
  const dagOut = Array.from({ length: n }, () => []);
  for (const root of rootOrder) {
    if (state[root] !== 0) continue;
    const stack = [[root, 0]];
    state[root] = 1;
    while (stack.length) {
      const frame = stack[stack.length - 1];
      const u = frame[0];
      if (frame[1] < out[u].length) {
        const v = out[u][frame[1]++];
        if (state[v] === 1) {
          dagOut[v].push(u); // back edge reversed
        } else {
          dagOut[u].push(v);
          if (state[v] === 0) {
            state[v] = 1;
            stack.push([v, 0]);
          }
        }
      } else {
        state[u] = 2;
        stack.pop();
      }
    }
  }

  // 2. Longest-path layering
  const dagIn = Array.from({ length: n }, () => []);
  const indeg = new Int32Array(n);
  dagOut.forEach((list, u) => list.forEach(v => { dagIn[v].push(u); indeg[v]++; }));
  const rank = new Int32Array(n);
  const queue = [];
  for (let i = 0; i < n; i++) if (indeg[i] === 0) queue.push(i);
  for (let head = 0; head < queue.length; head++) {
    const u = queue[head];
    for (const v of dagOut[u]) {
      if (rank[u] + 1 > rank[v]) rank[v] = rank[u] + 1;
      if (--indeg[v] === 0) queue.push(v);
    }
  }

  // 3. Layers and crossing reduction (isolated nodes handled separately)
  const isolated = [];
  const layers = [];
  queue.forEach(i => {
    if (degree[i] === 0) {
      isolated.push(i);
      return;
    }
    const r = rank[i];
    if (!layers[r]) layers[r] = [];
    layers[r].push(i);
  });
  const denseLayers = layers.filter(Boolean);
  const pos = new Float64Array(n);
  denseLayers.forEach(layer => layer.forEach((v, k) => { pos[v] = k; }));
  const sweep = (layerList, neighbours) => {
    layerList.forEach(layer => {
      const keyed = layer.map((v, k) => {
        const adj = neighbours[v];
        if (!adj.length) return { v, key: pos[v], k };
        let sum = 0;
        for (const u of adj) sum += pos[u];
        return { v, key: sum / adj.length, k };
      });
      keyed.sort((a, b) => a.key - b.key || a.k - b.k);
      keyed.forEach((item, k) => {
        layer[k] = item.v;
        pos[item.v] = k;
      });
    });
  };
  for (let iter = 0; iter < 4; iter++) {
    sweep(denseLayers.slice(1), dagIn);
    sweep(denseLayers.slice(0, -1).reverse(), dagOut);
  }

  // 4. Coordinates
  const LEVEL_GAP_Y = 150;
  const NODE_GAP_X = 110;
  const MAX_NODES_PER_ROW = 14;
  let currentY = 40;
  denseLayers.forEach(layer => {
    for (let start = 0; start < layer.length; start += MAX_NODES_PER_ROW) {
      const row = layer.slice(start, start + MAX_NODES_PER_ROW);
      let totalWidth = 0;
      let maxH = 80;
      row.forEach(v => {
        totalWidth += (nodes[v].data.width || 260) + NODE_GAP_X;
        maxH = Math.max(maxH, nodes[v].data.height || 140);
      });
      totalWidth -= NODE_GAP_X;
      let currentX = -totalWidth / 2;
      row.forEach(v => {
        const w = nodes[v].data.width || 260;
        const h = nodes[v].data.height || 140;
        nodes[v].position = { x: currentX + w / 2, y: currentY + h / 2 };
        currentX += w + NODE_GAP_X;
      });
      currentY += maxH + LEVEL_GAP_Y;
    }
  });

  if (isolated.length) {
    const cols = Math.max(1, Math.min(10, Math.ceil(Math.sqrt(isolated.length * 1.5))));
    const cellW = isolated.reduce((m, v) => Math.max(m, nodes[v].data.width || 260), 0) + NODE_GAP_X;
    const cellH = isolated.reduce((m, v) => Math.max(m, nodes[v].data.height || 140), 0) + 80;
    const startX = -((cols * cellW) - NODE_GAP_X) / 2;
    isolated.forEach((v, k) => {
      const c = k % cols;
      const r = Math.floor(k / cols);
      nodes[v].position = { x: startX + c * cellW + (cellW - NODE_GAP_X) / 2, y: currentY + r * cellH + (nodes[v].data.height || 140) / 2 };
    });
  }

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
