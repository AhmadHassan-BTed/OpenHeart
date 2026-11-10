/**
 * OpenHeart layout quality gate.
 * Runs the adaptive layout engine (web/js/layout/engine.js) with the real elkjs 0.9.3 on:
 *   - every bundled studio diagram (web/diagrams/*.json, validated as the studio does),
 *   - class and package diagrams the in-browser extractor builds from real code,
 *   - synthetic stress graphs,
 * and checks the result with its own geometry, independent of the engine's metrics:
 *   HARD  no NaN positions; no overlapping sibling boxes (package footprints include their label
 *         band, package boxes are emulated the way Cytoscape sizes compounds: children bbox +
 *         padding, enlarged to the layout box); children inside their package; route endpoints
 *         on their node borders; orthogonal routes axis-parallel; no edge through an unrelated
 *         card; no edge through an unrelated package; deterministic output.
 *   SOFT  edge crossings, bends, label collisions, mean edge length / card size, area use,
 *         aspect ratio, run time. The legacy layout is measured the same way for comparison
 *         (its taxi edges approximated as straight centre lines).
 * Usage (from scripts/layout-gate, after `npm install`):
 *   node --experimental-default-type=module run-gate.mjs [filter[,filter...]]
 * Env: OH_PROFILE=1 prints every candidate's time, score and metrics; OH_DUMP_ROUTER=<dir> writes
 * every router call's input; OH_OUT=<file> sets the results file (default: the OS temp dir).
 */
import { createRequire } from 'node:module';
import { pathToFileURL, fileURLToPath } from 'node:url';
import os from 'node:os';
import fs from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';

const REPO = process.env.OH_REPO || path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const DEPS = process.env.OH_DEPS || path.dirname(fileURLToPath(import.meta.url));
const OUT = process.env.OH_OUT || path.join(os.tmpdir(), 'openheart-layout-gate-results.json');
const FILTER = process.argv[2] || '';

const require = createRequire(pathToFileURL(path.join(DEPS, 'index.js')).href);
globalThis.ELK = require('elkjs/lib/elk.bundled.js');

const mod = p => import(pathToFileURL(path.join(REPO, p)).href);
const { computeLayout } = await mod('web/js/layout/engine.js');
const { loadGraphIrToCytoscape } = await mod('web/js/graph-loader.js');
const { validateGraphIr } = await mod('web/js/graph-ir-validate.js');
const { computeDeterministicLayout } = await mod('web/js/uml-layout.js');
const { parseSource, languageForPath } = await mod('web/js/analysis/source-parser.js');
const { buildClassModel } = await mod('web/js/analysis/model-builder.js');
const G = await mod('web/js/layout/geometry.js');

const VIEWPORT = { width: 1600, height: 900 };

// ───────────────────────── inputs ─────────────────────────

function isEdge(el) {
  return Boolean(el && el.data && el.data.source !== undefined && el.data.target !== undefined);
}

function bundledCases() {
  const manifest = JSON.parse(fs.readFileSync(path.join(REPO, 'web/diagrams/manifest.json'), 'utf8'));
  const cases = [];
  for (const cat of manifest.categories) {
    for (const d of cat.diagrams) {
      const file = path.join(REPO, 'web/diagrams', d.file_json || `${d.id}.json`);
      if (!fs.existsSync(file)) continue;
      cases.push({
        name: `bundled/${d.id}`,
        type: d.id,
        make: () => {
          const json = JSON.parse(fs.readFileSync(file, 'utf8'));
          const v = validateGraphIr(json, { expectedType: d.id });
          if (!v.ok) throw new Error(`validator rejected ${d.id}: ${v.errors.join(' ')}`);
          return loadGraphIrToCytoscape(v.ir);
        }
      });
    }
  }
  return cases;
}

function walk(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === 'target' || entry.name.startsWith('.')) continue;
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(p, out);
    else if (languageForPath(p)) out.push(p);
  }
  return out;
}

function packageProjection(ir) {
  const classToPkg = new Map();
  ir.nodes.forEach(n => { if (!n.is_package && n.parent) classToPkg.set(n.id, n.parent); });
  const pkgNodes = ir.nodes.filter(n => n.is_package).map(p => Object.assign({}, p, { parent: null }));
  const counts = new Map();
  (ir.edges || []).forEach(e => {
    const s = classToPkg.get(e.source);
    const t = classToPkg.get(e.target);
    if (s && t && s !== t) counts.set(`${s}\u0000${t}`, (counts.get(`${s}\u0000${t}`) || 0) + 1);
  });
  const pkgEdges = [...counts.entries()].map(([k, n]) => {
    const [s, t] = k.split('\u0000');
    return { id: `pkgdep:${s}->${t}`, source: s, target: t, kind: 'dependency', label: `${n} relation${n === 1 ? '' : 's'}`, arrow: '..>' };
  });
  return { diagram_type: 'package', nodes: pkgNodes, edges: pkgEdges };
}

function realCodeCases() {
  const sources = [
    ['examples-patterns', 'examples/test_patterns_codebase'],
    ['examples-enterprise', 'examples/huge_enterprise_src'],
    ['examples-big-project', 'examples/big_project_src'],
    ['sample-codebase', 'sample_codebase'],
    ['openheart-rust-src', 'src'],
    ['openheart-web-js', 'web/js']
  ];
  const cases = [];
  for (const [name, rel] of sources) {
    let ir = null;
    const getIr = () => {
      if (ir) return ir;
      const files = walk(path.join(REPO, rel));
      const parses = files.map(f => parseSource(path.relative(REPO, f).replace(/\\/g, '/'), fs.readFileSync(f, 'utf8')));
      const built = buildClassModel(parses, { title: name });
      ir = built.graph_ir || built;
      return ir;
    };
    cases.push({ name: `code/${name}/class`, type: 'class', make: () => loadGraphIrToCytoscape(JSON.parse(JSON.stringify(getIr()))) });
    cases.push({ name: `code/${name}/package`, type: 'package', make: () => loadGraphIrToCytoscape(packageProjection(JSON.parse(JSON.stringify(getIr())))) });
  }
  return cases;
}

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function members(r, maxF, maxM) {
  const f = Math.floor(r() * maxF);
  const m = Math.floor(r() * maxM);
  return {
    fields: Array.from({ length: f }, (_, i) => ({ name: `field${i}`, type: i % 2 ? 'String' : 'int', visibility: '-' })),
    methods: Array.from({ length: m }, (_, i) => ({ name: `operation${i}`, signature: `operation${i}(arg${i}: Value): Result`, visibility: '+' }))
  };
}

function syntheticClassIr(seed, { roots = 6, subPerRoot = 2, classesPerPkg = 18, isolatedPerPkg = 6, edgeCount = 140, deep = false } = {}) {
  const r = rng(seed);
  const nodes = [];
  const edges = [];
  const pkgs = [];
  for (let i = 0; i < roots; i++) {
    const id = `pkg_r${i}`;
    nodes.push({ id, name: `module${i}`, is_package: true, nest_level: 0 });
    pkgs.push(id);
    const subs = i % 2 === 0 ? subPerRoot : 0;
    for (let j = 0; j < subs; j++) {
      const sid = `${id}_s${j}`;
      nodes.push({ id: sid, name: `module${i}.part${j}`, is_package: true, parent: id, nest_level: 1 });
      pkgs.push(sid);
      if (deep && j === 0) {
        const did = `${sid}_d`;
        nodes.push({ id: did, name: `module${i}.part${j}.deep`, is_package: true, parent: sid, nest_level: 2 });
        pkgs.push(did);
      }
    }
  }
  const classes = [];
  pkgs.forEach((p, pi) => {
    for (let k = 0; k < classesPerPkg + isolatedPerPkg; k++) {
      const id = `c_${pi}_${k}`;
      const kind = k % 7 === 0 ? 'interface' : (k % 11 === 0 ? 'enum' : (k % 5 === 0 ? 'abstract' : 'class'));
      nodes.push(Object.assign({ id, name: `Type${pi}_${k}`, kind, parent: p }, members(r, 6, 9)));
      if (k < classesPerPkg) classes.push({ id, pkg: pi, kind });
    }
  });
  const kinds = ['generalization', 'realization', 'association', 'composition', 'dependency', 'aggregation'];
  const weights = [0.3, 0.15, 0.2, 0.15, 0.15, 0.05];
  const pickKind = () => {
    let x = r();
    for (let i = 0; i < kinds.length; i++) { if ((x -= weights[i]) <= 0) return kinds[i]; }
    return kinds[0];
  };
  const seen = new Set();
  for (let e = 0; edges.length < edgeCount && e < edgeCount * 10; e++) {
    const a = classes[Math.floor(r() * classes.length)];
    // Mostly local relations, some across packages.
    const local = r() < 0.7;
    const pool = local ? classes.filter(c => c.pkg === a.pkg && c.id !== a.id) : classes.filter(c => c.pkg !== a.pkg);
    if (!pool.length) continue;
    const b = pool[Math.floor(r() * pool.length)];
    let kind = pickKind();
    if (kind === 'realization' && b.kind !== 'interface') kind = 'association';
    const key = `${a.id}|${b.id}|${kind}`;
    if (seen.has(key)) continue;
    seen.add(key);
    edges.push({ id: `e${edges.length}`, source: a.id, target: b.id, kind, label: kind === 'association' ? `role${edges.length}` : '' });
  }
  return { diagram_type: 'class', nodes, edges };
}

function syntheticCfgIr(seed, blocks = 150) {
  const r = rng(seed);
  const nodes = [{ id: 'bb_entry', kind: 'bb', name: 'entry', label: 'entry', instructions: ['enter'] }];
  const edges = [];
  let last = 'bb_entry';
  let n = 0;
  const stack = [];
  const mk = () => {
    const id = `bb_${n++}`;
    nodes.push({ id, kind: 'bb', name: id, label: id, instructions: Array.from({ length: 1 + Math.floor(r() * 4) }, (_, i) => `v${i} = op(${i})`) });
    return id;
  };
  const link = (s, t, label) => edges.push({ id: `e${edges.length}`, source: s, target: t, kind: 'control_flow', label });
  while (n < blocks) {
    const x = r();
    if (x < 0.25) { // if/else diamond
      const c = mk(); link(last, c); const t = mk(); const f = mk(); const j = mk();
      link(c, t, '[true]'); link(c, f, '[false]'); link(t, j); link(f, j); last = j;
    } else if (x < 0.45) { // loop
      const h = mk(); link(last, h); const b = mk(); link(h, b, '[true]'); link(b, h, 'back'); const ex = mk(); link(h, ex, '[false]'); last = ex;
    } else if (x < 0.5 && stack.length < 3) {
      stack.push(last);
    } else {
      const s = mk(); link(last, s); last = s;
      if (stack.length && r() < 0.2) { link(stack.pop(), s, 'goto'); }
    }
  }
  nodes.push({ id: 'bb_exit', kind: 'bb', name: 'exit', label: 'exit', instructions: ['return'] });
  link(last, 'bb_exit');
  return { diagram_type: 'cfg', nodes, edges };
}

function syntheticCallgraphIr(seed, count = 120, extra = 260) {
  const r = rng(seed);
  const nodes = Array.from({ length: count }, (_, i) => ({ id: `fn${i}`, kind: 'class', name: `function_${i}`, methods: [{ name: `f${i}`, signature: `f${i}()`, visibility: '+' }] }));
  const edges = [];
  for (let i = 1; i < count; i++) edges.push({ id: `t${i}`, source: `fn${Math.floor(r() * i)}`, target: `fn${i}`, kind: 'association', label: 'call' });
  for (let k = 0; k < extra; k++) {
    const a = Math.floor(r() * count);
    const b = Math.floor(r() * count);
    if (a !== b) edges.push({ id: `x${k}`, source: `fn${a}`, target: `fn${b}`, kind: 'association', label: '' });
  }
  return { diagram_type: 'callgraph', nodes, edges };
}

function syntheticCommunicationIr(seed, count = 40, messages = 300) {
  const r = rng(seed);
  const nodes = [{ id: 'actor', kind: 'actor', name: 'User' }].concat(Array.from({ length: count }, (_, i) => ({ id: `p${i}`, kind: 'participant', name: `Service${i}` })));
  const edges = [];
  for (let k = 0; k < messages; k++) {
    const a = r() < 0.1 ? 'actor' : `p${Math.floor(Math.pow(r(), 2) * count)}`;
    const b = `p${Math.floor(r() * count)}`;
    if (a !== b) edges.push({ id: `m${k}`, source: a, target: b, kind: 'message', label: `${k + 1}: call${k}()` });
  }
  return { diagram_type: 'communication', nodes, edges };
}

function syntheticPackageDepsIr(seed, count = 30, deps = 80) {
  const r = rng(seed);
  const nodes = Array.from({ length: count }, (_, i) => ({ id: `pkg${i}`, name: `org.example.module${i}`, is_package: true, nest_level: 0 }));
  const edges = [];
  const seen = new Set();
  while (edges.length < deps) {
    const a = Math.floor(r() * count);
    const b = Math.floor(r() * count);
    if (a === b || seen.has(`${a}|${b}`)) continue;
    seen.add(`${a}|${b}`);
    edges.push({ id: `d${edges.length}`, source: `pkg${a}`, target: `pkg${b}`, kind: 'dependency', label: '«use»' });
  }
  return { diagram_type: 'package', nodes, edges };
}

function syntheticEdgeCasesIr() {
  // Edges to a package node, a nested edge (class to its own package), a self loop, a parent cycle,
  // a dangling parent, duplicate parallel edges, a root package chain to prune.
  const nodes = [
    { id: 'chain0', name: 'com', is_package: true, nest_level: 0 },
    { id: 'chain1', name: 'com.acme', is_package: true, parent: 'chain0', nest_level: 1 },
    { id: 'pA', name: 'com.acme.alpha', is_package: true, parent: 'chain1', nest_level: 2 },
    { id: 'pB', name: 'com.acme.beta', is_package: true, parent: 'chain1', nest_level: 2 },
    { id: 'a1', name: 'Alpha', kind: 'class', parent: 'pA' },
    { id: 'a2', name: 'AlphaBase', kind: 'abstract', parent: 'pA' },
    { id: 'b1', name: 'Beta', kind: 'class', parent: 'pB' },
    { id: 'b2', name: 'BetaApi', kind: 'interface', parent: 'pB' },
    { id: 'lone', name: 'Lonely', kind: 'class' },
    { id: 'dangling', name: 'Dangling', kind: 'class', parent: 'missing_pkg' },
    { id: 'pkgLeaf', name: 'com.other', is_package: true, nest_level: 0 }
  ];
  const edges = [
    { id: 'e1', source: 'a1', target: 'a2', kind: 'generalization' },
    { id: 'e2', source: 'b1', target: 'b2', kind: 'realization' },
    { id: 'e3', source: 'a1', target: 'b1', kind: 'association', label: 'uses' },
    { id: 'e3b', source: 'a1', target: 'b1', kind: 'association', label: 'also uses' },
    { id: 'e4', source: 'a1', target: 'a1', kind: 'association', label: 'self' },
    { id: 'e5', source: 'b1', target: 'pB', kind: 'dependency', label: 'nested' },
    { id: 'e6', source: 'pA', target: 'pkgLeaf', kind: 'dependency', label: 'pkg dep' },
    { id: 'e7', source: 'dangling', target: 'lone', kind: 'association' }
  ];
  return { diagram_type: 'class', nodes, edges };
}

function syntheticCases() {
  return [
    { name: 'synthetic/class-packages', type: 'class', make: () => loadGraphIrToCytoscape(syntheticClassIr(7)) },
    { name: 'synthetic/class-deep-nesting', type: 'class', make: () => loadGraphIrToCytoscape(syntheticClassIr(11, { roots: 4, subPerRoot: 3, classesPerPkg: 8, isolatedPerPkg: 3, edgeCount: 90, deep: true })) },
    { name: 'synthetic/class-isolated-only', type: 'class', make: () => loadGraphIrToCytoscape(syntheticClassIr(3, { edgeCount: 0 })) },
    { name: 'synthetic/class-edge-cases', type: 'class', make: () => loadGraphIrToCytoscape(syntheticEdgeCasesIr()) },
    { name: 'synthetic/cfg-150', type: 'cfg', make: () => loadGraphIrToCytoscape(syntheticCfgIr(5)) },
    { name: 'synthetic/callgraph-120', type: 'callgraph', make: () => loadGraphIrToCytoscape(syntheticCallgraphIr(9)) },
    { name: 'synthetic/communication-40', type: 'communication', make: () => loadGraphIrToCytoscape(syntheticCommunicationIr(13)) },
    { name: 'synthetic/package-deps-30', type: 'package', make: () => loadGraphIrToCytoscape(syntheticPackageDepsIr(17)) },
    { name: 'synthetic/class-large-1200', type: 'class', make: () => loadGraphIrToCytoscape(syntheticClassIr(21, { roots: 20, subPerRoot: 2, classesPerPkg: 30, isolatedPerPkg: 10, edgeCount: 1500 })) },
    ...perTypeStressCases()
  ];
}

/**
 * One stress input per remaining diagram type, shaped like that type's bundled sample (same node
 * kinds, nesting and edge kinds) but several times larger and denser, so every layout path in
 * the engine is exercised for every one of the 19 studio diagrams.
 */
function perTypeStressCases() {
  const ir = (diagram_type, nodes, edges) => ({ diagram_type, nodes, edges });
  const pick = (r, list) => list[Math.floor(r() * list.length)];
  const uniqueEdges = (r, count, from, to, kind, label, allowSelf = false) => {
    const out = [];
    const seen = new Set();
    for (let k = 0; out.length < count && k < count * 20; k++) {
      const a = pick(r, from);
      const b = pick(r, to);
      if ((!allowSelf && a === b) || seen.has(`${a}|${b}`)) continue;
      seen.add(`${a}|${b}`);
      out.push({ id: `${kind}_${out.length}`, source: a, target: b, kind, label: typeof label === 'function' ? label(out.length) : label });
    }
    return out;
  };
  const cases = [];

  cases.push({ name: 'stress/component', type: 'component', make: () => {
    const r = rng(101);
    const nodes = [];
    for (let p = 0; p < 6; p++) nodes.push({ id: `cp${p}`, name: `subsystem${p}`, is_package: true, nest_level: 0 });
    const comps = [];
    const ifaces = [];
    for (let i = 0; i < 40; i++) { nodes.push({ id: `comp${i}`, name: `Component${i}`, kind: 'component', parent: `cp${i % 6}` }); comps.push(`comp${i}`); }
    for (let i = 0; i < 25; i++) { nodes.push({ id: `if${i}`, name: `IService${i}`, kind: 'interface', parent: `cp${i % 6}` }); ifaces.push(`if${i}`); }
    const edges = uniqueEdges(r, 25, ifaces, comps, 'realization', 'provides').concat(uniqueEdges(r, 70, comps, ifaces, 'dependency', 'requires'));
    return loadGraphIrToCytoscape(ir('component', nodes, edges));
  } });

  cases.push({ name: 'stress/composite', type: 'composite', make: () => {
    const r = rng(102);
    const nodes = [];
    const parts = [];
    for (let c = 0; c < 4; c++) {
      nodes.push({ id: `cls${c}`, name: `Engine${c}`, kind: 'composite_classifier', is_package: true, nest_level: 0 });
      for (let p = 0; p < 8; p++) { const id = `part${c}_${p}`; nodes.push(Object.assign({ id, name: `unit${p}`, label: `unit${p}: Unit${c}${p}`, kind: 'part', parent: `cls${c}` }, members(r, 4, 3))); parts.push(id); }
    }
    return loadGraphIrToCytoscape(ir('composite', nodes, uniqueEdges(r, 45, parts, parts, 'assembly_connector', 'port')));
  } });

  cases.push({ name: 'stress/object', type: 'object', make: () => {
    const r = rng(103);
    const objs = Array.from({ length: 50 }, (_, i) => ({ id: `o${i}`, name: `obj${i}`, label: `obj${i} : Type${i % 9}`, kind: 'object', fields: Array.from({ length: Math.floor(r() * 5) }, (_, f) => `slot${f} = ${f}`) }));
    return loadGraphIrToCytoscape(ir('object', objs, uniqueEdges(r, 80, objs.map(o => o.id), objs.map(o => o.id), 'association', 'link')));
  } });

  cases.push({ name: 'stress/deployment', type: 'deployment', make: () => {
    const r = rng(104);
    const nodes = [];
    const arts = [];
    for (let d = 0; d < 8; d++) {
      nodes.push({ id: `dev${d}`, name: `server${d}`, is_package: true, nest_level: 0 });
      if (d % 3 === 0) nodes.push({ id: `env${d}`, name: `container${d}`, is_package: true, parent: `dev${d}`, nest_level: 1 });
      for (let a = 0; a < 4; a++) { const id = `art${d}_${a}`; nodes.push({ id, name: `module${d}_${a}.jar`, kind: 'artifact', parent: d % 3 === 0 && a < 2 ? `env${d}` : `dev${d}` }); arts.push(id); }
    }
    const edges = uniqueEdges(r, 30, arts, arts, 'dependency', '«deploy»').concat(arts.slice(0, 6).map((a, i) => ({ id: `man${i}`, source: a, target: a.replace(/_\d+$/, '').replace('art', 'dev'), kind: 'manifestation', label: '' })));
    return loadGraphIrToCytoscape(ir('deployment', nodes, edges));
  } });

  cases.push({ name: 'stress/profile', type: 'profile', make: () => {
    const r = rng(105);
    const metas = Array.from({ length: 10 }, (_, i) => ({ id: `meta${i}`, name: `Metaclass${i}`, kind: 'metaclass', fields: ['name: String'] }));
    const sts = Array.from({ length: 25 }, (_, i) => ({ id: `st${i}`, name: `Stereo${i}`, kind: 'stereotype', fields: Array.from({ length: 1 + (i % 3) }, (_, f) => `tag${f}: String`) }));
    const edges = sts.map((s, i) => ({ id: `ext${i}`, source: s.id, target: pick(r, metas).id, kind: 'extension', label: '' }))
      .concat(uniqueEdges(r, 10, sts.map(s => s.id), sts.map(s => s.id), 'generalization', ''));
    return loadGraphIrToCytoscape(ir('profile', metas.concat(sts), edges));
  } });

  cases.push({ name: 'stress/sequence', type: 'sequence', make: () => {
    const r = rng(106);
    const parts = [{ id: 'a0', name: 'User', kind: 'actor' }, { id: 'a1', name: 'Admin', kind: 'actor' }].concat(Array.from({ length: 25 }, (_, i) => ({ id: `p${i}`, name: `Service${i}`, kind: 'participant' })));
    const ids = parts.map(p => p.id);
    const edges = [];
    for (let k = 0; k < 400; k++) {
      const a = pick(r, ids);
      const b = r() < 0.25 ? a : pick(r, ids);
      edges.push({ id: `m${k}`, source: a, target: b, kind: 'message', label: `${k + 1}: ${r() < 0.2 ? 'computeAggregatedResult' : 'call'}${k}()` });
    }
    return loadGraphIrToCytoscape(ir('sequence', parts, edges));
  } });

  cases.push({ name: 'stress/statemachine', type: 'statemachine', make: () => {
    const r = rng(107);
    const states = [{ id: 's_init', name: '[*]', kind: 'state', stereotype: '<<initial>>' }]
      .concat(Array.from({ length: 38 }, (_, i) => ({ id: `s${i}`, name: `State${i}`, kind: 'state', instructions: i % 4 === 0 ? [`entry / enter${i}()`, `exit / leave${i}()`] : [] })))
      .concat([{ id: 's_final', name: 'final', kind: 'state', stereotype: '<<final>>' }]);
    const ids = states.slice(1, -1).map(s => s.id);
    const edges = [{ id: 't_init', source: 's_init', target: 's0', kind: 'transition', label: 'start' }]
      .concat(ids.slice(1).map((s, i) => ({ id: `tc${i}`, source: ids[Math.floor(r() * (i + 1))], target: s, kind: 'transition', label: `event${i}` })))
      .concat(uniqueEdges(r, 45, ids, ids, 'transition', i => `retry${i} [guard${i}]`))
      .concat([{ id: 't_final', source: ids[ids.length - 1], target: 's_final', kind: 'transition', label: 'done' }]);
    return loadGraphIrToCytoscape(ir('statemachine', states, edges));
  } });

  cases.push({ name: 'stress/activity', type: 'activity', make: () => {
    const r = rng(108);
    const nodes = [];
    const acts = [];
    for (let p = 0; p < 5; p++) nodes.push({ id: `lane${p}`, name: `Lane${p}`, is_package: true, nest_level: 0 });
    nodes.push({ id: 'act_start', name: 'start', kind: 'action', stereotype: '<<start>>', parent: 'lane0' });
    for (let i = 0; i < 60; i++) { const id = `act${i}`; nodes.push({ id, name: `doStep${i}`, label: `doStep${i}()`, kind: 'action', parent: `lane${Math.floor(r() * 5)}` }); acts.push(id); }
    nodes.push({ id: 'act_stop', name: 'stop', kind: 'action', stereotype: '<<stop>>', parent: 'lane4' });
    const edges = [{ id: 'f_start', source: 'act_start', target: acts[0], kind: 'control_flow' }]
      .concat(acts.slice(1).map((a, i) => ({ id: `f${i}`, source: acts[i], target: a, kind: 'control_flow', label: i % 7 === 0 ? '[ok]' : null })))
      .concat(uniqueEdges(r, 20, acts, acts, 'control_flow', i => (i % 2 ? '[retry]' : null)))
      .concat([{ id: 'f_stop', source: acts[acts.length - 1], target: 'act_stop', kind: 'control_flow' }]);
    return loadGraphIrToCytoscape(ir('activity', nodes, edges));
  } });

  cases.push({ name: 'stress/usecase', type: 'usecase', make: () => {
    const r = rng(109);
    const actors = Array.from({ length: 6 }, (_, i) => ({ id: `actor${i}`, name: `Role${i}`, kind: 'actor' }));
    const ucs = Array.from({ length: 25 }, (_, i) => ({ id: `uc${i}`, name: `Perform task ${i}`, kind: 'usecase' }));
    const edges = uniqueEdges(r, 40, actors.map(a => a.id), ucs.map(u => u.id), 'association', '')
      .concat(uniqueEdges(r, 18, ucs.map(u => u.id), ucs.map(u => u.id), 'dependency', i => (i % 2 ? '«include»' : '«extend»')));
    return loadGraphIrToCytoscape(ir('usecase', actors.concat(ucs), edges));
  } });

  cases.push({ name: 'stress/interaction', type: 'interaction', make: () => {
    const r = rng(110);
    const nodes = [{ id: 'io_start', name: 'Start', kind: 'action', stereotype: '<<initial>>' }]
      .concat(Array.from({ length: 30 }, (_, i) => ({ id: `ref${i}`, name: `ref${i}`, label: `ref: Scenario${i}`, kind: 'action', stereotype: '<<interaction_use>>', instructions: [`call${i}()`, `reply${i}()`] })))
      .concat([{ id: 'io_end', name: 'End', kind: 'action', stereotype: '<<final>>' }]);
    const ids = nodes.slice(1, -1).map(n => n.id);
    const edges = [{ id: 'c_s', source: 'io_start', target: ids[0], kind: 'control_flow' }]
      .concat(ids.slice(1).map((n, i) => ({ id: `c${i}`, source: ids[Math.floor(r() * (i + 1))], target: n, kind: 'control_flow', label: i % 5 === 0 ? '[alt]' : '' })))
      .concat(uniqueEdges(r, 12, ids, ids, 'control_flow', '[loop]'))
      .concat([{ id: 'c_e', source: ids[ids.length - 1], target: 'io_end', kind: 'control_flow' }]);
    return loadGraphIrToCytoscape(ir('interaction', nodes, edges));
  } });

  cases.push({ name: 'stress/timing', type: 'timing', make: () => {
    const r = rng(111);
    const tracks = Array.from({ length: 12 }, (_, i) => ({ id: `tr${i}`, name: `Lifeline${i}`, kind: 'timing_track', instructions: ['idle', 'busy', 'idle'] }));
    return loadGraphIrToCytoscape(ir('timing', tracks, uniqueEdges(r, 40, tracks.map(t => t.id), tracks.map(t => t.id), 'transition', i => `t${i}`)));
  } });

  cases.push({ name: 'stress/dfg', type: 'dfg', make: () => {
    const r = rng(112);
    const vars = Array.from({ length: 80 }, (_, i) => ({ id: `v${i}`, name: `v${i}`, label: `v${i}: Value${i % 7}`, kind: 'data_node' }));
    const edges = vars.slice(1).map((v, i) => ({ id: `d${i}`, source: vars[Math.floor(r() * (i + 1))].id, target: v.id, kind: 'data_flow', label: 'def-use' }))
      .concat(uniqueEdges(r, 60, vars.slice(0, 60).map(v => v.id), vars.slice(20).map(v => v.id), 'data_flow', 'use'));
    return loadGraphIrToCytoscape(ir('dfg', vars, edges));
  } });

  cases.push({ name: 'stress/cdg', type: 'cdg', make: () => {
    const r = rng(113);
    const nodes = [{ id: 'cdg_entry', name: 'CDG_Entry', kind: 'action', stereotype: '<<entry>>' }]
      .concat(Array.from({ length: 60 }, (_, i) => ({ id: `cd${i}`, name: `stmt${i}`, label: i % 4 === 0 ? `if (cond${i})` : `stmt${i}`, kind: 'action' })));
    const ids = nodes.map(n => n.id);
    const edges = ids.slice(1).map((n, i) => ({ id: `cd_e${i}`, source: ids[Math.floor(r() * Math.min(i + 1, 1 + Math.floor(i / 4) * 4))], target: n, kind: 'control_flow', label: r() < 0.5 ? 'T' : 'F' }));
    return loadGraphIrToCytoscape(ir('cdg', nodes, edges));
  } });

  cases.push({ name: 'stress/robdd', type: 'robdd', make: () => {
    // A reduced ordered BDD shape: 8 variable levels, 2-5 nodes per level, low/high edges to
    // strictly lower levels or terminals, no node with identical low and high.
    const r = rng(114);
    const nodes = [{ id: 't0', name: '0', kind: 'bdd_terminal' }, { id: 't1', name: '1', kind: 'bdd_terminal' }];
    const levels = [];
    for (let v = 8; v >= 1; v--) {
      
      const count = v === 1 ? 1 : 2 + Math.floor(r() * 4);
      const lvl = [];
      for (let k = 0; k < count; k++) { const id = `x${v}_${k}`; nodes.push({ id, name: `x${v}`, label: `x${v}: cond${v}`, kind: 'bdd_gate' }); lvl.push(id); }
      levels.push(lvl);
    }
    const edges = [];
    levels.forEach((lvl, li) => {
      const below = levels.slice(0, li).flat().concat(['t0', 't1']);
      lvl.forEach(id => {
        const lo = pick(r, below);
        let hi = pick(r, below);
        if (hi === lo) hi = lo === 't1' ? 't0' : 't1';
        edges.push({ id: `${id}_lo`, source: id, target: lo, kind: 'low_branch', label: 'lo (0)' });
        edges.push({ id: `${id}_hi`, source: id, target: hi, kind: 'high_branch', label: 'hi (1)' });
      });
    });
    return loadGraphIrToCytoscape(ir('robdd', nodes, edges));
  } });

  return cases;
}

// ───────────────────────── independent checks ─────────────────────────

function rectEdges(r) {
  return { l: r.cx - r.w / 2, t: r.cy - r.h / 2, r: r.cx + r.w / 2, b: r.cy + r.h / 2 };
}

function fromLTRB(l, t, r, b) {
  return { cx: (l + r) / 2, cy: (t + b) / 2, w: r - l, h: b - t };
}

function overlapArea(a, b) {
  const A = rectEdges(a);
  const B = rectEdges(b);
  const w = Math.min(A.r, B.r) - Math.max(A.l, B.l);
  const h = Math.min(A.b, B.b) - Math.max(A.t, B.t);
  return w > 0.5 && h > 0.5 ? w * h : 0;
}

function gapBetween(a, b) {
  const A = rectEdges(a);
  const B = rectEdges(b);
  const dx = Math.max(B.l - A.r, A.l - B.r);
  const dy = Math.max(B.t - A.b, A.t - B.b);
  return Math.max(dx, dy);
}

/**
 * Emulates what the canvas draws: leaf cards at their positions; package boxes as Cytoscape sizes
 * them (children bbox incl. child labels + padding), enlarged to data.layoutBox when present;
 * footprints add the centred label above each package.
 */
function geometryOf(elements) {
  const nodes = elements.filter(e => e && e.data && !isEdge(e));
  const byId = new Map(nodes.map(n => [String(n.data.id), n]));
  const children = new Map();
  nodes.forEach(n => {
    const p = n.data.parent !== undefined && n.data.parent !== null ? String(n.data.parent) : null;
    if (p && byId.has(p)) {
      if (!children.has(p)) children.set(p, []);
      children.get(p).push(String(n.data.id));
    }
  });
  const info = new Map();
  const issues = [];
  const compute = (id, depth = 0) => {
    if (info.has(id)) return info.get(id);
    if (depth > 64) throw new Error('parent cycle');
    const n = byId.get(id);
    const d = n.data;
    const kids = children.get(id) || [];
    const parent = d.parent !== undefined && d.parent !== null && byId.has(String(d.parent)) ? String(d.parent) : null;
    let rec;
    if (kids.length === 0) {
      const ok = n.position && Number.isFinite(n.position.x) && Number.isFinite(n.position.y);
      if (!ok) issues.push(`NaN/missing position: ${id}`);
      const s = G.nodeSize(d);
      const rect = { cx: ok ? n.position.x : 0, cy: ok ? n.position.y : 0, w: s.w, h: s.h };
      rec = { id, parent, isCompound: false, drawn: rect, foot: rect, natural: rect, pad: 0 };
    } else {
      const kidRecs = kids.map(k => compute(k, depth + 1));
      const bb = G.boundsOf(kidRecs.map(k => k.foot));
      const isPkg = Boolean(d.isPackage);
      const pad = isPkg ? G.packagePadding(d.nestLevel || 0) : 0;
      const natural = fromLTRB(bb.minX - pad, bb.minY - pad, bb.maxX + pad, bb.maxY + pad);
      let drawn = natural;
      const box = d.layoutBox;
      if (box && Number.isFinite(box.cx)) {
        const N = rectEdges(natural);
        const L = rectEdges(box);
        // min-width/min-height enlarge to the target; children outside it still widen the box.
        drawn = fromLTRB(Math.min(N.l, L.l), Math.min(N.t, L.t), Math.max(N.r, L.r), Math.max(N.b, L.b));
        if (N.l < L.l - 1 || N.t < L.t - 1 || N.r > L.r + 1 || N.b > L.b + 1) {
          issues.push(`children spill out of layoutBox: ${id} by ${Math.max(L.l - N.l, L.t - N.t, N.r - L.r, N.b - L.b).toFixed(1)}px`);
        }
      }
      const band = isPkg ? G.packageLabelBand(d.nestLevel || 0) : 0;
      const labelW = isPkg ? G.packageLabelWidth(d.label || d.name || id, d.nestLevel || 0) : 0;
      const D = rectEdges(drawn);
      const footW = Math.max(drawn.w, labelW);
      const foot = fromLTRB(drawn.cx - footW / 2, D.t - band, drawn.cx + footW / 2, D.b);
      rec = { id, parent, isCompound: true, drawn, foot, natural, pad };
    }
    info.set(id, rec);
    return rec;
  };
  nodes.forEach(n => compute(String(n.data.id)));
  return { info, issues, byId };
}

function ancestorsOf(info, id) {
  const out = new Set();
  let cur = info.get(id) && info.get(id).parent;
  while (cur) {
    out.add(cur);
    cur = info.get(cur) && info.get(cur).parent;
  }
  return out;
}

function segIntersectsInterior(p, q, r, inset) {
  return G.segmentIntersectsRectInterior(p, q, r, inset);
}

function edgePolyline(edge, info) {
  const d = edge.data;
  const route = d.layoutRoute;
  const s = info.get(String(d.source));
  const t = info.get(String(d.target));
  if (!s || !t) return null;
  if (route && Array.isArray(route.points) && route.points.length >= 2) return { points: route.points, routed: true, style: route.style };
  const a = G.clipToRectBorder(s.drawn, { x: t.drawn.cx, y: t.drawn.cy });
  const b = G.clipToRectBorder(t.drawn, { x: s.drawn.cx, y: s.drawn.cy });
  return { points: [a, b], routed: false, style: route ? route.style : 'straight-approx' };
}

function inspect(elements, type) {
  const { info, issues } = geometryOf(elements);
  const hard = { nan: 0, siblingOverlaps: 0, spills: 0, badAttachments: 0, nonOrthogonal: 0, edgeThroughCard: 0, edgeThroughPackage: 0 };
  const examples = [];
  const note = (k, msg) => { hard[k]++; if (examples.length < 12) examples.push(msg); };
  issues.forEach(m => { if (m.startsWith('NaN')) note('nan', m); else note('spills', m); });

  // Sibling overlaps (same parent), footprints include package label bands.
  const groups = new Map();
  info.forEach(rec => {
    const k = rec.parent || '__root__';
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(rec);
  });
  let minSiblingGap = Infinity;
  groups.forEach(list => {
    const sorted = list.slice().sort((a, b) => (a.foot.cx - a.foot.w / 2) - (b.foot.cx - b.foot.w / 2));
    for (let i = 0; i < sorted.length; i++) {
      const A = rectEdges(sorted[i].foot);
      for (let j = i + 1; j < sorted.length; j++) {
        const B = rectEdges(sorted[j].foot);
        if (B.l > A.r + 400) break;
        const area = overlapArea(sorted[i].foot, sorted[j].foot);
        if (area > 0) note('siblingOverlaps', `overlap ${sorted[i].id} x ${sorted[j].id} (${area.toFixed(0)} px²)`);
        else minSiblingGap = Math.min(minSiblingGap, gapBetween(sorted[i].foot, sorted[j].foot));
      }
    }
  });

  // Edges.
  const edges = elements.filter(isEdge);
  const leaves = [...info.values()].filter(r => !r.isCompound);
  const compounds = [...info.values()].filter(r => r.isCompound);
  const segs = [];
  let bends = 0;
  let totalLen = 0;
  let edgeN = 0;
  let routedN = 0;
  const labelRects = [];
  // Sequence and timing messages cross other lifelines and tracks by notation.
  const isSequence = type === 'sequence' || type === 'timing';
  for (const e of edges) {
    const s = String(e.data.source);
    const t = String(e.data.target);
    if (s === t) continue;
    const hiddenStyle = e.data.layoutRoute && ['merged', 'embedded', 'loop-grouped'].includes(e.data.layoutRoute.style);
    if (hiddenStyle) continue;
    // Cytoscape draws an edge between a package and something inside it as its own compound loop.
    if (ancestorsOf(info, s).has(t) || ancestorsOf(info, t).has(s)) continue;
    const poly = edgePolyline(e, info);
    if (!poly) continue;
    edgeN++;
    if (poly.routed) routedN++;
    const pts = poly.points;
    bends += Math.max(0, pts.length - 2);
    totalLen += G.polylineLength(pts);
    const S = info.get(s);
    const T = info.get(t);
    if (poly.routed) {
      const attach = (p, r) => {
        const E = rectEdges(r);
        const inside = p.x >= E.l - 0.5 && p.x <= E.r + 0.5 && p.y >= E.t - 0.5 && p.y <= E.b + 0.5;
        return inside ? (isSequence || S.isCompound || T.isCompound ? 0 : G.distanceToRectBorder(p, r)) : G.distanceToRectBorder(p, r);
      };
      const da = attach(pts[0], S.drawn);
      const db = attach(pts[pts.length - 1], T.drawn);
      if (da > 2.5 || db > 2.5) note('badAttachments', `endpoint off border: ${e.data.id} (${da.toFixed(1)}px / ${db.toFixed(1)}px)`);
      if (poly.style === 'orthogonal') {
        for (let i = 1; i < pts.length; i++) {
          if (Math.abs(pts[i].x - pts[i - 1].x) > 0.6 && Math.abs(pts[i].y - pts[i - 1].y) > 0.6) {
            note('nonOrthogonal', `diagonal segment: ${e.data.id}`);
            break;
          }
        }
      }
      if (e.data.layoutRoute.label) {
        const size = G.edgeLabelSize(e.data.label);
        if (size) labelRects.push({ id: e.data.id, cx: e.data.layoutRoute.label.x, cy: e.data.layoutRoute.label.y, w: size.w, h: size.h, s, t });
      }
    }
    const excluded = new Set([s, t, ...ancestorsOf(info, s), ...ancestorsOf(info, t)]);
    for (let i = 1; i < pts.length; i++) {
      const p = pts[i - 1];
      const q = pts[i];
      segs.push({ p, q, edge: e.data.id, s, t });
      if (isSequence) continue;
      for (const L of leaves) {
        if (excluded.has(L.id)) continue;
        if (segIntersectsInterior(p, q, L.drawn, 3)) { note('edgeThroughCard', `${e.data.id} crosses card ${L.id}`); }
      }
      for (const C of compounds) {
        if (excluded.has(C.id)) continue;
        if (segIntersectsInterior(p, q, C.drawn, 3)) { note('edgeThroughPackage', `${e.data.id} crosses package ${C.id}`); }
      }
    }
  }

  // Crossings between segments of different edges not sharing an endpoint node (bucketed).
  let crossings = 0;
  const cell = 400;
  const buckets = new Map();
  segs.forEach((sg, idx) => {
    const x0 = Math.floor(Math.min(sg.p.x, sg.q.x) / cell);
    const x1 = Math.floor(Math.max(sg.p.x, sg.q.x) / cell);
    const y0 = Math.floor(Math.min(sg.p.y, sg.q.y) / cell);
    const y1 = Math.floor(Math.max(sg.p.y, sg.q.y) / cell);
    for (let x = x0; x <= x1; x++) for (let y = y0; y <= y1; y++) {
      const k = `${x},${y}`;
      if (!buckets.has(k)) buckets.set(k, []);
      buckets.get(k).push(idx);
    }
  });
  const counted = new Set();
  buckets.forEach(list => {
    for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length; j++) {
      const a = segs[list[i]];
      const b = segs[list[j]];
      if (a.edge === b.edge || a.s === b.s || a.s === b.t || a.t === b.s || a.t === b.t) continue;
      const key = list[i] < list[j] ? `${list[i]}|${list[j]}` : `${list[j]}|${list[i]}`;
      if (counted.has(key)) continue;
      if (G.segmentsProperlyIntersect(a.p, a.q, b.p, b.q)) { counted.add(key); crossings++; }
    }
  });

  // Label collisions.
  let labelOnCard = 0;
  let labelOnLabel = 0;
  if (!isSequence) labelRects.forEach(lr => {
    for (const L of leaves) {
      if (L.id === lr.s || L.id === lr.t) continue;
      if (overlapArea(lr, L.drawn) > 4) labelOnCard++;
    }
  });
  for (let i = 0; i < labelRects.length; i++) for (let j = i + 1; j < labelRects.length; j++) {
    if (overlapArea(labelRects[i], labelRects[j]) > 4) labelOnLabel++;
  }

  const all = G.boundsOf([...info.values()].filter(r => !r.parent).map(r => r.foot));
  const leafArea = leaves.reduce((a, r) => a + r.drawn.w * r.drawn.h, 0);
  const meanCard = leaves.length ? leaves.reduce((a, r) => a + (r.drawn.w + r.drawn.h) / 2, 0) / leaves.length : 1;
  const hardTotal = Object.values(hard).reduce((a, b) => a + b, 0);
  return {
    pass: hardTotal === 0,
    hard,
    examples,
    soft: {
      nodes: info.size,
      edges: edgeN,
      routed: routedN,
      crossings,
      bends,
      labelOnCard,
      labelOnLabel,
      meanEdgeLenPerCard: edgeN ? +(totalLen / edgeN / meanCard).toFixed(2) : 0,
      areaUse: all.w * all.h > 0 ? +(leafArea / (all.w * all.h)).toFixed(3) : 0,
      aspect: all.h > 0 ? +(all.w / all.h).toFixed(2) : 0,
      size: `${Math.round(all.w)}x${Math.round(all.h)}`,
      minSiblingGap: Number.isFinite(minSiblingGap) ? Math.round(minSiblingGap) : null
    }
  };
}

function fingerprint(elements) {
  const parts = [];
  elements.forEach(e => {
    if (!e || !e.data) return;
    if (isEdge(e)) parts.push(`${e.data.id}:${JSON.stringify(e.data.layoutRoute || null)}`);
    else parts.push(`${e.data.id}:${e.position ? `${e.position.x.toFixed(3)},${e.position.y.toFixed(3)}` : 'none'}:${JSON.stringify(e.data.layoutBox || null)}`);
  });
  return parts.join('\n');
}

// ───────────────────────── run ─────────────────────────

const FILTERS = FILTER ? FILTER.split(',').map(s => s.trim()).filter(Boolean) : [];
const cases = [...bundledCases(), ...realCodeCases(), ...syntheticCases()].filter(c => !FILTERS.length || FILTERS.some(f => c.name.includes(f)));
const results = [];
for (const c of cases) {
  const row = { name: c.name, type: c.type };
  try {
    const els = c.make();
    if (!els.length) { row.skipped = 'empty input'; results.push(row); console.log(`SKIP ${c.name}: empty`); continue; }
    const t0 = performance.now();
    let dumpIndex = 0;
    const diagnostics = process.env.OH_DUMP_ROUTER ? { onRouterInput: (input, options) => {
      fs.mkdirSync(process.env.OH_DUMP_ROUTER, { recursive: true });
      fs.writeFileSync(path.join(process.env.OH_DUMP_ROUTER, `${c.name.replace(/[^A-Za-z0-9]+/g, '_')}_${dumpIndex++}.json`), JSON.stringify({ input, options }));
    } } : undefined;
    const r = await computeLayout(els, c.type, { viewport: VIEWPORT, diagnostics });
    row.ms = Math.round(performance.now() - t0);
    row.strategy = r.strategy;
    row.candidates = r.candidates || [];
    row.warnings = r.warnings;
    row.engineMetrics = r.metrics ? { nodeOverlaps: r.metrics.nodeOverlaps, edgeNodeCrossings: r.metrics.edgeNodeCrossings, edgeCrossings: r.metrics.edgeCrossings } : null;
    const kept = r.elements;
    const nodeIds = new Set(kept.filter(e => e.data && !isEdge(e)).map(e => String(e.data.id)));
    const drawn = kept.filter(e => !isEdge(e) || (nodeIds.has(String(e.data.source)) && nodeIds.has(String(e.data.target))));
    row.gate = inspect(drawn, c.type);

    // Determinism: a second run on fresh input must give the same geometry.
    const els2 = c.make();
    const r2 = await computeLayout(els2, c.type, { viewport: VIEWPORT });
    row.deterministic = fingerprint(r2.elements) === fingerprint(kept);
    if (!row.deterministic) { row.gate.pass = false; row.gate.examples.push('non-deterministic output'); }

    // Legacy baseline.
    const legacyEls = c.make();
    const legacy = computeDeterministicLayout(legacyEls, c.type);
    const lNodeIds = new Set(legacy.filter(e => e.data && !isEdge(e)).map(e => String(e.data.id)));
    row.legacy = inspect(legacy.filter(e => !isEdge(e) || (lNodeIds.has(String(e.data.source)) && lNodeIds.has(String(e.data.target)))), c.type);
  } catch (err) {
    row.error = (err && err.stack) || String(err);
  }
  results.push(row);
  const g = row.gate;
  const flag = row.error ? 'ERROR' : (row.skipped ? 'SKIP' : (g.pass ? 'PASS' : 'FAIL'));
  const hardStr = g ? Object.entries(g.hard).filter(([, v]) => v).map(([k, v]) => `${k}=${v}`).join(' ') : '';
  const lg = row.legacy;
  console.log(`${flag.padEnd(5)} ${c.name.padEnd(42)} ${String(row.ms ?? '').padStart(6)}ms ${g ? `n=${g.soft.nodes} e=${g.soft.edges} routed=${g.soft.routed} X=${g.soft.crossings} bends=${g.soft.bends} lblCard=${g.soft.labelOnCard} lblLbl=${g.soft.labelOnLabel} len=${g.soft.meanEdgeLenPerCard} area=${g.soft.areaUse} ${g.soft.size}` : ''} ${hardStr}`);
  if (lg) {
    const lHard = Object.entries(lg.hard).filter(([, v]) => v).map(([k, v]) => `${k}=${v}`).join(' ');
    console.log(`      legacy: X=${lg.soft.crossings} len=${lg.soft.meanEdgeLenPerCard} area=${lg.soft.areaUse} ${lg.soft.size} ${lHard}`);
  }
  if (row.strategy) console.log(`      strategy: ${row.strategy}${row.deterministic === false ? '  [NON-DETERMINISTIC]' : ''}`);
  if (process.env.OH_PROFILE && row.candidates) row.candidates.forEach(c => console.log(`        block ${c.block} n=${c.nodes} e=${c.edges} ${c.candidate.padEnd(44)} ${String(c.ms).padStart(6)}ms score=${c.score} ${c.metrics ? JSON.stringify(c.metrics) : ''}`));
  if (row.warnings && row.warnings.length) console.log(`      warnings: ${row.warnings.join(' | ')}`);
  if (g && g.examples.length) console.log(`      e.g. ${g.examples.slice(0, 5).join('; ')}`);
  if (row.error) console.log(`      ${row.error.split('\n').slice(0, 4).join('\n      ')}`);
}
fs.writeFileSync(OUT, JSON.stringify(results, null, 2));
const summary = results.reduce((a, r) => {
  const k = r.error ? 'error' : r.skipped ? 'skip' : r.gate.pass ? 'pass' : 'fail';
  a[k] = (a[k] || 0) + 1;
  return a;
}, {});
console.log(`\nSUMMARY ${JSON.stringify(summary)}  results: ${OUT}`);
