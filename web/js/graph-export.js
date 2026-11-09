/**
 * OpenHeart Interchange Exporters for the current diagram view.
 * Graphviz DOT and GraphML (yEd, Gephi, Neo4j import, NetworkX) from a plain snapshot
 * produced by InteractiveGraphCanvas.getGraphSnapshot(). Package containment is emitted as
 * DOT clusters and GraphML nested graphs.
 */

function dotId(value) {
  return `"${String(value).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`;
}

const DOT_EDGE_STYLE = {
  generalization: 'arrowhead=empty',
  realization: 'arrowhead=empty, style=dashed',
  composition: 'arrowtail=diamond, dir=back',
  aggregation: 'arrowtail=odiamond, dir=back',
  dependency: 'style=dashed, arrowhead=vee',
  association: 'arrowhead=vee'
};

export function toDot(snapshot) {
  const lines = [];
  lines.push(`digraph ${dotId(`${snapshot.codebaseName} · ${snapshot.diagramType}`)} {`);
  lines.push('  graph [rankdir=TB, compound=true, fontname="Helvetica"];');
  lines.push('  node [shape=box, style="rounded", fontname="Helvetica", fontsize=10];');
  lines.push('  edge [fontname="Helvetica", fontsize=9];');

  const children = new Map();
  const byId = new Map(snapshot.nodes.map(n => [n.id, n]));
  snapshot.nodes.forEach(n => {
    const parent = n.parent && byId.has(n.parent) ? n.parent : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(n);
  });
  let clusterSeq = 0;
  const emit = (parentId, indent) => {
    (children.get(parentId) || []).forEach(n => {
      const hasChildren = children.has(n.id);
      if (hasChildren) {
        lines.push(`${indent}subgraph cluster_${clusterSeq++} {`);
        lines.push(`${indent}  label=${dotId(n.name)};`);
        lines.push(`${indent}  style="rounded,dashed";`);
        emit(n.id, `${indent}  `);
        lines.push(`${indent}}`);
      } else {
        const attrs = [`label=${dotId(n.stereotype ? `${n.stereotype}\n${n.name}` : n.name)}`];
        if (n.isPackage) attrs.push('shape=folder');
        if (n.file) attrs.push(`tooltip=${dotId(n.line ? `${n.file}:${n.line}` : n.file)}`);
        lines.push(`${indent}${dotId(n.id)} [${attrs.join(', ')}];`);
      }
    });
  };
  emit(null, '  ');

  const clusterIds = new Set(Array.from(children.keys()).filter(Boolean));
  snapshot.edges.forEach(e => {
    // Edges attached to a cluster (compound) node cannot be drawn by Graphviz; skip them explicitly.
    if (clusterIds.has(e.source) || clusterIds.has(e.target)) return;
    const style = DOT_EDGE_STYLE[e.kind] || 'arrowhead=vee';
    const label = e.label ? `, label=${dotId(e.label)}` : '';
    lines.push(`  ${dotId(e.source)} -> ${dotId(e.target)} [${style}${label}, tooltip=${dotId(e.kind)}];`);
  });
  lines.push('}');
  return lines.join('\n') + '\n';
}

function xmlEscape(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
    // Characters not allowed in XML 1.0
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, '');
}

export function toGraphMl(snapshot) {
  const out = [];
  out.push('<?xml version="1.0" encoding="UTF-8"?>');
  out.push('<graphml xmlns="http://graphml.graphdrawing.org/xmlns" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd">');
  const keys = [
    ['d_name', 'node', 'name', 'string'],
    ['d_kind', 'node', 'kind', 'string'],
    ['d_stereotype', 'node', 'stereotype', 'string'],
    ['d_qualified', 'node', 'qualified_name', 'string'],
    ['d_file', 'node', 'file', 'string'],
    ['d_line', 'node', 'line', 'int'],
    ['d_ekind', 'edge', 'relation', 'string'],
    ['d_elabel', 'edge', 'label', 'string']
  ];
  keys.forEach(([id, forWhat, name, type]) => {
    out.push(`  <key id="${id}" for="${forWhat}" attr.name="${name}" attr.type="${type}"/>`);
  });
  out.push(`  <graph id="G" edgedefault="directed">`);
  out.push(`    <desc>${xmlEscape(`${snapshot.codebaseName} · ${snapshot.diagramType} (source: ${snapshot.sourceKind})`)}</desc>`);

  const byId = new Map(snapshot.nodes.map(n => [n.id, n]));
  const children = new Map();
  snapshot.nodes.forEach(n => {
    const parent = n.parent && byId.has(n.parent) ? n.parent : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(n);
  });
  const emitNode = (n, indent) => {
    out.push(`${indent}<node id="${xmlEscape(n.id)}">`);
    out.push(`${indent}  <data key="d_name">${xmlEscape(n.name)}</data>`);
    out.push(`${indent}  <data key="d_kind">${xmlEscape(n.kind)}</data>`);
    if (n.stereotype) out.push(`${indent}  <data key="d_stereotype">${xmlEscape(n.stereotype)}</data>`);
    if (n.qualifiedName) out.push(`${indent}  <data key="d_qualified">${xmlEscape(n.qualifiedName)}</data>`);
    if (n.file) out.push(`${indent}  <data key="d_file">${xmlEscape(n.file)}</data>`);
    if (Number.isInteger(n.line)) out.push(`${indent}  <data key="d_line">${n.line}</data>`);
    const kids = children.get(n.id);
    if (kids && kids.length) {
      out.push(`${indent}  <graph id="${xmlEscape(n.id)}::" edgedefault="directed">`);
      kids.forEach(k => emitNode(k, `${indent}    `));
      out.push(`${indent}  </graph>`);
    }
    out.push(`${indent}</node>`);
  };
  (children.get(null) || []).forEach(n => emitNode(n, '    '));
  snapshot.edges.forEach(e => {
    out.push(`    <edge id="${xmlEscape(e.id)}" source="${xmlEscape(e.source)}" target="${xmlEscape(e.target)}">`);
    out.push(`      <data key="d_ekind">${xmlEscape(e.kind)}</data>`);
    if (e.label) out.push(`      <data key="d_elabel">${xmlEscape(e.label)}</data>`);
    out.push('    </edge>');
  });
  out.push('  </graph>');
  out.push('</graphml>');
  return out.join('\n') + '\n';
}
