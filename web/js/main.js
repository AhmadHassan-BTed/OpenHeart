/**
 * OpenHeart Studio Bootstrap
 * Externalised from index.html so the page runs under a strict Content-Security-Policy
 * (no inline script). Every dynamic string reaches the DOM through textContent / h();
 * nothing here assigns innerHTML.
 */

import { icon } from './icons.js';
import { h, replaceChildren, strong, safeColor, safeFileStem } from './dom.js';
import { InteractiveGraphCanvas } from './graph-canvas.js';
import { SourceEditorModule } from './editor.js';
import { FileTreeExplorer } from './file-tree.js';
import { GitHubEngine, GitHubError, INGESTION_LIMITS } from './github-engine.js';
import { initTheme, toggleTheme, onThemeChange, fetchManifest, getManifestError, isDarkMode } from './themes/index.js';
import { studioStore } from './studio-store.js';
import { localStore, sessionStore } from './storage.js';
import { validateDiagramState } from './graph-ir-validate.js';
import { hashGraphIr, shortHash } from './analysis/ir-hash.js';
import { toDot, toGraphMl } from './graph-export.js';

const ENGINE_HEALTH_TIMEOUT_MS = 2500;
const ENGINE_ANALYZE_TIMEOUT_MS = 10 * 60 * 1000;
const ENGINE_POLL_INTERVAL_MS = 1500;
const MAX_STATE_FILE_BYTES = 64 * 1024 * 1024;
const HISTORY_KEY = 'openheart_project_history';
const TOKEN_KEY = 'openheart_github_token';
const DEFAULT_HUD_HINT = 'Mouse wheel to Zoom · Drag to Pan · Shift+Wheel to Pan H · Hover to trace path';

const PRESETS = [
  { id: 'patterns', label: 'iluwatar/java-design-patterns', url: 'https://github.com/iluwatar/java-design-patterns' },
  { id: 'petclinic', label: 'spring-projects/spring-petclinic', url: 'https://github.com/spring-projects/spring-petclinic' }
];

const $ = (id) => document.getElementById(id);

// ── Clickjacking defence (WSEC-06) ──
// frame-ancestors cannot be delivered through a <meta> CSP and GitHub Pages sends no
// X-Frame-Options, so the studio refuses to initialise when framed.
function isFramed() {
  try {
    return window.top !== window.self;
  } catch (_) {
    return true;
  }
}

function renderFramedNotice() {
  replaceChildren(document.body,
    h('main', { className: 'framed-notice' },
      h('h1', null, 'OpenHeart Studio cannot run inside a frame'),
      h('p', null, 'For your safety the studio only runs as a top-level page.'),
      h('a', { href: window.location.href, target: '_top', rel: 'noopener noreferrer' }, 'Open the studio directly')
    )
  );
}

function start() {
  if (isFramed()) {
    renderFramedNotice();
    return;
  }
  boot().catch(err => {
    console.error('[OpenHeart Studio] Startup failed:', err);
    const hud = $('hud-status-text');
    if (hud) replaceChildren(hud, h('span', { className: 'hud-error' }, `Startup failed: ${(err && err.message) || err}`));
  });
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', start, { once: true });
} else {
  start();
}

async function boot() {
  const hudText = $('hud-status-text');
  const liveRegion = $('studio-live-region');

  const setHud = (...parts) => replaceChildren(hudText, ...parts);
  const setHudError = (message) => setHud(h('span', { className: 'hud-error' }, message));
  const announce = (message) => {
    if (!liveRegion) return;
    liveRegion.textContent = '';
    requestAnimationFrame(() => { liveRegion.textContent = message; });
  };

  // ── Theme first, so the first render already uses the right palette (WEB-07) ──
  initTheme();
  const btnTheme = $('btn-theme-toggle');
  const syncThemeButton = (isDark) => {
    const iconSpan = $('theme-toggle-icon');
    const labelSpan = $('theme-toggle-label');
    if (iconSpan) replaceChildren(iconSpan, icon(isDark ? 'sun' : 'moon'));
    if (labelSpan) labelSpan.textContent = isDark ? 'Light' : 'Dark';
    if (btnTheme) btnTheme.setAttribute('aria-pressed', String(isDark));
  };
  syncThemeButton(isDarkMode());

  if (typeof cytoscape === 'undefined') {
    setHudError('The diagram renderer (Cytoscape.js) failed to load from cdnjs. Check your network or content blockers and reload.');
    announce('The diagram renderer failed to load.');
  }

  const canvas = new InteractiveGraphCanvas('interactive-canvas');
  const editor = new SourceEditorModule('monaco-container');
  canvas.setAnnounceCallback(announce);

  // ── Diagram catalog ──
  const manifest = await fetchManifest();
  const diagramIds = new Set();
  const diagramNames = new Map();
  if (manifest) {
    manifest.categories.forEach(cat => cat.diagrams.forEach(d => {
      diagramIds.add(d.id);
      diagramNames.set(d.id, d.name);
    }));
  }
  const diagramName = (id) => diagramNames.get(id) || id;
  const savedGraph = localStore.get('openheart_active_graph', 'class');
  canvas.currentGraphType = diagramIds.has(savedGraph) ? savedGraph : 'class';

  const sidebarNav = $('sidebar-diagram-categories');
  const IN_BROWSER_VIEWS = new Set(['class', 'package']);

  function setActiveSidebar(graphType) {
    document.querySelectorAll('.sidebar-tab-btn').forEach(b => {
      const active = b.dataset.graphType === graphType;
      b.classList.toggle('active', active);
      b.setAttribute('aria-pressed', String(active));
    });
  }

  function markViewAvailability() {
    const inBrowser = studioStore.isInBrowserModel();
    document.querySelectorAll('.sidebar-tab-btn').forEach(b => {
      const unavailable = inBrowser && !IN_BROWSER_VIEWS.has(b.dataset.graphType);
      b.classList.toggle('view-unavailable', unavailable);
      if (unavailable) b.title = 'Not available from in-browser analysis: this view requires the OpenHeart Rust engine.';
      else b.removeAttribute('title');
    });
  }

  async function selectDiagram(graphType) {
    setActiveSidebar(graphType);
    localStore.set('openheart_active_graph', graphType);
    await canvas.renderGraph(graphType);
  }

  function renderSidebar() {
    if (!sidebarNav) return;
    if (!manifest) {
      replaceChildren(sidebarNav,
        h('div', { className: 'sidebar-error', role: 'alert' },
          getManifestError() || 'The diagram catalog (diagrams/manifest.json) could not be loaded.')
      );
      return;
    }
    replaceChildren(sidebarNav);
    manifest.categories.forEach((cat, ci) => {
      const headingId = `sidebar-cat-${ci}`;
      sidebarNav.appendChild(h('div', {
        className: 'sidebar-category-title',
        id: headingId,
        style: ci > 0 ? { 'margin-top': '18px' } : null
      }, h('span', null, cat.title), h('span', { className: 'phase-chip' }, cat.badge || '')));
      const ul = h('ul', { className: 'sidebar-nav-list', 'aria-labelledby': headingId });
      cat.diagrams.forEach(diag => {
        const active = diag.id === canvas.currentGraphType;
        ul.appendChild(h('li', null,
          h('button', {
            type: 'button',
            className: `sidebar-tab-btn ${active ? 'active' : ''}`,
            dataset: { graphType: diag.id },
            'aria-pressed': String(active),
            onClick: () => { selectDiagram(diag.id); }
          }, h('span', null, `${diag.number || ''}. ${diag.name}`), h('span', { className: 'phase-chip' }, diag.phase || ''))
        ));
      });
      sidebarNav.appendChild(ul);
    });
    markViewAvailability();
  }
  renderSidebar();

  // ── Breadcrumbs ──
  function updateBreadcrumbs(selectedFile) {
    const rootLabel = $('breadcrumb-root-label');
    const pkgEl = $('breadcrumb-package');
    const fileEl = $('breadcrumb-filename');
    const divPkg = $('breadcrumb-divider-pkg');
    const divFile = $('breadcrumb-divider-file');
    const editorFilename = $('editor-filename');
    if (rootLabel) rootLabel.textContent = studioStore.codebaseName;
    if (!selectedFile) {
      if (pkgEl) pkgEl.style.display = 'none';
      if (divPkg) divPkg.style.display = 'none';
      if (divFile) divFile.style.display = 'none';
      if (fileEl) fileEl.textContent = 'No source';
      if (editorFilename) editorFilename.textContent = 'No source';
      return;
    }
    const normalized = selectedFile.replace(/\\/g, '/').replace(/^\/+/, '');
    const parts = normalized.split('/').filter(Boolean);
    const baseFileName = parts.length ? parts[parts.length - 1] : normalized;
    const folderText = parts.slice(0, -1).join(' / ');
    if (editorFilename) { editorFilename.textContent = baseFileName; editorFilename.title = normalized; }
    if (fileEl) { fileEl.textContent = baseFileName; fileEl.title = normalized; }
    if (pkgEl) {
      pkgEl.textContent = folderText;
      pkgEl.style.display = folderText ? '' : 'none';
    }
    if (divPkg) divPkg.style.display = folderText ? '' : 'none';
    if (divFile) divFile.style.display = '';
  }

  // ── Notices (IR diagnostics, approximation disclosure, engine errors) ──
  const noticeEl = $('canvas-notice');
  let dismissedNoticeKey = null;
  function showNotice(level, title, items, key = null) {
    if (!noticeEl) return;
    if (key && key === dismissedNoticeKey) return;
    const list = (items || []).filter(Boolean);
    if (!title && list.length === 0) {
      noticeEl.hidden = true;
      replaceChildren(noticeEl);
      return;
    }
    noticeEl.hidden = false;
    noticeEl.dataset.level = level;
    replaceChildren(noticeEl,
      h('div', { className: 'canvas-notice-body' },
        title ? h('strong', null, title) : null,
        list.length ? h('ul', null, list.map(item => h('li', null, item))) : null
      ),
      h('button', {
        type: 'button',
        className: 'canvas-notice-dismiss',
        'aria-label': 'Dismiss notice',
        onClick: () => {
          dismissedNoticeKey = key;
          noticeEl.hidden = true;
        }
      }, icon('close'))
    );
  }

  function describeSkipped(skipped) {
    return Object.entries(skipped || {}).filter(([, n]) => n > 0).map(([k, n]) => `${n.toLocaleString()} ${k.replace(/_/g, ' ')}`).join(', ');
  }

  function describeFailures(failed) {
    return Object.entries(failed || {}).filter(([, n]) => n > 0).map(([k, n]) => `${n.toLocaleString()} ${k.replace(/_/g, ' ')}`).join(', ');
  }

  function inBrowserDisclosure(stats) {
    if (!stats) return [];
    const lines = [];
    lines.push(`Parsed ${stats.files_parsed.toLocaleString()} of ${stats.files_eligible.toLocaleString()} eligible source files (${stats.files_listed.toLocaleString()} files listed)${stats.sample_budget && stats.sample_budget.hit ? `; the in-browser budget is ${stats.sample_budget.max_files.toLocaleString()} files, sampled across ${stats.module_groups.toLocaleString()} top-level modules` : ''}.`);
    const skipped = describeSkipped(stats.files_skipped);
    if (skipped) lines.push(`Skipped before parsing: ${skipped}.`);
    const failed = describeFailures(stats.files_failed);
    if (failed) lines.push(`Not parsed: ${failed}.`);
    if (stats.tree_truncated) lines.push(stats.tree_walk_incomplete ? 'GitHub truncated the file listing and the directory walk hit its request budget; some directories were never listed.' : 'GitHub truncated the recursive listing; directories were walked individually.');
    if (stats.unresolved_references || stats.ambiguous_references) {
      lines.push(`${(stats.unresolved_references || 0).toLocaleString()} type references could not be resolved and ${(stats.ambiguous_references || 0).toLocaleString()} were ambiguous; those relations are omitted.`);
    }
    if (stats.rate_limit && stats.rate_limit.remaining !== null && stats.rate_limit.remaining !== undefined) {
      lines.push(`GitHub API requests used: ${stats.api_requests}; remaining this hour: ${stats.rate_limit.remaining}.`);
    }
    lines.push('Only class and package views are derived in the browser; other views need the Rust engine.');
    return lines;
  }

  // ── Provenance badge & footer ──
  const hashBadge = $('header-hash-badge');
  const badgeBox = hashBadge ? hashBadge.closest('.status-pill-badge') : null;
  const footerProvenance = $('footer-provenance');
  let provenanceSeq = 0;

  async function updateProvenance(meta) {
    const seq = ++provenanceSeq;
    const session = studioStore.get();
    const kind = session.source.kind;
    let label;
    let detail;
    let state;
    if (kind === 'github' || kind === 'folder' || kind === 'zip') {
      const st = session.stats || {};
      label = `In-browser sample · ${st.files_parsed ?? '?'}/${st.files_eligible ?? '?'} files`;
      detail = `Approximate in-browser declaration extraction of ${session.source.name}.`;
      state = 'approximate';
    } else if (kind === 'engine') {
      label = `Rust engine · ${session.engine && session.engine.session_id ? session.engine.session_id : 'session'}`;
      detail = `Results from the OpenHeart analysis server for ${session.source.name}.`;
      state = 'engine';
    } else {
      label = 'Bundled sample diagrams';
      detail = 'Static sample files shipped in web/diagrams; they were not generated from your code.';
      state = 'bundled';
    }
    if (meta && meta.origin === 'state-file') {
      label = 'Saved state file';
      detail = 'A diagram snapshot loaded from a .openheart.json file.';
      state = 'snapshot';
    }
    let hash = null;
    let hashReason = null;
    if (meta && meta.ir) {
      const result = (kind === 'github' || kind === 'folder' || kind === 'zip') && session.ir === meta.ir && session.provenance && session.provenance.content_hash
        ? { value: session.provenance.content_hash, reason: null }
        : await hashGraphIr(meta.ir);
      if (seq !== provenanceSeq) return;
      hash = result.value;
      hashReason = result.reason;
    }
    if (hashBadge) {
      hashBadge.textContent = hash ? `${label} · ${shortHash(hash)}` : label;
      const titleParts = [detail];
      if (hash) titleParts.push(`Graph IR content hash: ${hash}`);
      else if (hashReason) titleParts.push(`No content hash: ${hashReason}`);
      else titleParts.push('This view has no Graph IR to hash.');
      if (badgeBox) {
        badgeBox.title = titleParts.join('\n');
        badgeBox.dataset.state = state;
      }
    }
    if (footerProvenance) {
      const coverage = session.provenance && session.provenance.coverage ? ` · ${session.provenance.coverage}` : '';
      footerProvenance.textContent = `Analysis source: ${session.provenance ? session.provenance.engine : label}${coverage}`;
    }
  }

  // ── File tree ──
  const fileTree = new FileTreeExplorer('file-tree-container', (selectedFile, fileEntry) => {
    const nodeData = (fileEntry && fileEntry.data) || canvas.getNodeDataById(fileEntry && fileEntry.nodeId);
    const lines = nodeData && nodeData.anchor && Number.isInteger(nodeData.anchor.start_line)
      ? [nodeData.anchor.start_line]
      : ((nodeData && nodeData.lines) || []);
    editor.loadFile(selectedFile, nodeData, lines);
    if (fileEntry && fileEntry.nodeId) canvas.focusNodeById(fileEntry.nodeId, { inspect: false });
    updateBreadcrumbs(selectedFile);
    if (nodeData) $('prop-kind').textContent = kindLabel(nodeData.kind);
  });

  function kindLabel(kind) {
    const kinds = (manifest && manifest.node_kinds) || {};
    const conf = kinds[kind];
    return String(conf && conf.label ? conf.label : (kind || 'node')).toUpperCase();
  }

  canvas.setRenderCompleteCallback((elements, meta) => {
    fileTree.updateFromElements(elements);
    const countBadge = $('explorer-file-count');
    if (countBadge) countBadge.textContent = String(fileTree.fileCount || 0);
    updateBreadcrumbs(null);
    setActiveSidebar(meta.graphType);

    const session = studioStore.get();
    const notices = [];
    let level = 'info';
    let title = null;
    if (meta.warnings && meta.warnings.length) {
      level = 'warning';
      title = 'Diagram data was repaired before rendering';
      notices.push(...meta.warnings);
    }
    if (studioStore.isInBrowserModel() && meta.origin === 'in-browser') {
      title = title || 'Approximate in-browser analysis';
      notices.push(...inBrowserDisclosure(session.stats));
    }
    if (session.source.kind === 'engine' && session.engine && session.engine.errors && session.engine.errors.length) {
      level = 'warning';
      title = title || 'The analysis server reported errors';
      notices.push(...session.engine.errors.map(e => `Engine: ${e}`));
    }
    if (meta.origin === 'bundled') {
      title = title || 'Bundled sample diagrams';
      notices.push('These are static sample files shipped with the studio, not an analysis of your code. Analyze a repository, folder or ZIP to replace them.');
    }
    showNotice(level, title, notices, `${session.source.kind}:${session.source.name}:${meta.graphType}:${notices.length}`);

    if (meta.emptyReason) {
      setHud(strong(diagramName(meta.graphType)), `: ${meta.emptyReason}`);
    }
    updateProvenance(meta);
    markViewAvailability();
  });

  // ── Inspector ──
  const setProp = (id, text) => { const el = $(id); if (el) el.textContent = text; };
  canvas.setNodeSelectCallback((nodeData) => {
    const file = nodeData.file || null;
    updateBreadcrumbs(file);
    if (file) fileTree.selectFile(file, nodeData.id);

    setProp('inspect-headline', nodeData.name || nodeData.rawName || nodeData.id);
    setProp('inspect-meta', `${nodeData.qualifiedName || nodeData.id} · ${nodeData.stereotype || nodeData.kind || 'node'}`);
    setProp('prop-kind', kindLabel(nodeData.kind));
    const anchor = nodeData.anchor;
    if (anchor && anchor.path && Number.isInteger(anchor.start_line)) {
      setProp('prop-anchor', `${anchor.path}:${anchor.start_line}:${anchor.start_col || 1}${Number.isInteger(anchor.end_line) ? ` – ${anchor.end_line}:${anchor.end_col || 1}` : ''}`);
    } else if (file) {
      setProp('prop-anchor', Array.isArray(nodeData.lines) && nodeData.lines.length ? `${file}:${nodeData.lines[0]}` : `${file} (no line recorded)`);
    } else {
      setProp('prop-anchor', 'Not recorded in the diagram data');
    }
    const parentData = nodeData.parent ? canvas.getNodeDataById(nodeData.parent) : null;
    setProp('prop-container', parentData ? (parentData.rawName || parentData.name || parentData.id) : 'None');
    setProp('prop-sat', Number.isFinite(nodeData.bddSatCount) ? `${nodeData.bddSatCount} satisfying path${nodeData.bddSatCount === 1 ? '' : 's'}` : 'Not available for this node');
    setProp('prop-idom', Number.isFinite(nodeData.idomRank) ? `Rank #${nodeData.idomRank}` : 'Not available for this node');
    if (nodeData.branchPredicate) setProp('prop-predicate', nodeData.branchPredicate);
    else if (Number.isFinite(nodeData.cyclomatic)) setProp('prop-predicate', `v(G) = ${nodeData.cyclomatic}`);
    else setProp('prop-predicate', 'Not available for this node');

    const lines = anchor && Number.isInteger(anchor.start_line) ? [anchor.start_line] : (nodeData.lines || []);
    editor.loadFile(file, nodeData, lines);
  });

  canvas.setNodeHoverCallback((nodeData) => {
    const title = nodeData.name || nodeData.rawName || nodeData.id;
    setHud('Path Illuminated: ', strong(title));
  });

  // ── Relationship layers ──
  const layersPanel = $('canvas-layers-panel');
  const layersContainer = $('layers-checklist-container');
  const layersTotalCount = $('layers-total-count');
  function renderLayers(activeKinds) {
    if (!layersContainer) return;
    replaceChildren(layersContainer);
    if (!activeKinds || activeKinds.length === 0) {
      if (layersPanel) layersPanel.style.display = 'none';
      return;
    }
    if (layersPanel) layersPanel.style.display = 'flex';
    let totalEdges = 0;
    const relTypes = (manifest && manifest.relationship_types) || {};
    const dark = isDarkMode();
    activeKinds.forEach(item => {
      totalEdges += item.count;
      const conf = Object.prototype.hasOwnProperty.call(relTypes, item.kind) ? relTypes[item.kind] : null;
      const color = safeColor(conf ? (dark ? conf.color_dark : conf.color_light) : null, dark ? '#94A3B8' : '#334155');
      const labelText = conf && typeof conf.label === 'string' ? conf.label : item.kind;
      const iconText = conf && typeof conf.icon === 'string' ? conf.icon.slice(0, 3) : '→';
      const checkbox = h('input', { type: 'checkbox', style: { 'accent-color': color } });
      checkbox.checked = item.visible;
      const chip = h('label', {
        className: `layer-chip-label ${item.visible ? '' : 'unchecked'}`,
        dataset: { kind: item.kind },
        style: { 'border-left': `3px solid ${color}` }
      },
        checkbox,
        h('span', { className: 'layer-chip-text' },
          h('span', { className: 'layer-chip-dot', style: { 'background-color': color, 'box-shadow': `0 0 5px ${color}` }, 'aria-hidden': 'true' }),
          h('strong', { className: 'layer-chip-icon', style: { color }, 'aria-hidden': 'true' }, iconText),
          h('span', null, labelText),
          h('span', { className: 'layer-chip-count' }, `(${item.count})`)
        )
      );
      checkbox.addEventListener('change', () => {
        canvas.setEdgeFilter(item.kind, checkbox.checked);
        chip.classList.toggle('unchecked', !checkbox.checked);
      });
      layersContainer.appendChild(chip);
    });
    if (layersTotalCount) layersTotalCount.textContent = `${totalEdges} RELATIONS`;
  }
  canvas.setLayersUpdateCallback(renderLayers);

  const setAllLayers = (visible) => {
    document.querySelectorAll('#layers-checklist-container .layer-chip-label').forEach(chip => {
      const cb = chip.querySelector('input[type="checkbox"]');
      if (cb) cb.checked = visible;
      chip.classList.toggle('unchecked', !visible);
      canvas.setEdgeFilter(chip.dataset.kind, visible);
    });
  };
  $('btn-layers-all').addEventListener('click', () => setAllLayers(true));
  $('btn-layers-none').addEventListener('click', () => setAllLayers(false));

  onThemeChange((theme, isDark) => {
    syncThemeButton(isDark);
    renderLayers(canvas.getActiveEdgeKinds());
  });
  if (btnTheme) btnTheme.addEventListener('click', () => toggleTheme());

  // ── Pan sensitivity ──
  const panSlider = $('pan-sensitivity-slider');
  const panValLabel = $('pan-sensitivity-val');
  if (panSlider && panValLabel) {
    const savedPan = Math.min(0.5, Math.max(0.01, localStore.getNumber('openheart_pan_sensitivity', 0.10)));
    panSlider.value = String(savedPan);
    panValLabel.textContent = `${Math.round(savedPan * 100)}%`;
    canvas.setPanSensitivity(savedPan);
    panSlider.addEventListener('input', (e) => {
      const val = parseFloat(e.target.value);
      panValLabel.textContent = `${Math.round(val * 100)}%`;
      canvas.setPanSensitivity(val);
    });
  }

  // ── Inspector tabs (WAI-ARIA tabs) ──
  const tabs = [
    { tab: $('tab-btn-explorer'), pane: $('pane-explorer') },
    { tab: $('tab-btn-source'), pane: $('pane-source') }
  ];
  const activateTab = (index, focus = false) => {
    tabs.forEach((t, i) => {
      const active = i === index;
      t.tab.classList.toggle('active', active);
      t.tab.setAttribute('aria-selected', String(active));
      t.tab.setAttribute('tabindex', active ? '0' : '-1');
      t.pane.style.display = active ? 'block' : 'none';
      t.pane.hidden = !active;
    });
    if (focus) tabs[index].tab.focus();
  };
  tabs.forEach((t, i) => {
    t.tab.addEventListener('click', () => activateTab(i));
    t.tab.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        e.preventDefault();
        activateTab((i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length, true);
      } else if (e.key === 'Home') {
        e.preventDefault();
        activateTab(0, true);
      } else if (e.key === 'End') {
        e.preventDefault();
        activateTab(tabs.length - 1, true);
      }
    });
  });
  activateTab(0);

  // ── Session restore ──
  const restore = studioStore.restore();
  if (restore.restored) {
    editor.setMultipleSources(studioStore.getFiles());
  }
  syncRepoInput();

  canvas.init();
  editor.init();

  if (restore.restored) {
    setHud('Restored ', strong(studioStore.codebaseName), ' from this tab\'s session · Ready');
  }
  if (restore.warnings && restore.warnings.length) {
    showNotice('warning', 'The restored session was repaired', restore.warnings);
  }

  // ── Zoom controls ──
  $('btn-zoom-in').addEventListener('click', () => canvas.zoomIn());
  $('btn-zoom-out').addEventListener('click', () => canvas.zoomOut());
  $('btn-zoom-reset').addEventListener('click', () => canvas.resetView());

  // ── Analysis progress & cancellation ──
  const btnImport = $('btn-import-repo');
  const btnCancel = $('btn-cancel-analysis');
  const progressTrack = $('clone-progress-bar-container');
  const progressFill = $('clone-progress-fill');
  const canvasCard = document.querySelector('.canvas-card');
  let activeAnalysis = null;

  function beginProgress() {
    if (activeAnalysis) activeAnalysis.controller.abort();
    const controller = new AbortController();
    activeAnalysis = { controller };
    if (btnImport) {
      btnImport.disabled = true;
      replaceChildren(btnImport, h('span', { className: 'btn-spinner', 'aria-hidden': 'true' }), ' Working');
    }
    if (btnCancel) btnCancel.hidden = false;
    if (progressTrack) {
      progressTrack.classList.add('active');
      progressTrack.setAttribute('aria-hidden', 'false');
    }
    if (progressFill) progressFill.style.width = '4%';
    if (canvasCard) canvasCard.setAttribute('aria-busy', 'true');
    return controller;
  }

  function setProgress(pct, message, indeterminate = false) {
    if (progressTrack) progressTrack.classList.toggle('indeterminate', indeterminate);
    if (progressFill && !indeterminate) progressFill.style.width = `${Math.max(4, Math.min(100, pct))}%`;
    if (progressTrack && !indeterminate) progressTrack.setAttribute('aria-valuenow', String(Math.round(pct)));
    if (btnImport && !indeterminate) {
      replaceChildren(btnImport, h('span', { className: 'btn-spinner', 'aria-hidden': 'true' }), ` ${Math.round(pct)}%`);
    }
    if (message) setHud(message);
  }

  function endProgress(controller) {
    if (activeAnalysis && activeAnalysis.controller === controller) activeAnalysis = null;
    if (activeAnalysis) return;
    setTimeout(() => {
      if (activeAnalysis) return;
      if (progressTrack) {
        progressTrack.classList.remove('active', 'indeterminate');
        progressTrack.setAttribute('aria-hidden', 'true');
      }
      if (progressFill) progressFill.style.width = '0%';
    }, 600);
    if (btnImport) {
      btnImport.disabled = false;
      btnImport.textContent = 'Analyze Repo';
    }
    if (btnCancel) btnCancel.hidden = true;
    if (canvasCard) canvasCard.removeAttribute('aria-busy');
  }

  if (btnCancel) {
    btnCancel.addEventListener('click', () => {
      if (activeAnalysis) activeAnalysis.controller.abort();
    });
  }

  // ── Rust engine (analysis server) ──
  async function probeEngine(signal) {
    const timeout = new AbortController();
    const timer = setTimeout(() => timeout.abort(), ENGINE_HEALTH_TIMEOUT_MS);
    const onAbort = () => timeout.abort();
    signal.addEventListener('abort', onAbort, { once: true });
    try {
      const res = await fetch('/api/health', { signal: timeout.signal, cache: 'no-store' });
      if (!res.ok) return false;
      const json = await res.json();
      return Boolean(json && json.status === 'online');
    } catch (_) {
      return false;
    } finally {
      clearTimeout(timer);
      signal.removeEventListener('abort', onAbort);
    }
  }

  function sanitizeArtifactBase(value) {
    return typeof value === 'string' && value.startsWith('/') && !value.startsWith('//') && !value.includes('..') ? value.slice(0, 300) : null;
  }

  function pickPuml(diagrams) {
    const out = {};
    if (diagrams && typeof diagrams === 'object') {
      Object.entries(diagrams).forEach(([k, v]) => {
        if (/^[a-z_]{1,32}$/.test(k) && typeof v === 'string' && v.trim()) out[k] = v;
      });
    }
    return out;
  }

  async function pollEngineJob(jobId, signal, onTick) {
    const path = `/api/analyze/${encodeURIComponent(jobId)}`;
    while (true) {
      if (signal.aborted) throw signal.reason instanceof Error ? signal.reason : Object.assign(new Error('Analysis cancelled.'), { name: 'AbortError' });
      await new Promise((resolve) => setTimeout(resolve, ENGINE_POLL_INTERVAL_MS));
      const res = await fetch(path, { signal, cache: 'no-store' });
      if (!res.ok) throw new Error(`The analysis server returned HTTP ${res.status} for job ${jobId}.`);
      const json = await res.json();
      if (json.status === 'queued' || json.status === 'running') {
        onTick(json);
        continue;
      }
      return json;
    }
  }

  /**
   * Runs the analysis on the Rust engine. Returns true when the engine result was adopted,
   * false when the caller should fall back to in-browser analysis (with engineErrors set).
   */
  async function analyzeWithEngine(url, info, controller, started, engineErrors) {
    const elapsedTimer = setInterval(() => {
      const secs = ((performance.now() - started) / 1000).toFixed(0);
      setProgress(0, `Waiting for the analysis server (clone + pipeline) for ${info.owner}/${info.repo}: ${secs}s elapsed`, true);
    }, 1000);
    // The engine request has its own controller: a timeout abandons only the engine attempt,
    // while the user's Cancel aborts everything.
    const engineController = new AbortController();
    const forwardAbort = () => engineController.abort();
    controller.signal.addEventListener('abort', forwardAbort, { once: true });
    let timedOut = false;
    const timeout = setTimeout(() => {
      timedOut = true;
      engineController.abort();
    }, ENGINE_ANALYZE_TIMEOUT_MS);
    try {
      setProgress(0, `Sent ${info.owner}/${info.repo} to the analysis server...`, true);
      let res;
      let data = null;
      try {
        res = await fetch('/api/analyze', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ repo_url: url }),
          signal: engineController.signal
        });
        try {
          data = await res.json();
        } catch (_) {
          engineErrors.push(`The analysis server returned HTTP ${res.status} without a JSON body.`);
          return false;
        }
        if (data && data.job_id && (data.status === 'queued' || data.status === 'running')) {
          data = await pollEngineJob(data.job_id, engineController.signal, (job) => {
            if (typeof job.progress === 'number' && typeof job.phase === 'string') {
              setProgress(job.progress, `Analysis server: ${job.phase}`);
            }
          });
        }
      } catch (err) {
        if (controller.signal.aborted) throw err;
        if (timedOut) {
          engineErrors.push(`The analysis server did not finish within ${ENGINE_ANALYZE_TIMEOUT_MS / 60000} minutes.`);
        } else {
          engineErrors.push(`The analysis server request failed: ${(err && err.message) || err}.`);
        }
        return false;
      }
      const errors = Array.isArray(data && data.errors) ? data.errors.filter(e => typeof e === 'string') : [];
      if (!res.ok || !data || data.status !== 'success') {
        engineErrors.push(...(errors.length ? errors : [`The analysis server returned HTTP ${res.status} (${data && data.status ? data.status : 'no status'}).`]));
        return false;
      }
      const stats = data.stats && typeof data.stats === 'object' ? data.stats : null;
      const session = {
        source: { kind: 'engine', name: `${info.owner}/${info.repo}`, url },
        ir: null,
        engine: {
          session_id: typeof data.session_id === 'string' ? data.session_id.slice(0, 200) : null,
          artifact_base_url: sanitizeArtifactBase(data.artifact_base_url),
          puml_by_type: pickPuml(data.diagrams),
          errors,
          logs: Array.isArray(data.logs) ? data.logs.filter(l => typeof l === 'string').slice(0, 200) : []
        },
        stats,
        provenance: {
          engine: 'OpenHeart Rust engine (analysis server)',
          approximate: false,
          content_hash: null,
          coverage: stats && Number.isFinite(stats.files_processed) ? `${stats.files_processed.toLocaleString()} files processed` : null
        }
      };
      studioStore.replace(session, new Map());
      editor.clearSources();
      addToProjectHistory({ id: url, name: `${info.owner}/${info.repo}`, type: 'github', url, badge: 'Engine' });
      syncRepoInput();
      const totalSec = ((performance.now() - started) / 1000).toFixed(1);
      await canvas.renderGraph(canvas.currentGraphType);
      const msg = `Rust engine analysis of ${info.owner}/${info.repo}: ${stats && stats.files_processed !== undefined ? `${stats.files_processed} files, ` : ''}${stats && stats.total_classes !== undefined ? `${stats.total_classes} classes, ` : ''}${totalSec}s total`;
      setHud(msg);
      announce(msg);
      return true;
    } finally {
      clearInterval(elapsedTimer);
      clearTimeout(timeout);
      controller.signal.removeEventListener('abort', forwardAbort);
      if (progressTrack) progressTrack.classList.remove('indeterminate');
    }
  }

  // ── In-browser results ──
  async function adoptInBrowserResult(result, historyItem, started) {
    const stats = result.stats;
    const coverage = `${stats.files_parsed.toLocaleString()} of ${stats.files_eligible.toLocaleString()} eligible files parsed`;
    const session = {
      source: result.source,
      ir: result.graph_ir,
      engine: null,
      stats,
      provenance: {
        engine: 'In-browser declaration extractor (approximate)',
        approximate: true,
        content_hash: result.graph_ir.metadata.content_hash || null,
        content_hash_unavailable: result.graph_ir.metadata.content_hash_unavailable || null,
        coverage
      }
    };
    studioStore.replace(session, result.fileCache);
    editor.clearSources();
    editor.setMultipleSources(result.fileCache);
    addToProjectHistory(historyItem);
    syncRepoInput();
    if (!IN_BROWSER_VIEWS.has(canvas.currentGraphType)) {
      localStore.set('openheart_active_graph', 'class');
    }
    await canvas.renderGraph(IN_BROWSER_VIEWS.has(canvas.currentGraphType) ? canvas.currentGraphType : 'class');
    const totalSec = ((performance.now() - started) / 1000).toFixed(1);
    const msg = `In-browser class model of ${result.source.name}: ${stats.total_classes.toLocaleString()} types, ${stats.total_relations.toLocaleString()} relations from ${coverage} (${totalSec}s)`;
    setHud(msg);
    announce(msg);
    if (!studioStore.lastPersistOk) {
      showNotice('warning', 'Session not saved for reload', ['This browser refused to store the analysis (storage blocked or full); it will be lost on reload.']);
    }
  }

  function syncRepoInput() {
    const input = $('input-repo-url');
    if (!input) return;
    const session = studioStore.get();
    if (session.source.kind === 'github' || session.source.kind === 'engine') {
      input.value = session.source.url || '';
    } else if (session.source.kind === 'folder' || session.source.kind === 'zip') {
      input.value = '';
      input.placeholder = `Local codebase "${session.source.name}" is active · paste a GitHub URL to analyze another`;
    } else {
      input.placeholder = 'Paste a GitHub URL or pick a recent project...';
    }
  }

  function githubToken() {
    const token = sessionStore.get(TOKEN_KEY, null);
    return typeof token === 'string' && token.trim() ? token.trim() : null;
  }

  async function importRepo(rawInput) {
    const url = String(rawInput || '').trim();
    if (!url) return;
    const info = GitHubEngine.parseRepoUrl(url);
    if (!info) {
      setHudError(`"${url}" is not a GitHub repository URL (expected https://github.com/owner/repo).`);
      return;
    }
    const controller = beginProgress();
    const started = performance.now();
    const engineErrors = [];
    try {
      if (await probeEngine(controller.signal)) {
        if (await analyzeWithEngine(url, info, controller, started, engineErrors)) return;
      }
      if (controller.signal.aborted) throw Object.assign(new Error('Analysis cancelled.'), { name: 'AbortError' });
      setProgress(5, `Analyzing ${info.owner}/${info.repo} in the browser (approximate)...`);
      const result = await GitHubEngine.analyzeGitHubRepo(url, {
        signal: controller.signal,
        token: githubToken(),
        onProgress: (pct, msg) => setProgress(pct, msg)
      });
      if (progressFill) progressFill.style.width = '100%';
      await adoptInBrowserResult(result, { id: url, name: `${info.owner}/${info.repo}`, type: 'github', url, badge: 'In-browser' }, started);
      if (engineErrors.length) {
        showNotice('warning', 'The analysis server failed; showing in-browser approximate results', engineErrors);
      }
    } catch (err) {
      if (err && err.name === 'AbortError') {
        setHud('Analysis cancelled.');
        announce('Analysis cancelled.');
        return;
      }
      console.error('[OpenHeart Studio] Repo analysis error:', err);
      const message = err instanceof GitHubError || (err && err.message) ? err.message : String(err);
      setHudError(`Analysis failed: ${message}`);
      announce(`Analysis failed: ${message}`);
      if (engineErrors.length) showNotice('warning', 'The analysis server also failed', engineErrors);
    } finally {
      endProgress(controller);
    }
  }

  async function importLocal(fileItems, name, kind) {
    const controller = beginProgress();
    const started = performance.now();
    try {
      setProgress(8, `Analyzing ${name} in the browser (approximate)...`);
      const result = await GitHubEngine.analyzeSourceFiles(fileItems, name, {
        signal: controller.signal,
        sourceKind: kind,
        onProgress: (pct, msg) => setProgress(pct, msg)
      });
      if (progressFill) progressFill.style.width = '100%';
      await adoptInBrowserResult(result, { id: `${kind}:${name}`, name, type: kind, url: null, badge: kind === 'zip' ? 'ZIP' : 'Folder' }, started);
    } catch (err) {
      if (err && err.name === 'AbortError') {
        setHud('Analysis cancelled.');
        announce('Analysis cancelled.');
        return;
      }
      console.error('[OpenHeart Studio] Codebase import error:', err);
      setHudError(`Analysis failed: ${(err && err.message) || err}`);
      announce('Analysis failed.');
    } finally {
      endProgress(controller);
    }
  }

  // ── Local folder, ZIP and drag-and-drop ingestion ──
  const fileItemsFromFiles = (files) => files.map(f => ({
    path: f.webkitRelativePath || f.name,
    size: typeof f.size === 'number' ? f.size : null,
    getText: () => f.text()
  }));

  const inputFolder = $('input-folder-picker');
  const btnFolder = $('btn-import-folder');
  if (btnFolder && inputFolder) {
    btnFolder.addEventListener('click', () => {
      inputFolder.value = '';
      inputFolder.click();
    });
    inputFolder.addEventListener('change', async (e) => {
      const files = Array.from(e.target.files || []);
      if (files.length === 0) return;
      const firstPath = files[0].webkitRelativePath || files[0].name;
      const folderName = firstPath.split('/')[0] || 'LocalFolder';
      await importLocal(fileItemsFromFiles(files), folderName, 'folder');
    });
  }

  async function importZip(file) {
    const zipName = file.name.replace(/\.zip$/i, '') || 'Archive';
    setHud('Unpacking ZIP archive ', strong(file.name), '...');
    let items;
    try {
      items = await GitHubEngine.readZipArchive(file);
    } catch (zipErr) {
      console.error('[OpenHeart Studio] ZIP read error:', zipErr);
      setHudError(`Failed to read ZIP: ${(zipErr && zipErr.message) || zipErr}`);
      return;
    }
    await importLocal(items, zipName, 'zip');
  }

  const inputZip = $('input-zip-picker');
  const btnZip = $('btn-import-zip');
  if (btnZip && inputZip) {
    btnZip.addEventListener('click', () => {
      inputZip.value = '';
      inputZip.click();
    });
    inputZip.addEventListener('change', async (e) => {
      const file = e.target.files && e.target.files[0];
      if (file) await importZip(file);
    });
  }

  async function readDroppedDirectory(entry) {
    const items = [];
    const maxEntries = INGESTION_LIMITS.maxLocalFiles * 20;
    let visited = 0;
    const walk = async (dirEntry, prefix) => {
      const reader = dirEntry.createReader();
      while (visited < maxEntries) {
        const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
        if (!batch.length) break;
        for (const child of batch) {
          visited++;
          const path = prefix ? `${prefix}/${child.name}` : child.name;
          if (child.isDirectory) {
            await walk(child, path);
          } else if (child.isFile) {
            const file = await new Promise((resolve, reject) => child.file(resolve, reject));
            items.push({ path, size: file.size, getText: () => file.text() });
          }
          if (visited >= maxEntries) break;
        }
      }
    };
    await walk(entry, entry.name);
    return items;
  }

  window.addEventListener('dragover', (e) => e.preventDefault());
  window.addEventListener('drop', async (e) => {
    e.preventDefault();
    const dt = e.dataTransfer;
    if (!dt) return;
    const entries = Array.from(dt.items || [])
      .map(item => (typeof item.webkitGetAsEntry === 'function' ? item.webkitGetAsEntry() : null))
      .filter(Boolean);
    const dirEntry = entries.find(en => en.isDirectory);
    if (dirEntry) {
      try {
        const items = await readDroppedDirectory(dirEntry);
        await importLocal(items, dirEntry.name, 'folder');
      } catch (err) {
        setHudError(`Could not read the dropped folder: ${(err && err.message) || err}`);
      }
      return;
    }
    const files = Array.from(dt.files || []);
    const zipFile = files.find(f => f.name.toLowerCase().endsWith('.zip'));
    if (zipFile) {
      await importZip(zipFile);
    } else if (files.length > 0) {
      await importLocal(fileItemsFromFiles(files), files.length === 1 ? files[0].name : 'Dropped files', 'folder');
    }
  });

  // ── Repository input, presets and history ──
  const inputRepoEl = $('input-repo-url');
  if (inputRepoEl) {
    inputRepoEl.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        importRepo(inputRepoEl.value);
      }
    });
  }
  if (btnImport) {
    btnImport.addEventListener('click', () => importRepo($('input-repo-url').value));
  }

  document.querySelectorAll('.preset-chip').forEach(chip => {
    chip.addEventListener('click', () => {
      const preset = PRESETS.find(p => p.id === chip.dataset.repo);
      if (!preset) return;
      $('input-repo-url').value = preset.url;
      importRepo(preset.url);
    });
  });

  function getProjectHistory() {
    const raw = localStore.getJson(HISTORY_KEY, []);
    if (!Array.isArray(raw)) return [];
    return raw.filter(item => item && typeof item === 'object' &&
      typeof item.id === 'string' && typeof item.name === 'string' &&
      ['github', 'folder', 'zip'].includes(item.type) &&
      (item.type !== 'github' || (typeof item.url === 'string' && GitHubEngine.parseRepoUrl(item.url))))
      .map(item => ({
        id: item.id.slice(0, 300),
        name: item.name.slice(0, 200),
        type: item.type,
        url: item.type === 'github' ? item.url.slice(0, 500) : null,
        badge: typeof item.badge === 'string' ? item.badge.slice(0, 24) : item.type
      }))
      .slice(0, 8);
  }

  function addToProjectHistory(item) {
    let history = getProjectHistory().filter(x => x.id !== item.id);
    history.unshift(item);
    history = history.slice(0, 8);
    localStore.setJson(HISTORY_KEY, history);
    renderProjectHistoryDropdown();
  }

  const dropdown = $('repo-history-dropdown');
  const btnHistoryToggle = $('btn-repo-dropdown-toggle');

  function setHistoryOpen(open) {
    if (!dropdown) return;
    dropdown.classList.toggle('hidden-dropdown', !open);
    if (btnHistoryToggle) btnHistoryToggle.setAttribute('aria-expanded', String(open));
  }

  function historyButton(content, onSelect) {
    return h('button', {
      type: 'button',
      className: 'repo-history-item',
      role: 'menuitem',
      onClick: (e) => {
        e.stopPropagation();
        setHistoryOpen(false);
        onSelect();
      }
    }, ...content);
  }

  function renderProjectHistoryDropdown() {
    if (!dropdown) return;
    const history = getProjectHistory();
    const children = [];
    if (history.length > 0) {
      children.push(h('div', { className: 'repo-history-section-title' },
        h('span', null, 'Recent Projects'),
        h('button', {
          type: 'button',
          className: 'repo-history-clear-btn',
          onClick: (e) => {
            e.stopPropagation();
            localStore.remove(HISTORY_KEY);
            renderProjectHistoryDropdown();
          }
        }, 'Clear')
      ));
      history.forEach(item => {
        const iconName = item.type === 'github' ? 'github' : (item.type === 'zip' ? 'archive' : 'folder');
        children.push(historyButton([
          h('span', { className: 'repo-history-icon', 'aria-hidden': 'true' }, icon(iconName)),
          h('span', { className: 'repo-history-text' }, item.name),
          h('span', { className: 'repo-history-badge' }, item.badge || item.type)
        ], () => {
          if (item.type === 'github') {
            $('input-repo-url').value = item.url;
            importRepo(item.url);
            return;
          }
          const session = studioStore.get();
          if (session.source.kind === item.type && session.source.name === item.name && session.ir) {
            canvas.renderGraph(canvas.currentGraphType);
          } else {
            setHud('Project ', strong(item.name), ' is a local codebase: choose Folder or ZIP to load its files again.');
          }
        }));
      });
      children.push(h('div', { className: 'repo-history-divider' }));
    }
    children.push(h('div', { className: 'repo-history-section-title' }, h('span', null, 'Presets & Demo Codebases')));
    PRESETS.forEach(preset => {
      children.push(historyButton([
        h('span', { className: 'repo-history-icon', 'aria-hidden': 'true' }, icon('fileCode')),
        h('span', { className: 'repo-history-text' }, preset.label),
        h('span', { className: 'repo-history-badge' }, 'Preset')
      ], () => {
        $('input-repo-url').value = preset.url;
        importRepo(preset.url);
      }));
    });
    children.push(h('div', { className: 'repo-history-divider' }));
    children.push(historyButton([
      h('span', { className: 'repo-history-icon', 'aria-hidden': 'true' }, icon('fileCode')),
      h('span', { className: 'repo-history-text' }, 'Bundled sample diagrams'),
      h('span', { className: 'repo-history-badge' }, 'Reset')
    ], () => {
      studioStore.reset();
      editor.clearSources();
      syncRepoInput();
      canvas.renderGraph(canvas.currentGraphType);
    }));
    replaceChildren(dropdown, ...children);
  }

  renderProjectHistoryDropdown();
  if (btnHistoryToggle && dropdown) {
    btnHistoryToggle.addEventListener('click', (e) => {
      e.stopPropagation();
      renderProjectHistoryDropdown();
      setHistoryOpen(dropdown.classList.contains('hidden-dropdown'));
    });
  }
  if (inputRepoEl && dropdown) {
    inputRepoEl.addEventListener('focus', () => {
      renderProjectHistoryDropdown();
      setHistoryOpen(true);
    });
  }

  // ── GitHub token (kept in sessionStorage for this tab only, never in localStorage) ──
  const tokenDialog = $('github-token-dialog');
  const tokenInput = $('github-token-input');
  const btnToken = $('btn-github-token');
  const syncTokenButton = () => {
    if (!btnToken) return;
    const hasToken = Boolean(githubToken());
    btnToken.classList.toggle('active-pill', hasToken);
    btnToken.setAttribute('aria-pressed', String(hasToken));
    btnToken.title = hasToken ? 'A GitHub token is set for this tab' : 'Set a GitHub token (raises the rate limit, enables private repositories)';
  };
  syncTokenButton();
  if (btnToken && tokenDialog && typeof tokenDialog.showModal === 'function') {
    btnToken.addEventListener('click', () => {
      if (tokenInput) tokenInput.value = '';
      tokenDialog.showModal();
      if (tokenInput) tokenInput.focus();
    });
    $('btn-token-save').addEventListener('click', () => {
      const value = tokenInput ? tokenInput.value.trim() : '';
      if (value) sessionStore.set(TOKEN_KEY, value);
      if (tokenInput) tokenInput.value = '';
      tokenDialog.close();
      syncTokenButton();
    });
    $('btn-token-forget').addEventListener('click', () => {
      sessionStore.remove(TOKEN_KEY);
      if (tokenInput) tokenInput.value = '';
      tokenDialog.close();
      syncTokenButton();
    });
    $('btn-token-cancel').addEventListener('click', () => {
      if (tokenInput) tokenInput.value = '';
      tokenDialog.close();
    });
  } else if (btnToken) {
    btnToken.hidden = true;
  }

  // ── Pan lock ──
  const btnPanLock = $('btn-pan-lock');
  const syncPanLock = () => {
    const isLocked = canvas.isPanLocked;
    const iconPanLock = $('icon-pan-lock');
    const labelPanLock = $('label-pan-lock');
    if (iconPanLock) replaceChildren(iconPanLock, icon(isLocked ? 'lock' : 'unlock'));
    if (labelPanLock) labelPanLock.textContent = isLocked ? 'Locked' : 'Unlocked';
    if (btnPanLock) {
      btnPanLock.classList.toggle('active-locked', isLocked);
      btnPanLock.setAttribute('aria-pressed', String(isLocked));
    }
  };
  if (btnPanLock) {
    btnPanLock.addEventListener('click', () => {
      const isLocked = canvas.togglePanLock();
      syncPanLock();
      setHud(strong(isLocked ? 'Pan Lock Active' : 'Pan Lock Disabled'), isLocked ? ': layout locked; pan and inspect only.' : ': nodes can be repositioned freely.');
    });
  }

  $('btn-toggle-minimap').addEventListener('click', () => canvas.toggleMinimap());
  $('btn-toggle-loupe').addEventListener('click', () => canvas.toggleLoupe());

  // ── Canvas search ──
  const searchInput = $('canvas-search-input');
  const searchResults = $('canvas-search-results');
  let searchMatches = [];
  let searchCursor = -1;
  const closeSearch = () => {
    if (!searchResults) return;
    searchResults.hidden = true;
    replaceChildren(searchResults);
    if (searchInput) {
      searchInput.setAttribute('aria-expanded', 'false');
      searchInput.removeAttribute('aria-activedescendant');
    }
  };
  const chooseResult = (match) => {
    if (!match) return;
    canvas.focusNodeById(match.id, { inspect: !match.isPackage });
    closeSearch();
    announce(`Focused ${match.name}.`);
  };
  const renderSearch = () => {
    if (!searchResults || !searchInput) return;
    if (!searchMatches.length) {
      replaceChildren(searchResults, h('li', { className: 'canvas-search-empty', role: 'option', 'aria-disabled': 'true' }, 'No matching elements'));
      searchResults.hidden = false;
      searchInput.setAttribute('aria-expanded', 'true');
      return;
    }
    replaceChildren(searchResults, ...searchMatches.map((m, idx) => h('li', {
      id: `canvas-search-opt-${idx}`,
      role: 'option',
      className: `canvas-search-option ${idx === searchCursor ? 'active' : ''}`,
      'aria-selected': String(idx === searchCursor),
      onMousedown: (e) => {
        e.preventDefault();
        chooseResult(m);
      }
    }, h('span', { className: 'canvas-search-name' }, m.name), h('span', { className: 'canvas-search-kind' }, m.kind), m.qualifiedName !== m.name ? h('span', { className: 'canvas-search-qualified' }, m.qualifiedName) : null)));
    searchResults.hidden = false;
    searchInput.setAttribute('aria-expanded', 'true');
    if (searchCursor >= 0) searchInput.setAttribute('aria-activedescendant', `canvas-search-opt-${searchCursor}`);
    else searchInput.removeAttribute('aria-activedescendant');
  };
  if (searchInput && searchResults) {
    searchInput.addEventListener('input', () => {
      searchMatches = canvas.searchNodes(searchInput.value, 12);
      searchCursor = searchMatches.length ? 0 : -1;
      if (!searchInput.value.trim()) closeSearch();
      else renderSearch();
    });
    searchInput.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown' && searchMatches.length) {
        e.preventDefault();
        searchCursor = (searchCursor + 1) % searchMatches.length;
        renderSearch();
      } else if (e.key === 'ArrowUp' && searchMatches.length) {
        e.preventDefault();
        searchCursor = (searchCursor - 1 + searchMatches.length) % searchMatches.length;
        renderSearch();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        chooseResult(searchMatches[searchCursor]);
      } else if (e.key === 'Escape') {
        searchInput.value = '';
        closeSearch();
      }
    });
    searchInput.addEventListener('blur', () => setTimeout(closeSearch, 120));
    window.addEventListener('keydown', (e) => {
      if (e.key === '/' && !e.ctrlKey && !e.metaKey && !e.altKey && canvas.isCanvasFocused()) {
        e.preventDefault();
        searchInput.focus();
      }
    });
  }

  // ── Export menu ──
  const btnExportMenu = $('btn-export-menu');
  const exportMenu = $('export-menu-dropdown');
  const setExportOpen = (open) => {
    if (!exportMenu) return;
    exportMenu.classList.toggle('hidden-dropdown', !open);
    if (btnExportMenu) btnExportMenu.setAttribute('aria-expanded', String(open));
    if (open) {
      const first = exportMenu.querySelector('button');
      if (first) first.focus();
    }
  };
  if (btnExportMenu && exportMenu) {
    btnExportMenu.addEventListener('click', (e) => {
      e.stopPropagation();
      setExportOpen(exportMenu.classList.contains('hidden-dropdown'));
    });
    exportMenu.addEventListener('keydown', (e) => {
      const items = Array.from(exportMenu.querySelectorAll('button'));
      const idx = items.indexOf(document.activeElement);
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        items[(idx + 1) % items.length].focus();
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        items[(idx - 1 + items.length) % items.length].focus();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setExportOpen(false);
        btnExportMenu.focus();
      }
    });
  }

  function downloadBlob(content, mimeType, filename) {
    const blob = new Blob([content], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = h('a', { href: url, download: filename });
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function downloadDataUri(dataUri, filename) {
    const a = h('a', { href: dataUri, download: filename });
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  const exportStem = () => safeFileStem(canvas.snapshotLabel || studioStore.codebaseName);
  const onExport = (id, handler) => {
    const btn = $(id);
    if (!btn) return;
    btn.addEventListener('click', async () => {
      setExportOpen(false);
      try {
        await handler();
      } catch (err) {
        console.error('[OpenHeart Studio] Export failed:', err);
        setHudError(`Export failed: ${(err && err.message) || err}`);
      }
    });
  };

  onExport('btn-save-json', () => {
    const state = canvas.getDiagramStateJson();
    if (!state) return;
    const filename = `${exportStem()}_${canvas.currentGraphType}_state.openheart.json`;
    downloadBlob(JSON.stringify(state, null, 2), 'application/json', filename);
    setHud('Diagram state saved as ', strong(filename));
  });

  const btnLoadJson = $('btn-load-json');
  const inputLoadJson = $('input-load-json');
  if (btnLoadJson && inputLoadJson) {
    btnLoadJson.addEventListener('click', () => {
      setExportOpen(false);
      inputLoadJson.value = '';
      inputLoadJson.click();
    });
    inputLoadJson.addEventListener('change', async (e) => {
      const file = e.target.files && e.target.files[0];
      if (!file) return;
      if (file.size > MAX_STATE_FILE_BYTES) {
        showNotice('error', 'State file rejected', [`The file is ${Math.round(file.size / 1048576)} MB; the limit is ${MAX_STATE_FILE_BYTES / 1048576} MB.`]);
        return;
      }
      let parsed;
      try {
        parsed = JSON.parse(await file.text());
      } catch (err) {
        showNotice('error', 'State file rejected', [`${file.name} is not valid JSON: ${(err && err.message) || err}`]);
        return;
      }
      const v = validateDiagramState(parsed, {
        allowedDiagramTypes: diagramIds,
        allowedEdgeKinds: new Set(Object.keys((manifest && manifest.relationship_types) || {}))
      });
      if (!v.ok) {
        showNotice('error', `Could not load ${file.name}`, v.errors);
        setHudError(`Could not load ${file.name}.`);
        return;
      }
      const loaded = await canvas.loadDiagramState(v.state);
      if (!loaded) return;
      setActiveSidebar(v.state.diagram_type);
      syncPanLock();
      const items = [`Snapshot of "${v.state.codebase_name || 'unnamed codebase'}" from a saved state file. It is not linked to the active codebase; switching views reloads the active source.`];
      showNotice(v.warnings.length ? 'warning' : 'info', v.warnings.length ? 'State file loaded with repairs' : 'State file loaded', [...items, ...v.warnings]);
      setHud('Loaded diagram state from ', strong(file.name), ` (${diagramName(v.state.diagram_type)})`);
    });
  }

  onExport('btn-export-ai-json', () => {
    const ai = canvas.getCodeStructureForAi();
    if (!ai) return;
    const filename = `${exportStem()}_code_structure.ai.json`;
    downloadBlob(JSON.stringify(ai, null, 2), 'application/json', filename);
    setHud('AI code structure saved as ', strong(filename), ` (${ai.summary.total_types} types, ${ai.summary.total_relationships} relations)`);
  });

  onExport('btn-export-ai-md', async () => {
    const md = canvas.getCodeStructureMarkdownForAi();
    if (!md) return;
    try {
      await navigator.clipboard.writeText(md);
      setHud('Copied the ', strong('LLM Markdown prompt'), ' to the clipboard');
    } catch (_) {
      const filename = `${exportStem()}_llm_prompt.md`;
      downloadBlob(md, 'text/markdown;charset=utf-8', filename);
      setHud('Clipboard unavailable; saved the LLM prompt as ', strong(filename));
    }
  });

  onExport('btn-export-png', () => {
    const png = canvas.exportPng();
    if (!png) return;
    const filename = `${exportStem()}_${canvas.currentGraphType}.png`;
    downloadDataUri(png.dataUri, filename);
    setHud('PNG exported as ', strong(filename), png.downscaled ? ` (scaled to ${(png.scale * 100).toFixed(0)}% to fit browser canvas limits)` : '');
  });

  onExport('btn-export-svg', () => {
    const svg = canvas.exportSvg();
    if (!svg) {
      throw new Error('SVG export is unavailable because the cytoscape-svg plugin did not load.');
    }
    const filename = `${exportStem()}_${canvas.currentGraphType}.svg`;
    downloadBlob(svg, 'image/svg+xml;charset=utf-8', filename);
    setHud('Vector SVG exported as ', strong(filename));
  });

  onExport('btn-export-dot', () => {
    const snapshot = canvas.getGraphSnapshot();
    if (!snapshot) return;
    const filename = `${exportStem()}_${canvas.currentGraphType}.dot`;
    downloadBlob(toDot(snapshot), 'text/vnd.graphviz;charset=utf-8', filename);
    setHud('Graphviz DOT exported as ', strong(filename));
  });

  onExport('btn-export-graphml', () => {
    const snapshot = canvas.getGraphSnapshot();
    if (!snapshot) return;
    const filename = `${exportStem()}_${canvas.currentGraphType}.graphml`;
    downloadBlob(toGraphMl(snapshot), 'application/graphml+xml;charset=utf-8', filename);
    setHud('GraphML exported as ', strong(filename));
  });

  // ── Close popovers on outside click / Escape ──
  window.addEventListener('click', (e) => {
    if (dropdown && !dropdown.contains(e.target) && e.target !== btnHistoryToggle && e.target !== inputRepoEl) {
      setHistoryOpen(false);
    }
    if (exportMenu && !exportMenu.contains(e.target) && btnExportMenu && !btnExportMenu.contains(e.target)) {
      if (!exportMenu.classList.contains('hidden-dropdown')) setExportOpen(false);
    }
  });
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    if (dropdown && !dropdown.classList.contains('hidden-dropdown')) {
      setHistoryOpen(false);
      if (btnHistoryToggle) btnHistoryToggle.focus();
    }
  });

  studioStore.subscribe(() => {
    markViewAvailability();
    updateBreadcrumbs(null);
  });

  if (!restore.restored && !(hudText && hudText.querySelector('.hud-error'))) {
    setHud(DEFAULT_HUD_HINT);
  }
}
