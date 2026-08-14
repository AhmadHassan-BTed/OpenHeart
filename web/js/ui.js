/**
 * OpenHeart Web Studio — UI Controller Module
 * Manages DOM interaction, preset matrix cards, toolbar actions, and pipeline progress bars.
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
    this.bindViewModes();
    this.bindZoomControls();

    StudioState.subscribe((event, data) => {
      if (event === 'diagrams_selected') {
        this.updateCountDisplay();
      } else if (event === 'diagrams_generated') {
        this.renderStudioTabs();
      }
    });
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
          alert('Please enter a valid GitHub repository URL starting with https://github.com/');
          return;
        }

        if (StudioState.selectedDiagrams.size === 0) {
          alert('Please select at least one UML diagram projection.');
          return;
        }

        StudioState.setRepoUrl(url);
        this.runPipelineExecution(url);
      });
    }

    // Export & Copy Buttons
    const btnCopy = document.getElementById('btn-copy-plantuml');
    const btnExportPuml = document.getElementById('btn-export-puml');

    if (btnCopy) {
      btnCopy.addEventListener('click', () => {
        const code = StudioState.generatedDiagrams[StudioState.currentActiveTab];
        if (code) {
          navigator.clipboard.writeText(code);
          alert('PlantUML diagram source copied to clipboard!');
        }
      });
    }

    if (btnExportPuml) {
      btnExportPuml.addEventListener('click', () => {
        const code = StudioState.generatedDiagrams[StudioState.currentActiveTab];
        if (code) {
          const blob = new Blob([code], { type: 'text/plain;charset=utf-8' });
          const a = document.createElement('a');
          a.href = URL.createObjectURL(blob);
          a.download = `openheart_${StudioState.currentActiveTab}_diagram.puml`;
          document.body.appendChild(a);
          a.click();
          document.body.removeChild(a);
        }
      });
    }
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
    const btnPresetQuick = document.getElementById('btn-preset-quick');
    const btnSelectAll = document.getElementById('btn-select-all');
    const btnClearAll = document.getElementById('btn-clear-all');

    if (btnPresetQuick) {
      btnPresetQuick.addEventListener('click', () => {
        this.setPreset(['class', 'sequence', 'activity', 'component']);
        this.setActivePresetBtn(btnPresetQuick);
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

      await this.sleep(400);
      if (pipelineStatus) pipelineStatus.classList.add('hidden');
    } else {
      if (statusStepTitle) statusStepTitle.textContent = 'ERROR IN PIPELINE EXECUTION';
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
