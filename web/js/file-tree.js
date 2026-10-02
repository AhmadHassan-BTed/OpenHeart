/**
 * OpenHeart Dynamic File Hierarchy Tree Explorer (Zero Hardcoding)
 * Dynamically builds the VS Code / JetBrains / Solution Explorer directory tree
 * from parsed compiler elements and real project file paths.
 */

import { Icons } from './icons.js';

export class FileTreeExplorer {
  constructor(containerId, onFileSelectCallback) {
    this.container = document.getElementById(containerId);
    this.onFileSelect = onFileSelectCallback;
    this.activeFile = null;
    this.treeData = {
      name: "Workspace",
      type: "folder",
      expanded: true,
      children: []
    };
  }

  /**
   * Dynamically constructs the file tree from Cytoscape / PlantUML elements
   */
  updateFromElements(elements, explicitRootName = null) {
    if (!elements || elements.length === 0) return;

    // 1. Determine Project Root Name dynamically
    let rootName = explicitRootName;
    if (!rootName) {
      try {
        const customName = sessionStorage.getItem('openheart_custom_name');
        if (customName && customName !== 'Local Codebase' && customName !== 'OpenHeart') {
          rootName = customName;
        }
      } catch (_) {}
    }

    // Extract valid code file elements
    const fileElements = elements.filter(el => el.data && el.data.file && !el.data.isPackage && !el.data.source);

    // If still no rootName, infer from file paths or active repo input
    if (!rootName && fileElements.length > 0) {
      const normalizedPaths = fileElements.map(el => (el.data.file || '').replace(/\\/g, '/').replace(/^\/+/, ''));
      // Check if all paths share a common top-level directory (e.g. "NCacheClient/...")
      const firstDirs = normalizedPaths.map(p => p.includes('/') ? p.split('/')[0] : null).filter(Boolean);
      if (firstDirs.length === normalizedPaths.length && firstDirs.length > 0 && firstDirs.every(d => d === firstDirs[0])) {
        rootName = firstDirs[0];
      }
    }

    if (!rootName) {
      const inputEl = document.getElementById('input-repo-url');
      if (inputEl && inputEl.value) {
        const v = inputEl.value.replace(' (Active Codebase)', '').trim();
        const match = v.match(/github\.com\/[^\/]+\/([^\/\?#]+)/);
        if (match) {
          rootName = match[1].replace(/\.git$/, '');
        } else if (v && !v.startsWith('http')) {
          rootName = v;
        }
      }
    }

    if (!rootName) {
      rootName = 'Workspace';
    }

    const root = {
      name: rootName,
      type: "folder",
      expanded: true,
      children: []
    };

    // 2. Build Package / Namespace Map for fallback grouping
    const packageMap = new Map();
    elements.forEach(el => {
      if (el.data && el.data.isPackage) {
        const pkgPath = el.data.rawName || el.data.id.replace(/^pkg_/, '').replace(/_/g, '.');
        packageMap.set(el.data.id, {
          id: el.data.id,
          path: pkgPath,
          parent: el.data.parent
        });
      }
    });

    // Helper: Insert file node into nested directory structure
    const insertIntoTree = (folderNode, segments, fileEntry) => {
      let current = folderNode;
      for (const seg of segments) {
        let childFolder = current.children.find(c => c.type === 'folder' && c.name === seg);
        if (!childFolder) {
          childFolder = {
            name: seg,
            type: 'folder',
            expanded: true,
            children: []
          };
          current.children.push(childFolder);
        }
        current = childFolder;
      }
      current.children.push(fileEntry);
    };

    // 3. Populate files into directory structure
    fileElements.forEach(el => {
      const fullPath = (el.data.file || '').replace(/\\/g, '/').replace(/^\/+/, '');
      const kind = el.data.kind || 'class';
      let kindLetter = 'C';
      const lower = fullPath.toLowerCase();

      if (lower.endsWith('.kt')) kindLetter = 'K';
      else if (lower.endsWith('.rs')) kindLetter = 'R';
      else if (lower.endsWith('.ts') || lower.endsWith('.tsx') || lower.endsWith('.js') || lower.endsWith('.jsx')) kindLetter = 'T';
      else if (lower.endsWith('.py')) kindLetter = 'P';
      else if (lower.endsWith('.cs')) kindLetter = 'C';
      else if (lower.endsWith('.go')) kindLetter = 'G';
      else if (kind === 'interface') kindLetter = 'I';
      else if (kind === 'abstract') kindLetter = 'A';
      else if (kind === 'enum') kindLetter = 'E';
      else if (kind === 'actor' || kind === 'usecase') kindLetter = 'U';
      else if (kind === 'bb') kindLetter = 'B';
      else if (kind === 'timing_track') kindLetter = 'T';

      // Determine relative path stripped of top-level root if repeated
      let relPath = fullPath;
      if (relPath.startsWith(`${rootName}/`)) {
        relPath = relPath.slice(rootName.length + 1);
      }

      const pathSegments = relPath.split('/').filter(Boolean);
      let folderSegments = [];
      let baseFileName = fullPath;

      if (pathSegments.length > 1) {
        folderSegments = pathSegments.slice(0, -1);
        baseFileName = pathSegments[pathSegments.length - 1];
      } else if (pathSegments.length === 1) {
        baseFileName = pathSegments[0];
        // If file has no directory slashes, check parent package
        const parentPkg = el.data.parent ? packageMap.get(el.data.parent) : null;
        if (parentPkg && parentPkg.path) {
          folderSegments = [parentPkg.path];
        }
      }

      const fileEntry = {
        name: baseFileName,
        fullPath: fullPath,
        type: kind,
        kind: kindLetter,
        nodeId: el.data.id,
        data: el.data
      };

      insertIntoTree(root, folderSegments, fileEntry);
    });

    // 4. Sort folders and files recursively (folders first, alphabetical)
    const sortTree = (node) => {
      if (!node.children || node.children.length === 0) return;
      node.children.sort((a, b) => {
        if (a.type !== b.type) {
          return a.type === 'folder' ? -1 : 1;
        }
        return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      });
      node.children.forEach(c => {
        if (c.type === 'folder') sortTree(c);
      });
    };

    sortTree(root);

    this.treeData = root;
    this.render();
  }

  render() {
    if (!this.container) return;
    this.container.innerHTML = '';
    const rootEl = this.buildNodeElement(this.treeData, 0);
    this.container.appendChild(rootEl);
  }

  buildNodeElement(node, depth) {
    const isFolder = node.type === 'folder';
    const wrapper = document.createElement('div');
    wrapper.className = 'tree-item-wrapper';

    const row = document.createElement('div');
    row.className = isFolder ? 'tree-folder-row' : 'tree-file-row';
    row.style.paddingLeft = `${depth * 14 + 8}px`;

    const fileIdentifier = node.fullPath || node.name;
    if (!isFolder && (fileIdentifier === this.activeFile || node.name === this.activeFile)) {
      row.classList.add('active');
    }

    if (isFolder) {
      const arrow = document.createElement('span');
      arrow.className = 'tree-arrow';
      arrow.innerHTML = node.expanded ? Icons.chevronDown : Icons.chevronRight;

      const folderIcon = document.createElement('span');
      folderIcon.className = 'tree-icon folder-icon';
      folderIcon.innerHTML = node.expanded ? Icons.folderOpen : Icons.folder;

      const label = document.createElement('span');
      label.className = 'tree-folder-label';
      label.textContent = node.name;

      row.appendChild(arrow);
      row.appendChild(folderIcon);
      row.appendChild(label);

      const childrenContainer = document.createElement('div');
      childrenContainer.className = 'tree-children';
      childrenContainer.style.display = node.expanded ? 'block' : 'none';

      if (node.children) {
        node.children.forEach(child => {
          childrenContainer.appendChild(this.buildNodeElement(child, depth + 1));
        });
      }

      row.addEventListener('click', (e) => {
        e.stopPropagation();
        node.expanded = !node.expanded;
        arrow.innerHTML = node.expanded ? Icons.chevronDown : Icons.chevronRight;
        folderIcon.innerHTML = node.expanded ? Icons.folderOpen : Icons.folder;
        childrenContainer.style.display = node.expanded ? 'block' : 'none';
      });

      wrapper.appendChild(row);
      wrapper.appendChild(childrenContainer);
    } else {
      const badge = document.createElement('span');
      badge.className = `tree-badge badge-${node.kind ? node.kind.toLowerCase() : 'c'}`;
      badge.textContent = node.kind || 'C';

      const label = document.createElement('span');
      label.className = 'tree-file-label';
      label.textContent = node.name;
      label.title = node.fullPath || node.name;

      row.setAttribute('data-node-id', node.nodeId || '');
      row.setAttribute('data-file-name', node.fullPath || node.name || '');
      row.setAttribute('data-base-name', node.name || '');

      row.appendChild(badge);
      row.appendChild(label);

      row.addEventListener('click', (e) => {
        e.stopPropagation();
        const targetPath = node.fullPath || node.name;
        this.selectFile(targetPath, node.nodeId);
        if (this.onFileSelect) {
          this.onFileSelect(targetPath, node);
        }
      });

      wrapper.appendChild(row);
    }

    return wrapper;
  }

  selectFile(fileName, nodeId = null) {
    this.activeFile = fileName;
    if (!this.container) return;

    const baseName = fileName ? fileName.replace(/\\/g, '/').split('/').pop() : null;

    const allFileRows = this.container.querySelectorAll('.tree-file-row');
    allFileRows.forEach(r => {
      const rowNodeId = r.getAttribute('data-node-id');
      const rowFileName = r.getAttribute('data-file-name');
      const rowBaseName = r.getAttribute('data-base-name');

      const isMatch = (nodeId && rowNodeId === nodeId) ||
                      (rowFileName === fileName) ||
                      (rowBaseName === fileName) ||
                      (baseName && rowBaseName === baseName) ||
                      (fileName && rowFileName && rowFileName.endsWith('/' + fileName));

      if (isMatch) {
        r.classList.add('active');
        r.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
        let parent = r.parentElement;
        while (parent && parent !== this.container) {
          if (parent.classList.contains('tree-children')) {
            parent.style.display = 'block';
            const folderRow = parent.previousElementSibling;
            if (folderRow) {
              const arrow = folderRow.querySelector('.tree-arrow');
              const icon = folderRow.querySelector('.folder-icon');
              if (arrow) arrow.innerHTML = Icons.chevronDown;
              if (icon) icon.innerHTML = Icons.folderOpen;
            }
          }
          parent = parent.parentElement;
        }
      } else {
        r.classList.remove('active');
      }
    });
  }
}

