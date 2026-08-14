/**
 * OpenHeart Web Studio — Studio State Store
 * Single source of truth managing studio selection, diagrams, tabs, view modes, and traceability metadata.
 */

export class StudioStateStore {
  constructor() {
    this.selectedDiagrams = new Set(['class', 'object', 'component', 'package', 'activity', 'sequence']);
    this.generatedDiagrams = {};
    this.currentActiveTab = 'class';
    this.currentViewMode = 'visual'; // 'visual' | 'code' | 'matrix'
    this.currentZoom = 1.0;
    this.activeTraceabilityList = [];
    this.activeRepoUrl = '';
    this.isStudioEngaged = false;
    this.listeners = [];
  }

  subscribe(listener) {
    this.listeners.push(listener);
  }

  notify(event, data) {
    for (const listener of this.listeners) {
      listener(event, data, this);
    }
  }

  setRepoUrl(url) {
    this.activeRepoUrl = url;
    this.notify('repo_url_changed', url);
  }

  setSelectedDiagrams(set) {
    this.selectedDiagrams = new Set(set);
    this.notify('diagrams_selected', this.selectedDiagrams);
  }

  addDiagramType(type) {
    this.selectedDiagrams.add(type);
    this.notify('diagrams_selected', this.selectedDiagrams);
  }

  removeDiagramType(type) {
    this.selectedDiagrams.delete(type);
    this.notify('diagrams_selected', this.selectedDiagrams);
  }

  setGeneratedDiagrams(diagrams) {
    this.generatedDiagrams = diagrams || {};
    this.notify('diagrams_generated', this.generatedDiagrams);
  }

  setActiveTab(tab) {
    this.currentActiveTab = tab;
    this.notify('tab_changed', tab);
  }

  setViewMode(mode) {
    this.currentViewMode = mode;
    this.notify('view_mode_changed', mode);
  }

  setZoom(zoom) {
    this.currentZoom = zoom;
    this.notify('zoom_changed', zoom);
  }

  setTraceabilityList(list) {
    this.activeTraceabilityList = list || [];
    window.activeTraceabilityList = this.activeTraceabilityList;
    this.notify('traceability_updated', this.activeTraceabilityList);
  }
}

export const StudioState = new StudioStateStore();
