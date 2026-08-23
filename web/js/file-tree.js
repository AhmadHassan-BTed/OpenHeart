/**
 * OpenHeart VS Code / Android Studio Style File Hierarchy Tree Explorer
 * Provides interactive directory tree with folder expansion, type icons, and Monaco sync.
 */

export class FileTreeExplorer {
  constructor(containerId, onFileSelectCallback) {
    this.container = document.getElementById(containerId);
    this.onFileSelect = onFileSelectCallback;
    this.activeFile = 'VideoConversionFacade.java';

    this.treeData = {
      name: "src/main/java",
      type: "folder",
      expanded: true,
      children: [
        {
          name: "com.patterns.behavioral",
          type: "folder",
          expanded: true,
          children: [
            {
              name: "observer",
              type: "folder",
              expanded: true,
              children: [
                { name: "NewsAgency.java", type: "class", kind: "C" },
                { name: "NewsChannel.java", type: "class", kind: "C" },
                { name: "Observer.java", type: "interface", kind: "I" },
                { name: "Subject.java", type: "interface", kind: "I" }
              ]
            },
            {
              name: "strategy",
              type: "folder",
              expanded: true,
              children: [
                { name: "CreditCardStrategy.java", type: "class", kind: "C" },
                { name: "PaymentStrategy.java", type: "interface", kind: "I" },
                { name: "PaypalStrategy.java", type: "class", kind: "C" },
                { name: "ShoppingCart.java", type: "class", kind: "C" }
              ]
            },
            {
              name: "templatemethod",
              type: "folder",
              expanded: true,
              children: [
                { name: "CsvDataMiner.java", type: "class", kind: "C" },
                { name: "DataMiner.java", type: "abstract", kind: "A" },
                { name: "PdfDataMiner.java", type: "class", kind: "C" }
              ]
            }
          ]
        },
        {
          name: "com.patterns.creational",
          type: "folder",
          expanded: true,
          children: [
            {
              name: "builder",
              type: "folder",
              expanded: true,
              children: [
                { name: "Computer.java", type: "class", kind: "C" },
                { name: "ComputerBuilder.java", type: "class", kind: "C" },
                { name: "Director.java", type: "class", kind: "C" }
              ]
            },
            {
              name: "factory",
              type: "folder",
              expanded: true,
              children: [
                { name: "Logistics.java", type: "abstract", kind: "A" },
                { name: "RoadLogistics.java", type: "class", kind: "C" },
                { name: "SeaLogistics.java", type: "class", kind: "C" },
                { name: "Ship.java", type: "class", kind: "C" },
                { name: "Transport.java", type: "interface", kind: "I" },
                { name: "Truck.java", type: "class", kind: "C" }
              ]
            },
            {
              name: "singleton",
              type: "folder",
              expanded: true,
              children: [
                { name: "DatabaseConnectionPool.java", type: "class", kind: "C" }
              ]
            }
          ]
        },
        {
          name: "com.patterns.structural",
          type: "folder",
          expanded: true,
          children: [
            {
              name: "adapter",
              type: "folder",
              expanded: true,
              children: [
                { name: "AdvancedMediaPlayer.java", type: "interface", kind: "I" },
                { name: "AudioPlayer.java", type: "class", kind: "C" },
                { name: "MediaAdapter.java", type: "class", kind: "C" },
                { name: "MediaPlayer.java", type: "interface", kind: "I" },
                { name: "Mp4Player.java", type: "class", kind: "C" },
                { name: "VlcPlayer.java", type: "class", kind: "C" }
              ]
            },
            {
              name: "decorator",
              type: "folder",
              expanded: true,
              children: [
                { name: "Beverage.java", type: "interface", kind: "I" },
                { name: "CondimentDecorator.java", type: "abstract", kind: "A" },
                { name: "Espresso.java", type: "class", kind: "C" },
                { name: "Mocha.java", type: "class", kind: "C" },
                { name: "Whip.java", type: "class", kind: "C" }
              ]
            },
            {
              name: "facade",
              type: "folder",
              expanded: true,
              children: [
                { name: "AudioMixer.java", type: "class", kind: "C" },
                { name: "BitrateReader.java", type: "class", kind: "C" },
                { name: "VideoConversionFacade.java", type: "class", kind: "C" }
              ]
            }
          ]
        }
      ]
    };
  }

  render() {
    if (!this.container) return;
    this.container.innerHTML = '';
    const rootEl = this.renderNode(this.treeData, 0);
    this.container.appendChild(rootEl);
  }

  renderNode(node, depth) {
    const el = document.createElement('div');
    el.className = 'tree-node';

    if (node.type === 'folder') {
      const header = document.createElement('div');
      header.className = 'tree-folder-header';
      header.style.paddingLeft = `${depth * 14 + 8}px`;

      const arrow = document.createElement('span');
      arrow.className = `tree-arrow ${node.expanded ? 'expanded' : ''}`;
      arrow.textContent = node.expanded ? '▼' : '▶';

      const icon = document.createElement('span');
      icon.className = 'tree-folder-icon';
      icon.textContent = '📁';

      const label = document.createElement('span');
      label.className = 'tree-folder-label';
      label.textContent = node.name;

      header.appendChild(arrow);
      header.appendChild(icon);
      header.appendChild(label);

      const childrenContainer = document.createElement('div');
      childrenContainer.className = 'tree-children';
      childrenContainer.style.display = node.expanded ? 'block' : 'none';

      header.addEventListener('click', () => {
        node.expanded = !node.expanded;
        arrow.textContent = node.expanded ? '▼' : '▶';
        arrow.className = `tree-arrow ${node.expanded ? 'expanded' : ''}`;
        childrenContainer.style.display = node.expanded ? 'block' : 'none';
      });

      if (node.children) {
        node.children.forEach(child => {
          childrenContainer.appendChild(this.renderNode(child, depth + 1));
        });
      }

      el.appendChild(header);
      el.appendChild(childrenContainer);
    } else {
      const fileRow = document.createElement('div');
      fileRow.className = `tree-file-row ${node.name === this.activeFile ? 'active' : ''}`;
      fileRow.style.paddingLeft = `${depth * 14 + 18}px`;
      fileRow.setAttribute('data-filename', node.name);

      const badge = document.createElement('span');
      badge.className = `tree-badge badge-${node.kind.toLowerCase()}`;
      badge.textContent = node.kind;

      const label = document.createElement('span');
      label.className = 'tree-file-label';
      label.textContent = node.name;

      fileRow.appendChild(badge);
      fileRow.appendChild(label);

      fileRow.addEventListener('click', () => {
        this.selectFile(node.name);
        if (this.onFileSelect) {
          this.onFileSelect(node.name);
        }
      });

      el.appendChild(fileRow);
    }

    return el;
  }

  selectFile(fileName) {
    this.activeFile = fileName;
    if (!this.container) return;
    this.container.querySelectorAll('.tree-file-row').forEach(row => {
      if (row.getAttribute('data-filename') === fileName) {
        row.classList.add('active');
        row.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      } else {
        row.classList.remove('active');
      }
    });
  }
}
