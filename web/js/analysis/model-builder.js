/**
 * OpenHeart In-Browser Class Model Builder
 * Turns per-file declaration extracts (source-parser.js) into a class-diagram Graph IR.
 *
 *  - Node ids are fully qualified names (package + enclosing types), so same-named types in
 *    different packages or nested in different outers stay distinct.
 *  - Relations are resolved through nested scope, explicit imports, the declaring package,
 *    wildcard imports, and only then a corpus-unique simple name. Unresolvable references
 *    are dropped and counted, never attached to an arbitrary same-named type.
 *  - Every type node carries a source anchor (path, start/end line and column) taken from the
 *    declaration tokens, and members carry their declaration line.
 */

import { BUILTIN_TYPE_NAMES } from './source-parser.js';

export const EXTRACTOR_ID = 'openheart-studio-declaration-extractor';
export const EXTRACTOR_VERSION = '2.0.0';

const RELATION_ARROWS = {
  generalization: '--|>',
  realization: '..|>',
  composition: '*--',
  aggregation: 'o--',
  association: '-->',
  dependency: '..>'
};

function dirOf(path) {
  const idx = path.lastIndexOf('/');
  return idx >= 0 ? path.slice(0, idx) : '';
}

function dottedDir(path) {
  return dirOf(path).split('/').filter(Boolean).join('.');
}

function filePackage(parse) {
  switch (parse.language) {
    case 'java':
    case 'kotlin':
    case 'scala':
      return parse.packageDecl || '';
    case 'python':
      return parse.packageDecl || '';
    case 'csharp':
      return (parse.fileNamespace || []).join('.');
    default:
      // Go, Rust, TypeScript/JavaScript, Swift and C/C++ have no single declared package;
      // the directory is the package boundary used by their module systems.
      return dottedDir(parse.path);
  }
}

function signatureForField(f) {
  return f.type ? `${f.name}: ${f.type}${f.default_value ? ` = ${f.default_value}` : ''}` : `${f.name}${f.default_value ? ` = ${f.default_value}` : ''}`;
}

function signatureForMethod(m) {
  const params = (m.parameters || []).map(p => {
    if (p.name && p.type) return `${p.name}: ${p.type}`;
    return p.name || p.type || '_';
  }).join(', ');
  return `${m.name}(${params})${m.return_type ? `: ${m.return_type}` : ''}`;
}

export function buildClassModel(parses, options = {}) {
  const { title = 'Codebase', rawUrlFor = null } = options;
  const sorted = parses.slice().sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));

  const entries = [];
  const byFqn = new Map();
  const bySimple = new Map();
  const usedIds = new Set();
  const fileContexts = new Map();
  const stats = {
    total_classes: 0,
    total_relations: 0,
    unresolved_references: 0,
    ambiguous_references: 0,
    unresolved_extensions: 0,
    resolution: { scope: 0, import: 0, package: 0, wildcard: 0, 'unique-name': 0 },
    languages: {}
  };

  // ── 1. File contexts and type entries ──
  for (const parse of sorted) {
    const pkg = filePackage(parse);
    const imports = new Map();
    (parse.imports || []).forEach(imp => {
      if (imp && imp.alias && imp.target && !imports.has(imp.alias)) imports.set(imp.alias, imp.target);
    });
    const ctx = {
      path: parse.path,
      language: parse.language,
      package: pkg,
      imports,
      wildcards: (parse.wildcardImports || []).slice()
    };
    fileContexts.set(parse.path, ctx);
    stats.languages[parse.language] = (stats.languages[parse.language] || 0) + 1;

    const local = [];
    (parse.classes || []).forEach((rec, idx) => {
      const chain = [];
      let cur = rec;
      let guard = 0;
      while (cur && guard < 32) {
        chain.unshift(cur.name);
        cur = cur.outer !== null && cur.outer !== undefined ? parse.classes[cur.outer] : null;
        guard++;
      }
      const namespace = parse.language === 'java' || parse.language === 'kotlin' || parse.language === 'scala' || parse.language === 'python'
        ? []
        : (rec.namespace || []);
      const prefix = [pkg, ...namespace].filter(Boolean).join('.');
      const localName = chain.join('.');
      const fqn = prefix ? `${prefix}.${localName}` : localName;
      let id = fqn;
      if (usedIds.has(id)) id = `${fqn}@${parse.path}`;
      let n = 2;
      while (usedIds.has(id)) id = `${fqn}@${parse.path}#${n++}`;
      usedIds.add(id);
      const entry = {
        id,
        fqn,
        prefix,
        simple: rec.name,
        localName,
        rec,
        ctx,
        parse,
        outerEntry: null,
        fields: rec.fields.slice(),
        methods: rec.methods.slice(),
        superTypes: rec.superTypes.slice()
      };
      local[idx] = entry;
      entries.push(entry);
      if (!byFqn.has(fqn)) byFqn.set(fqn, entry);
      if (!bySimple.has(rec.name)) bySimple.set(rec.name, []);
      bySimple.get(rec.name).push(entry);
    });
    (parse.classes || []).forEach((rec, idx) => {
      if (rec.outer !== null && rec.outer !== undefined && local[rec.outer]) local[idx].outerEntry = local[rec.outer];
    });
    ctx.entries = local.filter(Boolean);
  }

  // ── 2. Name resolution ──
  const resolveSimple = (simple, ctx, fromEntry) => {
    let cur = fromEntry;
    let guard = 0;
    while (cur && guard < 32) {
      const nested = byFqn.get(`${cur.fqn}.${simple}`);
      if (nested) return { entry: nested, via: 'scope' };
      if (cur.simple === simple) return { entry: cur, via: 'scope' };
      cur = cur.outerEntry;
      guard++;
    }
    const imported = ctx.imports.get(simple);
    if (imported) {
      const hit = byFqn.get(imported);
      if (hit) return { entry: hit, via: 'import' };
      // An explicit import to an external type wins over any same-named type in the corpus.
      return { entry: null, via: 'external-import' };
    }
    const prefix = fromEntry ? fromEntry.prefix : ctx.package;
    if (prefix) {
      const parts = prefix.split('.');
      for (let len = parts.length; len >= 1; len--) {
        const hit = byFqn.get(`${parts.slice(0, len).join('.')}.${simple}`);
        if (hit) return { entry: hit, via: 'package' };
        if (ctx.language !== 'csharp' && ctx.language !== 'cpp') break; // only C#/C++ namespaces nest lookups outward
      }
    } else {
      const hit = byFqn.get(simple);
      if (hit) return { entry: hit, via: 'package' };
    }
    for (const w of ctx.wildcards) {
      const hit = byFqn.get(`${w}.${simple}`);
      if (hit) return { entry: hit, via: 'wildcard' };
    }
    const candidates = bySimple.get(simple);
    if (candidates && candidates.length === 1) return { entry: candidates[0], via: 'unique-name' };
    if (candidates && candidates.length > 1) return { entry: null, via: 'ambiguous' };
    return { entry: null, via: 'unresolved' };
  };

  const resolveName = (name, ctx, fromEntry) => {
    if (!name) return { entry: null, via: 'unresolved' };
    if (!name.includes('.')) return resolveSimple(name, ctx, fromEntry);
    const exact = byFqn.get(name);
    if (exact) return { entry: exact, via: 'import' };
    const [head, ...rest] = name.split('.');
    const headHit = resolveSimple(head, ctx, fromEntry);
    if (headHit.entry) {
      const nested = byFqn.get(`${headHit.entry.fqn}.${rest.join('.')}`);
      if (nested) return { entry: nested, via: headHit.via };
    }
    const viaImport = ctx.imports.get(head);
    if (viaImport) {
      const hit = byFqn.get(`${viaImport}.${rest.join('.')}`);
      if (hit) return { entry: hit, via: 'import' };
    }
    if (ctx.package) {
      const hit = byFqn.get(`${ctx.package}.${name}`);
      if (hit) return { entry: hit, via: 'package' };
    }
    // Qualified reference through a package alias (Go `pkg.Type`, Rust `module::Type`).
    const simple = rest[rest.length - 1];
    const candidates = (bySimple.get(simple) || []).filter(e =>
      e.fqn.endsWith(`.${name}`) || (e.fqn.endsWith(`.${rest.join('.')}`) && e.prefix.split('.').pop() === head));
    if (candidates.length === 1) return { entry: candidates[0], via: 'import' };
    return { entry: null, via: candidates.length > 1 ? 'ambiguous' : 'unresolved' };
  };

  const countMiss = (via) => {
    if (via === 'ambiguous') stats.ambiguous_references++;
    else if (via !== 'external-import') stats.unresolved_references++;
  };

  // ── 3. Extensions (Rust impl, Swift extension, Go receiver methods) ──
  for (const parse of sorted) {
    const ctx = fileContexts.get(parse.path);
    (parse.extensions || []).forEach(ext => {
      const target = resolveName(ext.qualifiedTarget || ext.targetName, ctx, null);
      if (!target.entry) {
        stats.unresolved_extensions++;
        return;
      }
      const entry = target.entry;
      ext.fields.forEach(f => entry.fields.push(Object.assign({}, f, { declared_in: parse.path })));
      ext.methods.forEach(m => entry.methods.push(Object.assign({}, m, { declared_in: parse.path })));
      ext.superTypes.forEach(s => entry.superTypes.push(Object.assign({}, s, { ctxPath: parse.path })));
    });
  }

  // ── 4. Relations ──
  const edges = [];
  const edgeKeys = new Set();
  const addEdge = (kind, src, tgt, via, detail) => {
    const key = `${kind}|${src.id}|${tgt.id}`;
    if (edgeKeys.has(key)) return;
    edgeKeys.add(key);
    stats.resolution[via] = (stats.resolution[via] || 0) + 1;
    edges.push({
      id: `${kind}:${src.id}->${tgt.id}`,
      source: src.id,
      target: tgt.id,
      kind,
      label: '',
      arrow: RELATION_ARROWS[kind] || '-->',
      resolution: via,
      evidence: detail || null
    });
  };

  for (const entry of entries) {
    const srcKind = entry.rec.kind;
    for (const st of entry.superTypes) {
      const ctx = st.ctxPath ? fileContexts.get(st.ctxPath) : entry.ctx;
      const hit = resolveName(st.name, ctx, entry);
      if (!hit.entry || hit.entry === entry) {
        if (!hit.entry) countMiss(hit.via);
        continue;
      }
      const targetKind = hit.entry.rec.kind;
      let kind;
      if (st.role === 'embeds') kind = 'composition';
      else if (st.role === 'implements') kind = 'realization';
      else if (st.role === 'extends') kind = (targetKind === 'interface' && srcKind !== 'interface') ? 'realization' : 'generalization';
      else kind = (targetKind === 'interface' && srcKind !== 'interface') ? 'realization' : 'generalization';
      addEdge(kind, entry, hit.entry, hit.via, { declaration_line: st.line || null });
    }
    for (const f of entry.fields) {
      const ctx = f.declared_in ? fileContexts.get(f.declared_in) : entry.ctx;
      for (const ref of (f.type_refs || [])) {
        const simple = ref.name.split('.').pop();
        if (BUILTIN_TYPE_NAMES.has(simple) && !ref.name.includes('.')) continue;
        const hit = resolveName(ref.name, ctx, entry);
        if (!hit.entry) {
          countMiss(hit.via);
          continue;
        }
        addEdge(ref.collection ? 'aggregation' : 'association', entry, hit.entry, hit.via, { field: f.name, declaration_line: f.line || null });
      }
    }
  }

  // ── 5. Graph IR ──
  const nodes = [];
  const packageIds = new Map();
  const packageIdFor = (prefix) => {
    const key = prefix || '';
    if (packageIds.has(key)) return packageIds.get(key);
    const id = `pkg:${key || '(default)'}`;
    packageIds.set(key, id);
    const display = key || '(default package)';
    nodes.push({
      id,
      name: display,
      label: `package [${display}]`,
      kind: 'package',
      stereotype: '<<package>>',
      is_package: true,
      is_domain_tier: true,
      nest_level: 0,
      parent: null,
      file: null,
      lines: [],
      fields: [],
      methods: [],
      instructions: []
    });
    return id;
  };

  entries.forEach(entry => { packageIdFor(entry.prefix); });

  for (const entry of entries) {
    const rec = entry.rec;
    const path = entry.parse.path;
    const fields = entry.fields.map(f => ({
      visibility: f.visibility || '+',
      name: f.name,
      type_name: f.type,
      signature: signatureForField(f),
      is_static: Boolean(f.is_static),
      is_final: Boolean(f.is_final),
      is_collection: Boolean(f.is_collection),
      default_value: f.default_value || null,
      line: f.line || null,
      declared_in: f.declared_in || null
    }));
    (rec.constants || []).forEach(c => {
      fields.push({
        visibility: '+',
        name: c.name,
        type_name: null,
        signature: c.name,
        is_static: true,
        is_final: true,
        is_collection: false,
        is_enum_literal: true,
        default_value: null,
        line: c.line || null,
        declared_in: null
      });
    });
    const methods = entry.methods.map(m => ({
      visibility: m.visibility || '+',
      name: m.name,
      type_name: m.return_type,
      return_type: m.return_type,
      signature: signatureForMethod(m),
      parameters: (m.parameters || []).map(p => ({ name: p.name, type: p.type })),
      is_static: Boolean(m.is_static),
      is_abstract: Boolean(m.is_abstract),
      is_async: Boolean(m.is_async),
      is_constructor: Boolean(m.is_constructor),
      line: m.line || null,
      declared_in: m.declared_in || null
    }));
    stats.total_classes++;
    nodes.push({
      id: entry.id,
      name: entry.localName,
      label: entry.localName,
      qualified_name: entry.fqn,
      kind: rec.kind,
      stereotype: `<<${rec.stereotype || rec.kind}>>`,
      visibility: rec.visibility || null,
      parent: packageIdFor(entry.prefix),
      outer: entry.outerEntry ? entry.outerEntry.id : null,
      nest_level: 1,
      is_package: false,
      is_domain_tier: false,
      language: entry.parse.language,
      file: path,
      raw_url: rawUrlFor ? rawUrlFor(path) : null,
      lines: [rec.line],
      anchor: {
        path,
        start_line: rec.line,
        start_col: rec.col,
        end_line: rec.endLine,
        end_col: rec.endCol
      },
      fields,
      methods,
      instructions: []
    });
  }

  stats.total_relations = edges.length;

  return {
    graph_ir: {
      schema: 'openheart.graph-ir',
      schema_version: 1,
      diagram_type: 'class',
      title,
      nodes,
      edges,
      metadata: {
        total_nodes: nodes.length,
        total_edges: edges.length,
        generator: EXTRACTOR_ID,
        generator_version: EXTRACTOR_VERSION,
        approximate: true,
        content_hash: null
      }
    },
    stats
  };
}
