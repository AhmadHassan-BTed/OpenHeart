/**
 * OpenHeart File Hierarchy Tree Explorer
 * Builds the project tree from the source files that the current diagram's nodes actually
 * reference (nodes without a recorded file are not listed). Rendered as a WAI-ARIA tree:
 * roving tabindex, Up/Down to move, Right/Left to expand/collapse, Home/End, Enter/Space to open.
 */

import { icon } from './icons.js';
import { h, replaceChildren } from './dom.js';
import { studioStore } from './studio-store.js';

function kindLetterFor(fullPath, kind) {
  const lower = fullPath.toLowerCase();
  if (lower.endsWith('.kt') || lower.endsWith('.kts')) return 'K';
  if (lower.endsWith('.rs')) return 'R';
  if (/\.(ts|tsx|mts|cts|js|jsx|mjs|cjs)$/.test(lower)) return 'T';
  if (lower.endsWith('.py')) return 'P';
  if (lower.endsWith('.go')) return 'G';
  if (lower.endsWith('.cs')) return 'C';
  if (kind === 'interface') return 'I';
  if (kind === 'abstract') return 'A';
  if (kind === 'enum') return 'E';
  return 'C';
}

export class FileTreeExplorer {
  constructor(containerId, onFileSelectCallback) {
    this.container = document.getElementById(containerId);
    this.onFileSelect = onFileSelectCallback;
    this.activeFile = null;
    this.activeNodeId = null;
    this.treeData = { name: 'Workspace', type: 'folder', expanded: true, children: [] };
    if (this.container) {
      this.container.addEventListener('keydown', (e) => this.handleKeydown(e));
    }
  }

  /**
   * Builds the tree from Cytoscape elements carrying a `file` (and, when known, an anchor).
   */
  updateFromElements(elements) {
    const fileElements = (elements || []).filter(el => el.data && el.data.file && !el.data.isPackage && !el.data.source && !el.data.isNotice);
    const rootName = studioStore.codebaseName || 'Workspace';
    const root = { name: rootName, type: 'folder', expanded: true, children: [] };

    const insertIntoTree = (folderNode, segments, fileEntry) => {
      let current = folderNode;
      for (const seg of segments) {
        let childFolder = current.children.find(c => c.type === 'folder' && c.name === seg);
        if (!childFolder) {
          childFolder = { name: seg, type: 'folder', expanded: true, children: [] };
          current.children.push(childFolder);
        }
        current = childFolder;
      }
      // Several types can live in one file; keep one entry per file and remember every node.
      const existing = current.children.find(c => c.type !== 'folder' && c.fullPath === fileEntry.fullPath);
      if (existing) {
        existing.nodeIds.push(fileEntry.nodeId);
        return;
      }
      current.children.push(fileEntry);
    };

    fileElements.forEach(el => {
      const fullPath = String(el.data.file).replace(/\\/g, '/').replace(/^\/+/, '');
      const segments = fullPath.split('/').filter(Boolean);
      if (segments.length === 0) return;
      const baseFileName = segments[segments.length - 1];
      insertIntoTree(root, segments.slice(0, -1), {
        name: baseFileName,
        fullPath,
        type: 'file',
        kind: kindLetterFor(fullPath, el.data.kind || 'class'),
        nodeId: el.data.id,
        nodeIds: [el.data.id],
        data: el.data
      });
    });

    const sortTree = (node) => {
      if (!node.children || node.children.length === 0) return;
      node.children.sort((a, b) => {
        if ((a.type === 'folder') !== (b.type === 'folder')) return a.type === 'folder' ? -1 : 1;
        return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      });
      node.children.forEach(c => { if (c.type === 'folder') sortTree(c); });
    };
    sortTree(root);

    this.treeData = root;
    this.fileCount = fileElements.length ? new Set(fileElements.map(el => el.data.file)).size : 0;
    this.render();
  }

  render() {
    if (!this.container) return;
    const tree = h('div', { role: 'tree', 'aria-label': 'Repository files', className: 'file-tree-root' });
    if (this.treeData.children.length === 0) {
      replaceChildren(this.container,
        h('div', { className: 'tree-empty-note' }, 'No source files are linked to the nodes of this diagram.')
      );
      return;
    }
    tree.appendChild(this.buildNodeElement(this.treeData, 0, 1, 1));
    replaceChildren(this.container, tree);
    this.ensureTabStop();
  }

  buildNodeElement(node, depth, posInSet, setSize) {
    const isFolder = node.type === 'folder';
    const wrapper = h('div', { className: 'tree-item-wrapper', role: 'none' });
    const row = h('div', {
      className: isFolder ? 'tree-folder-row' : 'tree-file-row',
      role: 'treeitem',
      tabindex: '-1',
      'aria-level': depth + 1,
      'aria-posinset': posInSet,
      'aria-setsize': setSize,
      style: { 'padding-left': `${depth * 14 + 8}px` }
    });

    if (isFolder) {
      row.setAttribute('aria-expanded', String(node.expanded));
      const arrow = h('span', { className: 'tree-arrow', 'aria-hidden': 'true' }, icon(node.expanded ? 'chevronDown' : 'chevronRight'));
      const folderIcon = h('span', { className: 'tree-icon folder-icon', 'aria-hidden': 'true' }, icon(node.expanded ? 'folderOpen' : 'folder'));
      row.append(arrow, folderIcon, h('span', { className: 'tree-folder-label' }, node.name));

      const childrenContainer = h('div', { className: 'tree-children', role: 'group', style: { display: node.expanded ? 'block' : 'none' } });
      node.children.forEach((child, idx) => {
        childrenContainer.appendChild(this.buildNodeElement(child, depth + 1, idx + 1, node.children.length));
      });

      const setExpanded = (expanded) => {
        node.expanded = expanded;
        replaceChildren(arrow, icon(expanded ? 'chevronDown' : 'chevronRight'));
        replaceChildren(folderIcon, icon(expanded ? 'folderOpen' : 'folder'));
        childrenContainer.style.display = expanded ? 'block' : 'none';
        row.setAttribute('aria-expanded', String(expanded));
      };
      row._setExpanded = setExpanded;
      row._isExpanded = () => node.expanded;

      row.addEventListener('click', (e) => {
        e.stopPropagation();
        setExpanded(!node.expanded);
        this.focusRow(row);
      });

      wrapper.append(row, childrenContainer);
    } else {
      const badge = h('span', { className: `tree-badge badge-${node.kind ? node.kind.toLowerCase() : 'c'}`, 'aria-hidden': 'true' }, node.kind || 'C');
      const label = h('span', { className: 'tree-file-label', title: node.fullPath || node.name }, node.name);
      row.dataset.nodeId = node.nodeId || '';
      row.dataset.fileName = node.fullPath || node.name || '';
      row.dataset.baseName = node.name || '';
      if (node.fullPath === this.activeFile) {
        row.classList.add('active');
        row.setAttribute('aria-selected', 'true');
      } else {
        row.setAttribute('aria-selected', 'false');
      }
      row.append(badge, label);
      const open = () => {
        this.selectFile(node.fullPath, node.nodeId);
        if (this.onFileSelect) this.onFileSelect(node.fullPath, node);
      };
      row._open = open;
      row.addEventListener('click', (e) => {
        e.stopPropagation();
        open();
        this.focusRow(row);
      });
      wrapper.appendChild(row);
    }
    return wrapper;
  }

  visibleRows() {
    if (!this.container) return [];
    return Array.from(this.container.querySelectorAll('[role="treeitem"]')).filter(r => r.offsetParent !== null);
  }

  ensureTabStop() {
    const rows = this.visibleRows();
    if (rows.length === 0) return;
    const current = rows.find(r => r.getAttribute('tabindex') === '0');
    if (!current) {
      const active = rows.find(r => r.classList.contains('active')) || rows[0];
      active.setAttribute('tabindex', '0');
    }
  }

  focusRow(row) {
    if (!row || !this.container) return;
    this.container.querySelectorAll('[role="treeitem"][tabindex="0"]').forEach(r => r.setAttribute('tabindex', '-1'));
    row.setAttribute('tabindex', '0');
    row.focus({ preventScroll: false });
  }

  handleKeydown(e) {
    const row = e.target instanceof Element ? e.target.closest('[role="treeitem"]') : null;
    if (!row) return;
    const rows = this.visibleRows();
    const idx = rows.indexOf(row);
    const isFolder = row.classList.contains('tree-folder-row');
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (idx < rows.length - 1) this.focusRow(rows[idx + 1]);
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (idx > 0) this.focusRow(rows[idx - 1]);
        break;
      case 'Home':
        e.preventDefault();
        this.focusRow(rows[0]);
        break;
      case 'End':
        e.preventDefault();
        this.focusRow(rows[rows.length - 1]);
        break;
      case 'ArrowRight':
        e.preventDefault();
        if (isFolder && row._isExpanded && !row._isExpanded()) row._setExpanded(true);
        else if (isFolder && idx < rows.length - 1) this.focusRow(this.visibleRows()[idx + 1]);
        break;
      case 'ArrowLeft': {
        e.preventDefault();
        if (isFolder && row._isExpanded && row._isExpanded()) {
          row._setExpanded(false);
        } else {
          const group = row.parentElement && row.parentElement.parentElement;
          const parentRow = group && group.classList.contains('tree-children') ? group.previousElementSibling : null;
          if (parentRow) this.focusRow(parentRow);
        }
        break;
      }
      case 'Enter':
      case ' ':
        e.preventDefault();
        if (isFolder && row._setExpanded) row._setExpanded(!row._isExpanded());
        else if (row._open) row._open();
        break;
      default:
        break;
    }
  }

  /**
   * Marks the row for a file (or node id) as active and reveals it. Matching is by exact
   * path or node id, never by bare file name, so same-named files stay distinct.
   */
  selectFile(fileName, nodeId = null) {
    this.activeFile = fileName;
    this.activeNodeId = nodeId;
    if (!this.container) return;
    const normalized = fileName ? String(fileName).replace(/\\/g, '/').replace(/^\/+/, '') : null;
    this.container.querySelectorAll('.tree-file-row').forEach(r => {
      const isMatch = (nodeId && r.dataset.nodeId === nodeId) || (normalized && r.dataset.fileName === normalized);
      r.classList.toggle('active', Boolean(isMatch));
      r.setAttribute('aria-selected', isMatch ? 'true' : 'false');
      if (!isMatch) return;
      let parent = r.parentElement;
      while (parent && parent !== this.container) {
        if (parent.classList.contains('tree-children') && parent.style.display === 'none') {
          const folderRow = parent.previousElementSibling;
          if (folderRow && folderRow._setExpanded) folderRow._setExpanded(true);
          else parent.style.display = 'block';
        }
        parent = parent.parentElement;
      }
      r.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
      this.container.querySelectorAll('[role="treeitem"][tabindex="0"]').forEach(x => x.setAttribute('tabindex', '-1'));
      r.setAttribute('tabindex', '0');
    });
  }
}
