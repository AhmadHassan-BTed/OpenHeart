/**
 * OpenHeart Web Studio — UI Controller Module
 * Manages DOM interaction, preset matrix cards, toolbar actions, modals, shortcuts, and toasts.
 */

import { Logger } from './logger.js';
import { StudioState } from './state.js';
import { APIClient } from './api.js';
import { OrbEngine } from './orb.js';

export class StudioUIController {
  init() {
    this.bindButtons();
    this.bindCheckboxes();
    this.bindPresets();
    this.bindCategoryTabs();
    this.bindViewModes();
    this.bindZoomControls();
    this.bindSampleChips();
    this.bindModals();
    this.bindKeyboardShortcuts();

    StudioState.subscribe((event, data) => {
      if (event === 'diagrams_selected') {
        this.updateCountDisplay();
      } else if (event === 'diagrams_generated') {
        this.renderStudioTabs();
      }
    });
  }

  showToast(message) {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const toast = document.createElement('div');
    toast.className = 'toast-message';
    toast.textContent = message;
    container.appendChild(toast);
    setTimeout(() => {
      if (toast.parentNode) toast.parentNode.removeChild(toast);
    }, 3000);
  }

  bindButtons() {
    const btnEngageOrb = document.getElementById('btn-engage-orb');
    const orbCanvas = document.getElementById('orb-canvas');
    const heroLanding = document.getElementById('hero-landing');
    const studioApp = document.getElementById('studio-app');
    const btnBackLanding = document.getElementById('btn-back-landing');
    const cornerBrandTopRight = document.querySelector('.corner-brand.top-right');
    const btnFetch = document.getElementById('btn-fetch');
    const repoUrlInput = document.getElementById('repo-url-input');

    const engageStudio = () => {
      StudioState.isStudioEngaged = true;
      OrbEngine.setStudioEngaged(true);
      if (heroLanding) heroLanding.classList.add('hero-hidden');
      if (cornerBrandTopRight) cornerBrandTopRight.classList.add('corner-hidden');
      setTimeout(() => {
        if (studioApp) studioApp.classList.remove('studio-hidden');
      }, 400);
    };

    if (btnEngageOrb) btnEngageOrb.addEventListener('click', engageStudio);
    if (orbCanvas) orbCanvas.addEventListener('click', engageStudio);

    if (btnBackLanding) {
      btnBackLanding.addEventListener('click', () => {
        StudioState.isStudioEngaged = false;
        OrbEngine.setStudioEngaged(false);
        if (studioApp) studioApp.classList.add('studio-hidden');
        if (cornerBrandTopRight) cornerBrandTopRight.classList.remove('corner-hidden');
        setTimeout(() => {
          if (heroLanding) heroLanding.classList.remove('hero-hidden');
        }, 300);
      });
    }

    if (btnFetch) {
      btnFetch.addEventListener('click', () => {
        const url = repoUrlInput ? repoUrlInput.value.trim() : '';
        if (!url || !url.startsWith('https://github.com/')) {
          this.showToast('⚠️ Please enter a valid GitHub repository URL.');
          return;
        }

        if (StudioState.selectedDiagrams.size === 0) {
          this.showToast('⚠️ Please select at least one UML diagram projection.');
          return;
        }

        StudioState.setRepoUrl(url);
        this.runPipelineExecution(url);
      });
    }

    // Export & Copy Buttons
    const btnCopy = document.getElementById('btn-copy-plantuml');
    const btnExportPuml = document.getElementById('btn-export-puml');
    const btnExportXmi = document.getElementById('btn-export-xmi');
    const btnExportJson = document.getElementById('btn-export-json');

    if (btnCopy) {
      btnCopy.addEventListener('click', () => {
        const code = StudioState.generatedDiagrams[StudioState.currentActiveTab];
        if (code) {
          navigator.clipboard.writeText(code);
          this.showToast('📋 Diagram source copied to clipboard!');
        }
      });
    }

    if (btnExportPuml) {
      btnExportPuml.addEventListener('click', () => {
        const code = StudioState.generatedDiagrams[StudioState.currentActiveTab];
        if (code) {
          this.downloadFile(`openheart_${StudioState.currentActiveTab}.puml`, code, 'text/plain');
          this.showToast(`💾 Downloaded ${StudioState.currentActiveTab}.puml`);
        }
      });
    }

    if (btnExportXmi) {
      btnExportXmi.addEventListener('click', () => {
        const code = `<?xml version="1.0" encoding="UTF-8"?>\n<xmi:XMI xmi:version="2.5" xmlns:uml="http://www.omg.org/spec/UML/20131001">\n  <!-- OpenHeart XMI Export -->\n</xmi:XMI>`;
        this.downloadFile(`openheart_${StudioState.currentActiveTab}.xmi`, code, 'application/xml');
        this.showToast(`💾 Exported XMI 2.5 metadata.`);
      });
    }

    if (btnExportJson) {
      btnExportJson.addEventListener('click', () => {
        const json = JSON.stringify(StudioState.generatedDiagrams, null, 2);
        this.downloadFile(`openheart_analysis.json`, json, 'application/json');
        this.showToast(`💾 Exported analysis JSON.`);
      });
    }
  }

  downloadFile(filename, content, mimeType) {
    const blob = new Blob([content], { type: `${mimeType};charset=utf-8` });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  bindSampleChips() {
    document.querySelectorAll('.btn-sample-chip').forEach(chip => {
      chip.addEventListener('click', () => {
        const url = chip.getAttribute('data-url');
        const repoUrlInput = document.getElementById('repo-url-input');
        if (repoUrlInput) {
          repoUrlInput.value = url;
        }
        this.setPreset(['class', 'object', 'component', 'deployment', 'package', 'composite', 'profile', 'usecase', 'activity', 'statemachine', 'sequence', 'communication', 'interaction', 'timing']);
        this.showToast(`⚡ Loaded sample: ${chip.textContent.trim()}`);
        this.runPipelineExecution(url);
      });
    });
  }

  bindModals() {
    const theoryModal = document.getElementById('theory-modal');
    const btnOpenTheory = document.getElementById('btn-open-theory-modal');
    const btnCloseTheory = document.getElementById('btn-close-theory');

    const shortcutsModal = document.getElementById('shortcuts-modal');
    const btnOpenShortcuts = document.getElementById('btn-open-shortcuts-modal');
    const btnCloseShortcuts = document.getElementById('btn-close-shortcuts');

    if (btnOpenTheory && theoryModal) {
      btnOpenTheory.addEventListener('click', () => {
        theoryModal.classList.remove('hidden-modal');
      });
    }
    if (btnCloseTheory && theoryModal) {
      btnCloseTheory.addEventListener('click', () => {
        theoryModal.classList.add('hidden-modal');
      });
    }

    if (btnOpenShortcuts && shortcutsModal) {
      btnOpenShortcuts.addEventListener('click', () => {
        shortcutsModal.classList.remove('hidden-modal');
      });
    }
    if (btnCloseShortcuts && shortcutsModal) {
      btnCloseShortcuts.addEventListener('click', () => {
        shortcutsModal.classList.add('hidden-modal');
      });
    }

    // Click outside modal to close
    [theoryModal, shortcutsModal].forEach(modal => {
      if (modal) {
        modal.addEventListener('click', (e) => {
          if (e.target === modal) {
            modal.classList.add('hidden-modal');
          }
        });
      }
    });
  }

  bindKeyboardShortcuts() {
    const structuralDiagrams = ['class', 'object', 'component', 'deployment', 'package', 'composite', 'profile'];
    const behavioralDiagrams = ['usecase', 'activity', 'statemachine', 'sequence', 'communication', 'interaction', 'timing'];

    window.addEventListener('keydown', (e) => {
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') {
        return;
      }

      if (e.key === 'Escape') {
        const theoryModal = document.getElementById('theory-modal');
        const shortcutsModal = document.getElementById('shortcuts-modal');
        if (theoryModal) theoryModal.classList.add('hidden-modal');
        if (shortcutsModal) shortcutsModal.classList.add('hidden-modal');
        return;
      }

      if (e.key === '?') {
        const shortcutsModal = document.getElementById('shortcuts-modal');
        if (shortcutsModal) {
          shortcutsModal.classList.toggle('hidden-modal');
        }
        return;
      }

      // 1-7 for Structural, Shift+1-7 for Behavioral
      const num = parseInt(e.key, 10);
      if (!isNaN(num) && num >= 1 && num <= 7) {
        const targetList = e.shiftKey ? behavioralDiagrams : structuralDiagrams;
        const targetDiagram = targetList[num - 1];
        if (targetDiagram && StudioState.generatedDiagrams[targetDiagram]) {
          this.activateTabByDiagramType(targetDiagram);
          this.showToast(`Switched to ${targetDiagram.toUpperCase()} Diagram`);
        }
      }

      // V/C/M for View Modes
      if (e.key === 'v' || e.key === 'V') {
        const btn = document.getElementById('view-mode-visual');
        if (btn) btn.click();
        this.showToast('Mode: Visual Diagram');
      } else if (e.key === 'c' || e.key === 'C') {
        const btn = document.getElementById('view-mode-code');
        if (btn) btn.click();
        this.showToast('Mode: Source Code');
      } else if (e.key === 'm' || e.key === 'M') {
        const btn = document.getElementById('view-mode-matrix');
        if (btn) btn.click();
        this.showToast('Mode: Symbol Matrix');
      } else if (e.key === '+' || e.key === '=') {
        const btn = document.getElementById('btn-zoom-in');
        if (btn) btn.click();
      } else if (e.key === '-' || e.key === '_') {
        const btn = document.getElementById('btn-zoom-out');
        if (btn) btn.click();
      } else if (e.key === '0') {
        const btn = document.getElementById('btn-zoom-reset');
        if (btn) btn.click();
      }
    });
  }

  activateTabByDiagramType(type) {
    document.querySelectorAll('.tab-btn').forEach(btn => {
      if (btn.getAttribute('data-type') === type) {
        document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        StudioState.setActiveTab(type);
      }
    });
  }

  bindCheckboxes() {
    document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
      cb.addEventListener('change', (e) => {
        const parentCard = e.target.closest('.brutalist-checkbox');
        if (e.target.checked) {
          StudioState.addDiagramType(e.target.value);
          if (parentCard) parentCard.classList.add('active');
        } else {
          StudioState.removeDiagramType(e.target.value);
          if (parentCard) parentCard.classList.remove('active');
        }
      });
    });
  }

  bindPresets() {
    const btnPresetCore = document.getElementById('btn-preset-core');
    const btnSelectAll = document.getElementById('btn-select-all');
    const btnClearAll = document.getElementById('btn-clear-all');

    if (btnPresetCore) {
      btnPresetCore.addEventListener('click', () => {
        this.setPreset(['class', 'sequence', 'activity', 'component']);
        this.setActivePresetBtn(btnPresetCore);
      });
    }

    if (btnSelectAll) {
      btnSelectAll.addEventListener('click', () => {
        const all = ['class', 'object', 'component', 'deployment', 'package', 'composite', 'profile', 'usecase', 'activity', 'statemachine', 'sequence', 'communication', 'interaction', 'timing'];
        this.setPreset(all);
        this.setActivePresetBtn(btnSelectAll);
      });
    }

    if (btnClearAll) {
      btnClearAll.addEventListener('click', () => {
        this.setPreset([]);
        this.setActivePresetBtn(btnClearAll);
      });
    }
  }

  bindCategoryTabs() {
    const tabStructural = document.getElementById('tab-structural');
    const tabBehavioral = document.getElementById('tab-behavioral');
    const viewStructural = document.getElementById('view-structural');
    const viewBehavioral = document.getElementById('view-behavioral');

    if (tabStructural && tabBehavioral && viewStructural && viewBehavioral) {
      tabStructural.addEventListener('click', () => {
        tabStructural.classList.add('active');
        tabBehavioral.classList.remove('active');
        viewStructural.classList.remove('hidden');
        viewBehavioral.classList.add('hidden');
      });

      tabBehavioral.addEventListener('click', () => {
        tabBehavioral.classList.add('active');
        tabStructural.classList.remove('active');
        viewBehavioral.classList.remove('hidden');
        viewStructural.classList.add('hidden');
      });
    }
  }

  setPreset(array) {
    StudioState.setSelectedDiagrams(array);
    document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
      const isChecked = StudioState.selectedDiagrams.has(cb.value);
      cb.checked = isChecked;
      const parentCard = cb.closest('.brutalist-checkbox');
      if (parentCard) {
        if (isChecked) parentCard.classList.add('active');
        else parentCard.classList.remove('active');
      }
    });
  }

  setActivePresetBtn(btn) {
    document.querySelectorAll('.btn-preset').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
  }

  bindViewModes() {
    const btnVisual = document.getElementById('view-mode-visual');
    const btnCode = document.getElementById('view-mode-code');
    const btnMatrix = document.getElementById('view-mode-matrix');

    const updateBtns = (activeMode) => {
      if (btnVisual) btnVisual.className = `btn-scope ${activeMode === 'visual' ? 'active' : ''}`;
      if (btnCode) btnCode.className = `btn-scope ${activeMode === 'code' ? 'active' : ''}`;
      if (btnMatrix) btnMatrix.className = `btn-scope ${activeMode === 'matrix' ? 'active' : ''}`;
    };

    if (btnVisual) {
      btnVisual.addEventListener('click', () => {
        StudioState.setViewMode('visual');
        updateBtns('visual');
      });
    }

    if (btnCode) {
      btnCode.addEventListener('click', () => {
        StudioState.setViewMode('code');
        updateBtns('code');
      });
    }

    if (btnMatrix) {
      btnMatrix.addEventListener('click', () => {
        StudioState.setViewMode('matrix');
        updateBtns('matrix');
      });
    }
  }

  bindZoomControls() {
    const btnZoomIn = document.getElementById('btn-zoom-in');
    const btnZoomOut = document.getElementById('btn-zoom-out');
    const btnZoomReset = document.getElementById('btn-zoom-reset');

    if (btnZoomIn) {
      btnZoomIn.addEventListener('click', () => {
        StudioState.setZoom(Math.min(StudioState.currentZoom + 0.25, 4.0));
      });
    }

    if (btnZoomOut) {
      btnZoomOut.addEventListener('click', () => {
        StudioState.setZoom(Math.max(StudioState.currentZoom - 0.25, 0.25));
      });
    }

    if (btnZoomReset) {
      btnZoomReset.addEventListener('click', () => {
        StudioState.setZoom(1.0);
      });
    }
  }

  updateCountDisplay() {
    const activeDiagramCount = document.getElementById('active-diagram-count');
    if (activeDiagramCount) {
      activeDiagramCount.textContent = StudioState.selectedDiagrams.size;
    }
  }

  async runPipelineExecution(url) {
    const pipelineStatus = document.getElementById('pipeline-status');
    const statusStepTitle = document.getElementById('status-step-title');
    const progressBarFill = document.getElementById('progress-bar-fill');
    const statusPercent = document.getElementById('status-percent');

    if (pipelineStatus) pipelineStatus.classList.remove('hidden');
    Logger.clear();

    if (progressBarFill) progressBarFill.style.width = '10%';
    if (statusPercent) statusPercent.textContent = '10%';
    if (statusStepTitle) statusStepTitle.textContent = 'STAGE 1: SENDING REPOSITORY TO RUST BACKEND...';

    const result = await APIClient.analyzeRepository(url, StudioState.selectedDiagrams);

    if (result && result.diagrams) {
      if (progressBarFill) progressBarFill.style.width = '100%';
      if (statusPercent) statusPercent.textContent = '100%';
      if (statusStepTitle) statusStepTitle.textContent = 'ANALYSIS COMPLETE :: SCPG ARTIFACT RENDERED';

      StudioState.setTraceabilityList(result.traceability || []);
      StudioState.setGeneratedDiagrams(result.diagrams);

      this.showToast('✅ 10-Phase SCPG Compilation Complete!');
      await this.sleep(400);
      if (pipelineStatus) pipelineStatus.classList.add('hidden');
    } else {
      if (statusStepTitle) statusStepTitle.textContent = 'ERROR IN PIPELINE EXECUTION';
      this.showToast('❌ Error in pipeline execution.');
    }
  }

  renderStudioTabs() {
    const diagramTabs = document.getElementById('diagram-tabs');
    if (!diagramTabs) return;

    diagramTabs.innerHTML = '';
    const selectedArray = Array.from(StudioState.selectedDiagrams);

    selectedArray.forEach((type, index) => {
      const btn = document.createElement('button');
      btn.className = `tab-btn ${index === 0 ? 'active' : ''}`;
      btn.setAttribute('data-type', type);
      btn.textContent = this.getDiagramLabel(type);
      btn.addEventListener('click', () => {
        document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        StudioState.setActiveTab(type);
      });
      diagramTabs.appendChild(btn);
    });

    if (selectedArray.length > 0) {
      StudioState.setActiveTab(selectedArray[0]);
    }
  }

  getDiagramLabel(type) {
    const labels = {
      class: 'CLASS',
      object: 'OBJECT',
      component: 'COMPONENT',
      deployment: 'DEPLOYMENT',
      package: 'PACKAGE',
      composite: 'COMPOSITE',
      profile: 'PROFILE',
      usecase: 'USE CASE',
      activity: 'ACTIVITY',
      statemachine: 'STATE MACHINE',
      sequence: 'SEQUENCE',
      communication: 'COMMUNICATION',
      interaction: 'INTERACTION OVERVIEW',
      timing: 'TIMING'
    };
    return labels[type] || type.toUpperCase();
  }

  sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}

export const UIController = new StudioUIController();
