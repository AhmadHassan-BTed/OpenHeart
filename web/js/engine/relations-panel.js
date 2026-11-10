/**
 * OpenHeart Relations panel (inspector tab): click-to-explore for the selected element.
 *
 * For a node with `data.symbol` (or `data.function`, the enclosing function of a node inside
 * one function's view) it asks the program model (program-model.js) for:
 *   callers / callees (depth 1-4, an expandable tree), dependencies (calls, reads, writes,
 *   types with relations, packages) and dependents (callers, readers, writers, type users,
 *   subtypes, overrides), plus the notes and unresolved call sites the engine reports.
 * Every entry is a button; the explorer decides whether to focus it on the canvas or open its
 * focused view. Errors (404, 422, ...) are shown as messages, never as empty results.
 *
 * DOM is built with dom.js h() only (Trusted Types safe); every control is a native button,
 * select or details element, so the panel is fully keyboard accessible.
 */

import { h, replaceChildren } from '../dom.js';
import { icon } from '../icons.js';

const DEPTHS = [1, 2, 3, 4];
const MAX_TREE_ITEMS = 400;
const MAX_LIST_ITEMS = 200;
const MAX_SITES = 20;

export const RELATION_ACTIONS = Object.freeze([
  { id: 'callgraph', label: 'Call graph from here', view: 'callgraph', depth: 2 },
  { id: 'cfg', label: 'Control flow', view: 'cfg', depth: 1 },
  { id: 'dfg', label: 'Data flow', view: 'dfg', depth: 1 },
  { id: 'cdg', label: 'Control dependence', view: 'cdg', depth: 1 },
  { id: 'sequence', label: 'Sequence from here', view: 'sequence', depth: 2 },
  { id: 'impact', label: 'Impact', view: null },
  { id: 'class', label: 'Show in class diagram', view: null }
]);

/** Human-readable text for a query failure. */
export function describeQueryError(err) {
  const status = err && Number.isInteger(err.status) ? err.status : null;
  const msg = String((err && err.message) || err || 'unknown error');
  switch (status) {
    case 404: return `Not found in the analysis (404): ${msg}`;
    case 422: return `Not derivable for this element (422): ${msg}`;
    case 410: return `Session ended: ${msg}`;
    case 400: return `Request rejected (400): ${msg}`;
    case 0: return msg;
    default: return status ? `The engine failed (${status}): ${msg}` : msg;
  }
}

function plural(n, word) {
  return `${n.toLocaleString()} ${word}${n === 1 ? '' : 's'}`;
}

function shortName(qualified) {
  const s = String(qualified || '');
  const noParams = s.replace(/\(.*$/, '');
  const parts = noParams.split(/[.:#/]/).filter(Boolean);
  const last = parts.length ? parts[parts.length - 1] : noParams;
  return s.includes('(') ? `${last}()` : last;
}

function siteText(site) {
  if (!site || typeof site !== 'object') return '';
  const file = typeof site.file === 'string' ? site.file : '?';
  const line = Number.isInteger(site.line) ? `:${site.line}` : '';
  const col = Number.isInteger(site.column) ? `:${site.column}` : '';
  return `${file}${line}${col}`;
}

function message(kind, text) {
  return h('p', { className: `rel-message rel-message-${kind}`, role: kind === 'error' ? 'alert' : null }, text);
}

function notesList(notes) {
  const list = Array.isArray(notes) ? notes.filter(n => typeof n === 'string' && n) : [];
  if (!list.length) return null;
  return h('ul', { className: 'rel-notes', 'aria-label': 'Engine notes' }, list.slice(0, 30).map(n => h('li', null, n)));
}

function unresolvedList(sites) {
  const list = Array.isArray(sites) ? sites.filter(s => s && typeof s === 'object') : [];
  if (!list.length) return null;
  const shown = list.slice(0, MAX_SITES);
  return h('div', { className: 'rel-unresolved' },
    h('div', { className: 'rel-subhead' }, `Unresolved call sites (${list.length.toLocaleString()})`),
    h('ul', null,
      shown.map(s => h('li', null,
        h('span', { className: 'rel-site' }, siteText(s)),
        typeof s.reason === 'string' && s.reason ? h('span', { className: 'rel-site-reason' }, ` ${s.reason}`) : null
      )),
      list.length > shown.length ? h('li', { className: 'rel-more' }, `and ${(list.length - shown.length).toLocaleString()} more`) : null
    )
  );
}

export class RelationsPanel {
  /**
   * @param {{
   *   container: HTMLElement,
   *   getModel: () => object|null,
   *   getGap: () => string|null,
   *   onNavigate: (entry: { symbol: string, qualifiedName?: string, name?: string, role: string }) => void,
   *   onAction: (actionId: string, target: object) => Promise<any>|void,
   *   announce?: (text: string) => void
   * }} options
   */
  constructor(options) {
    this.container = options.container;
    this.getModel = options.getModel;
    this.getGap = options.getGap;
    this.onNavigate = options.onNavigate;
    this.onAction = options.onAction;
    this.announce = options.announce || (() => {});
    this.seq = 0;
    this.target = null;
    this.depths = { callers: 1, callees: 1 };
    this.statusEl = null;
    this.clear();
  }

  /** Empty state (no selection). */
  clear(text = 'Select a function, method, type or field in the diagram to explore who calls it, what it calls, and what it reads, writes and depends on.') {
    this.seq++;
    this.target = null;
    replaceChildren(this.container, h('div', { className: 'rel-empty' }, text));
  }

  /**
   * Shows the relations of a selected node (Cytoscape node data) or of a symbol the explorer
   * navigated to ({ symbol, name, qualifiedName }).
   */
  show(nodeData) {
    const data = nodeData || {};
    const symbol = typeof data.symbol === 'string' && data.symbol ? data.symbol : null;
    const enclosing = typeof data.function === 'string' && data.function ? data.function : null;
    const spec = symbol || enclosing;
    const name = String(data.name || data.rawName || data.label || data.id || spec || 'element');
    if (!spec) {
      this.seq++;
      this.target = null;
      replaceChildren(this.container,
        h('div', { className: 'rel-header' },
          h('div', { className: 'rel-title' }, name),
          h('div', { className: 'rel-sub' }, data.qualifiedName || data.kind || '')
        ),
        h('div', { className: 'rel-empty' },
          'This element carries no engine symbol, so it cannot be queried. Diagrams produced by the OpenHeart engine (the WebAssembly module or the analysis server) mark every program element with its symbol; the in-browser class model and the bundled samples do not.')
      );
      return;
    }
    const model = this.getModel();
    this.target = {
      symbol: spec,
      isEnclosing: !symbol,
      name,
      qualifiedName: data.qualifiedName || data.qualified_name || null,
      kind: data.kind || null,
      owner: null
    };
    const seq = ++this.seq;
    const header = h('div', { className: 'rel-header' },
      h('div', { className: 'rel-title' }, this.target.isEnclosing ? `${name} (inside a function)` : name),
      h('div', { className: 'rel-sub', title: spec }, this.target.isEnclosing ? `Function ${spec}` : (this.target.qualifiedName || spec))
    );
    if (!model) {
      replaceChildren(this.container, header, message('info', this.getGap() || 'No program model is available for this session.'));
      return;
    }
    this.statusEl = h('div', { className: 'rel-status', role: 'status', 'aria-live': 'polite' });
    const actions = h('div', { className: 'rel-actions', role: 'group', 'aria-label': 'Diagrams and analyses for this element' },
      RELATION_ACTIONS.map(a => h('button', {
        type: 'button',
        className: 'rel-action-btn',
        dataset: { action: a.id },
        onClick: (e) => this.runAction(a, e.currentTarget)
      }, a.label))
    );
    const callers = this.treeSection('callers', 'Callers', seq);
    const callees = this.treeSection('callees', 'Callees', seq);
    const deps = this.lazySection('Dependencies', 'Calls, field reads and writes, and types this element depends on', seq, (body) => this.loadDependencies(body, seq));
    const dependents = this.lazySection('Dependents', 'Callers, readers, writers, type users, subtypes and overrides', seq, (body) => this.loadDependents(body, seq));
    replaceChildren(this.container,
      header,
      h('div', { className: 'rel-model' }, `Answered by: ${model.label}`),
      actions,
      this.statusEl,
      callers.el,
      callees.el,
      deps,
      dependents
    );
    callers.load();
    callees.load();
  }

  setStatus(kind, text, extra = null) {
    if (!this.statusEl) return;
    if (!text && !extra) {
      replaceChildren(this.statusEl);
      return;
    }
    replaceChildren(this.statusEl, text ? message(kind, text) : null, extra);
  }

  async runAction(action, button) {
    if (!this.target || !this.onAction) return;
    const seq = this.seq;
    if (button) button.setAttribute('aria-busy', 'true');
    this.setStatus('info', `${action.label}: asking the engine...`);
    try {
      const outcome = await this.onAction(action.id, Object.assign({}, this.target), action);
      if (seq !== this.seq) return;
      if (outcome && outcome.ok === false) {
        this.setStatus('error', outcome.message || `${action.label} failed.`);
      } else if (outcome && outcome.impact) {
        this.setStatus(null, null, this.renderImpact(outcome.impact, outcome.highlighted || 0));
      } else if (outcome && outcome.message) {
        this.setStatus('info', outcome.message);
      } else {
        this.setStatus(null, null);
      }
    } catch (err) {
      if (seq === this.seq) this.setStatus('error', describeQueryError(err));
    } finally {
      if (button) button.removeAttribute('aria-busy');
    }
  }

  // ── Callers / callees ──

  treeSection(direction, title, seq) {
    const body = h('div', { className: 'rel-section-body' });
    const select = h('select', { className: 'rel-depth-select', 'aria-label': `${title} depth` },
      DEPTHS.map(d => h('option', { value: String(d) }, `Depth ${d}`))
    );
    select.value = String(this.depths[direction]);
    const content = h('div', { className: 'rel-tree-host' });
    select.addEventListener('change', () => {
      this.depths[direction] = parseInt(select.value, 10) || 1;
      load();
    });
    body.appendChild(h('div', { className: 'rel-section-tools' }, h('label', { className: 'rel-depth-label' }, 'Levels ', select)));
    body.appendChild(content);
    const el = h('details', { className: 'rel-section', open: true },
      h('summary', null, h('span', { className: 'rel-section-title' }, title), h('span', { className: 'rel-count', dataset: { role: 'count' } }, '')),
      body
    );
    const countEl = el.querySelector('[data-role="count"]');
    const load = async () => {
      const model = this.getModel();
      if (!model || !this.target) return;
      const depth = this.depths[direction];
      replaceChildren(content, message('info', `Loading ${title.toLowerCase()}...`));
      try {
        const resp = direction === 'callers'
          ? await model.callers(this.target.symbol, depth)
          : await model.callees(this.target.symbol, depth);
        if (seq !== this.seq) return;
        this.absorbRootRef(resp);
        const { node, count } = this.renderCallTree(resp, direction, depth);
        countEl.textContent = count ? `(${count.toLocaleString()})` : '(0)';
        replaceChildren(content, node, unresolvedList(resp && resp.unresolved_sites), notesList(resp && resp.notes));
      } catch (err) {
        if (seq !== this.seq) return;
        countEl.textContent = '';
        replaceChildren(content, message('error', describeQueryError(err)));
      }
    };
    return { el, load };
  }

  /** Remembers the owner type of the selected function (for "Show in class diagram"). */
  absorbRootRef(resp) {
    if (!resp || !this.target || !resp.functions || typeof resp.functions !== 'object') return;
    const ref = resp.functions[resp.root] || resp.functions[this.target.symbol];
    if (ref && typeof ref === 'object') {
      if (typeof ref.owner === 'string' && ref.owner) this.target.owner = ref.owner;
      if (!this.target.qualifiedName && typeof ref.qualified_name === 'string') this.target.qualifiedName = ref.qualified_name;
    }
  }

  renderCallTree(resp, direction, depth) {
    const fns = resp && resp.functions && typeof resp.functions === 'object' ? resp.functions : {};
    const root = resp && typeof resp.root === 'string' ? resp.root : this.target.symbol;
    const adjacency = new Map();
    (Array.isArray(resp && resp.edges) ? resp.edges : []).forEach(e => {
      if (!e || typeof e !== 'object') return;
      const from = direction === 'callees' ? e.caller : e.callee;
      const to = direction === 'callees' ? e.callee : e.caller;
      if (typeof from !== 'string' || typeof to !== 'string') return;
      if (!adjacency.has(from)) adjacency.set(from, []);
      adjacency.get(from).push({ symbol: to, edge: e });
    });
    const labelOf = (sym) => {
      const ref = fns[sym];
      if (ref && typeof ref === 'object') return String(ref.name || shortName(ref.qualified_name) || sym);
      return shortName(sym) || sym;
    };
    adjacency.forEach(list => list.sort((a, b) => labelOf(a.symbol).localeCompare(labelOf(b.symbol))));
    const direct = adjacency.get(root) || [];
    if (direct.length === 0) {
      const none = direction === 'callers' ? 'No callers were found.' : 'No callees were found.';
      return { node: message('info', none), count: 0 };
    }
    let budget = MAX_TREE_ITEMS;
    let truncated = 0;
    const seen = new Set();
    const build = (sym, level, path) => {
      const children = adjacency.get(sym) || [];
      const ul = h('ul', { className: level === 1 ? 'rel-tree' : 'rel-tree-children' });
      for (const child of children) {
        if (budget <= 0) {
          truncated++;
          continue;
        }
        budget--;
        seen.add(child.symbol);
        const recursive = path.has(child.symbol);
        const grand = !recursive && level < depth ? (adjacency.get(child.symbol) || []) : [];
        const li = h('li', { className: 'rel-tree-item' });
        let childList = null;
        let toggle;
        if (grand.length) {
          toggle = h('button', {
            type: 'button',
            className: 'rel-tree-toggle',
            'aria-expanded': 'true',
            'aria-label': `Collapse ${labelOf(child.symbol)}`
          }, icon('chevronDown'));
        } else {
          toggle = h('span', { className: 'rel-tree-spacer', 'aria-hidden': 'true' });
        }
        li.appendChild(toggle);
        li.appendChild(this.functionEntry(child.symbol, fns[child.symbol], child.edge, recursive, direction));
        if (grand.length) {
          const nextPath = new Set(path);
          nextPath.add(child.symbol);
          childList = build(child.symbol, level + 1, nextPath);
          li.appendChild(childList);
          toggle.addEventListener('click', () => {
            const open = toggle.getAttribute('aria-expanded') !== 'true';
            toggle.setAttribute('aria-expanded', String(open));
            toggle.setAttribute('aria-label', `${open ? 'Collapse' : 'Expand'} ${labelOf(child.symbol)}`);
            replaceChildren(toggle, icon(open ? 'chevronDown' : 'chevronRight'));
            childList.hidden = !open;
          });
        }
        ul.appendChild(li);
      }
      return ul;
    };
    const tree = build(root, 1, new Set([root]));
    if (truncated) tree.appendChild(h('li', { className: 'rel-more' }, `${truncated.toLocaleString()} more not shown; lower the depth or open the call graph.`));
    return { node: tree, count: seen.size };
  }

  functionEntry(symbol, ref, edge, recursive, direction) {
    const r = ref && typeof ref === 'object' ? ref : {};
    const name = String(r.name || shortName(r.qualified_name) || shortName(symbol) || symbol);
    const qualified = typeof r.qualified_name === 'string' ? r.qualified_name : symbol;
    const sites = edge && Array.isArray(edge.sites) ? edge.sites : [];
    const badges = [];
    if (edge && typeof edge.dispatch === 'string' && edge.dispatch !== 'static') badges.push(edge.dispatch);
    if (edge && typeof edge.resolution === 'string' && edge.resolution !== 'exact') badges.push(edge.resolution);
    const titleParts = [qualified];
    if (typeof r.file === 'string') titleParts.push(`Declared at ${r.file}${Number.isInteger(r.line) ? `:${r.line}` : ''}`);
    if (sites.length) titleParts.push(`Call sites: ${sites.slice(0, 8).map(siteText).join(', ')}${sites.length > 8 ? ', ...' : ''}`);
    if (edge && edge.resolution) titleParts.push(`Resolution: ${edge.resolution}${edge.dispatch ? `, dispatch: ${edge.dispatch}` : ''}`);
    return h('button', {
      type: 'button',
      className: 'rel-entry',
      title: titleParts.join('\n'),
      onClick: () => this.onNavigate({ symbol, qualifiedName: qualified, name, role: 'function' })
    },
      h('span', { className: 'rel-entry-name' }, name),
      r.owner ? h('span', { className: 'rel-entry-owner' }, shortName(r.owner)) : null,
      sites.length > 1 ? h('span', { className: 'rel-entry-count' }, `${sites.length} sites`) : null,
      badges.map(b => h('span', { className: 'rel-badge' }, b)),
      recursive ? h('span', { className: 'rel-badge rel-badge-cycle' }, direction === 'callers' ? 'recursive caller' : 'recursive') : null
    );
  }

  // ── Dependencies / dependents ──

  lazySection(title, hint, seq, loader) {
    const body = h('div', { className: 'rel-section-body' }, message('info', 'Open to load.'));
    let loaded = false;
    const el = h('details', { className: 'rel-section' },
      h('summary', { title: hint }, h('span', { className: 'rel-section-title' }, title)),
      body
    );
    el.addEventListener('toggle', () => {
      if (el.open && !loaded && seq === this.seq) {
        loaded = true;
        loader(body);
      }
    });
    return el;
  }

  async loadDependencies(body, seq) {
    const model = this.getModel();
    if (!model || !this.target) return;
    replaceChildren(body, message('info', 'Loading dependencies...'));
    try {
      const resp = await model.dependencies(this.target.symbol);
      if (seq !== this.seq) return;
      const groups = [
        this.entryGroup('Calls', resp.calls, 'function', 'calls'),
        this.entryGroup('Reads', resp.reads, 'field', 'reads'),
        this.entryGroup('Writes', resp.writes, 'field', 'writes'),
        this.entryGroup('Types', resp.types, 'type', null),
        this.packageGroup(resp.packages)
      ].filter(Boolean);
      replaceChildren(body, groups.length ? groups : message('info', 'No dependencies were derived for this element.'), notesList(resp.notes));
    } catch (err) {
      if (seq === this.seq) replaceChildren(body, message('error', describeQueryError(err)));
    }
  }

  async loadDependents(body, seq) {
    const model = this.getModel();
    if (!model || !this.target) return;
    replaceChildren(body, message('info', 'Loading dependents...'));
    try {
      const resp = await model.dependents(this.target.symbol);
      if (seq !== this.seq) return;
      const groups = [
        this.entryGroup('Callers', resp.callers, 'function', 'calls'),
        this.entryGroup('Readers', resp.readers, 'function', 'reads'),
        this.entryGroup('Writers', resp.writers, 'function', 'writes'),
        this.entryGroup('Type users', resp.type_users, 'any', null),
        this.entryGroup('Subtypes', resp.subtypes, 'type', null),
        this.entryGroup('Overrides', resp.overrides, 'function', null)
      ].filter(Boolean);
      replaceChildren(body, groups.length ? groups : message('info', 'Nothing in the analysis depends on this element.'), notesList(resp.notes));
    } catch (err) {
      if (seq === this.seq) replaceChildren(body, message('error', describeQueryError(err)));
    }
  }

  entryGroup(title, entries, role, countWord) {
    const list = Array.isArray(entries) ? entries.filter(e => e && typeof e === 'object' && (typeof e.symbol === 'string' || typeof e.symbol === 'number')) : [];
    if (!list.length) return null;
    const shown = list.slice(0, MAX_LIST_ITEMS);
    return h('div', { className: 'rel-group' },
      h('div', { className: 'rel-subhead' }, `${title} (${list.length.toLocaleString()})`),
      h('ul', { className: 'rel-list' },
        shown.map(e => {
          const symbol = String(e.symbol);
          const qualified = typeof e.qualified_name === 'string' && e.qualified_name ? e.qualified_name : symbol;
          const name = typeof e.name === 'string' && e.name ? e.name : shortName(qualified);
          const relations = Array.isArray(e.relations) ? e.relations.filter(r => typeof r === 'string') : [];
          return h('li', null, h('button', {
            type: 'button',
            className: 'rel-entry',
            title: qualified,
            onClick: () => this.onNavigate({ symbol, qualifiedName: qualified, name, role })
          },
            h('span', { className: 'rel-entry-name' }, name),
            qualified !== name ? h('span', { className: 'rel-entry-owner' }, qualified) : null,
            Number.isInteger(e.count) && e.count > 1 && countWord ? h('span', { className: 'rel-entry-count' }, `${e.count} ${countWord}`) : null,
            relations.map(r => h('span', { className: 'rel-badge' }, r.replace(/_/g, ' ')))
          ));
        }),
        list.length > shown.length ? h('li', { className: 'rel-more' }, `and ${(list.length - shown.length).toLocaleString()} more`) : null
      )
    );
  }

  packageGroup(packages) {
    const list = Array.isArray(packages) ? packages.filter(p => typeof p === 'string' && p) : [];
    if (!list.length) return null;
    return h('div', { className: 'rel-group' },
      h('div', { className: 'rel-subhead' }, `Packages (${list.length.toLocaleString()})`),
      h('ul', { className: 'rel-list rel-list-plain' }, list.slice(0, MAX_LIST_ITEMS).map(p => h('li', null, p)))
    );
  }

  // ── Impact ──

  renderImpact(resp, highlighted) {
    const raw = resp && Array.isArray(resp.impacted) ? resp.impacted : [];
    const entries = raw.filter(e => e && typeof e === 'object' && e.reason !== 'changed' && e.reason !== 'Changed');
    const wrap = h('div', { className: 'rel-impact' },
      h('div', { className: 'rel-subhead' }, `Impact of a change: ${plural(entries.length, 'affected element')}${highlighted ? `, ${highlighted} highlighted on the canvas` : ''}`)
    );
    if (!entries.length) {
      wrap.appendChild(message('info', 'No other element is affected by a change to this one.'));
      return wrap;
    }
    const byReason = new Map();
    entries.forEach(e => {
      const reason = String(e.reason || 'affected').replace(/([a-z])([A-Z])/g, '$1 $2').replace(/_/g, ' ').toLowerCase();
      if (!byReason.has(reason)) byReason.set(reason, []);
      byReason.get(reason).push(e);
    });
    byReason.forEach((list, reason) => {
      list.sort((a, b) => (a.depth || 0) - (b.depth || 0));
      wrap.appendChild(h('div', { className: 'rel-group' },
        h('div', { className: 'rel-subhead' }, `${reason} (${list.length.toLocaleString()})`),
        h('ul', { className: 'rel-list' }, list.slice(0, MAX_LIST_ITEMS).map(e => {
          const spec = typeof e.symbol === 'string' && e.symbol ? e.symbol : (Number.isInteger(e.sym_id) ? String(e.sym_id) : null);
          const qualified = typeof e.qualified_name === 'string' && e.qualified_name ? e.qualified_name : null;
          const label = qualified ? shortName(qualified) : (typeof e.name === 'string' && e.name ? e.name : `symbol #${spec}`);
          const depthText = Number.isInteger(e.depth) ? `depth ${e.depth}` : null;
          if (!spec) return h('li', null, label);
          return h('li', null, h('button', {
            type: 'button',
            className: 'rel-entry',
            title: qualified || `Symbol ${spec}`,
            onClick: () => this.onNavigate({ symbol: spec, qualifiedName: qualified, name: label, role: 'any' })
          },
            h('span', { className: 'rel-entry-name' }, label),
            qualified && qualified !== label ? h('span', { className: 'rel-entry-owner' }, qualified) : null,
            depthText ? h('span', { className: 'rel-entry-count' }, depthText) : null
          ));
        }))
      ));
    });
    return wrap;
  }
}
