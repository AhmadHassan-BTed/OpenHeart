/**
 * OpenHeart Dynamic File Hierarchy Tree Explorer (Zero Hardcoding)
 * Dynamically builds the VS Code / Android Studio directory tree from parsed compiler elements.
 */

export class FileTreeExplorer {
  constructor(containerId, onFileSelectCallback) {
    this.container = document.getElementById(containerId);
    this.onFileSelect = onFileSelectCallback;
    this.activeFile = null;
    this.treeData = {
      name: "src/main/java",
      type: "folder",
      expanded: true,
      children: []
    };
  }

  /**
   * Dynamically constructs the file tree from Cytoscape / PlantUML elements
   */
  updateFromElements(elements) {
    if (!elements || elements.length === 0) return;

    const root = {
      name: "src/main/java",
      type: "folder",
      expanded: true,
      children: []
    };

    const packageMap = new Map();
    elements.forEach(el => {
      if (el.data && el.data.isPackage) {
        const pkgPath = el.data.rawName || el.data.id.replace(/^pkg_/, '').replace(/_/g, '.');
        packageMap.set(el.data.id, {
          path: pkgPath,
          parent: el.data.parent,
          children: []
        });
      }
    });

    // Group files by parent package
    elements.forEach(el => {
      if (el.data && el.data.file && !el.data.isPackage && !el.data.source) {
        const fileName = el.data.file;
        const kind = el.data.kind || 'class';
        let kindLetter = 'C';
        if (fileName.endsWith('.kt')) kindLetter = 'K';
        else if (fileName.endsWith('.rs')) kindLetter = 'R';
        else if (fileName.endsWith('.ts') || fileName.endsWith('.tsx') || fileName.endsWith('.js')) kindLetter = 'T';
        else if (fileName.endsWith('.py')) kindLetter = 'P';
        else if (kind === 'interface') kindLetter = 'I';
        else if (kind === 'abstract') kindLetter = 'A';
        else if (kind === 'enum') kindLetter = 'E';
        else if (kind === 'actor' || kind === 'usecase') kindLetter = 'U';
        else if (kind === 'bb') kindLetter = 'B';
        else if (kind === 'timing_track') kindLetter = 'T';

        const fileEntry = {
          name: fileName,
          type: kind,
          kind: kindLetter,
          nodeId: el.data.id,
          data: el.data
        };

        const parentId = el.data.parent;
        if (parentId && packageMap.has(parentId)) {
          packageMap.get(parentId).children.push(fileEntry);
        } else {
          root.children.push(fileEntry);
        }
      }
    });

    // Build hierarchical folder nodes
    packageMap.forEach((pkgInfo, pkgId) => {
      if (pkgInfo.children.length > 0) {
        const folderNode = {
          name: pkgInfo.path,
          type: "folder",
          expanded: true,
          children: pkgInfo.children
        };
        root.children.push(folderNode);
      }
    });

    // Sort folders and files
    root.children.sort((a, b) => a.name.localeCompare(b.name));

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

    if (!isFolder && node.name === this.activeFile) {
      row.classList.add('active');
    }

    if (isFolder) {
      const arrow = document.createElement('span');
      arrow.className = 'tree-arrow';
      arrow.textContent = node.expanded ? '▼' : '▶';

      const folderIcon = document.createElement('span');
      folderIcon.className = 'tree-icon folder-icon';
      folderIcon.textContent = node.expanded ? '📂' : '📁';

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
        arrow.textContent = node.expanded ? '▼' : '▶';
        folderIcon.textContent = node.expanded ? '📂' : '📁';
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

      row.setAttribute('data-node-id', node.nodeId || '');
      row.setAttribute('data-file-name', node.name || '');

      row.appendChild(badge);
      row.appendChild(label);

      row.addEventListener('click', (e) => {
        e.stopPropagation();
        this.selectFile(node.name, node.nodeId);
        if (this.onFileSelect) {
          this.onFileSelect(node.name, node);
        }
      });

      wrapper.appendChild(row);
    }

    return wrapper;
  }

  selectFile(fileName, nodeId = null) {
    this.activeFile = fileName;
    if (!this.container) return;

    const allFileRows = this.container.querySelectorAll('.tree-file-row');
    allFileRows.forEach(r => {
      const rowNodeId = r.getAttribute('data-node-id');
      const rowFileName = r.getAttribute('data-file-name');
      const isMatch = (nodeId && rowNodeId === nodeId) || (rowFileName === fileName);

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
              if (arrow) arrow.textContent = '▼';
              if (icon) icon.textContent = '📂';
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
