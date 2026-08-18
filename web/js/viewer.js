/**
 * OpenHeart Web Studio — Diagram Viewer Module
 * High-cohesion module for Kroki vector SVG rendering, scrollable code editing, and symbol matrix table.
 */

import { Logger } from './logger.js';
import { StudioState } from './state.js';
import { APIClient } from './api.js';

export class DiagramViewerModule {
  constructor() {
    this.renderContainer = null;
    this.panState = { isPanning: false, startX: 0, startY: 0, scrollLeft: 0, scrollTop: 0 };
  }

  init(containerId = 'plantuml-render-container') {
    this.renderContainer = document.getElementById(containerId);
    
    // Subscribe to state events
    StudioState.subscribe((event, data) => {
      if (event === 'tab_changed' || event === 'view_mode_changed') {
        this.renderCurrentDiagram();
      } else if (event === 'zoom_changed') {
        this.applyZoom(data);
      }
    });
  }

  escapeHtml(text) {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  applyZoom(zoomLevel = StudioState.currentZoom) {
    if (!this.renderContainer) return;
    const svg = this.renderContainer.querySelector('svg');
    if (svg) {
      svg.style.transform = `scale(${zoomLevel})`;
      svg.style.transformOrigin = 'center center';
      svg.style.transition = 'transform 0.15s ease-out';
    }
  }

  setupInteractivePanAndZoom(viewport) {
    if (!viewport) return;
    viewport.style.cursor = 'grab';
    viewport.style.overflow = 'auto';

    viewport.addEventListener('mousedown', (e) => {
      this.panState.isPanning = true;
      viewport.style.cursor = 'grabbing';
      this.panState.startX = e.pageX - viewport.offsetLeft;
      this.panState.startY = e.pageY - viewport.offsetTop;
      this.panState.scrollLeft = viewport.scrollLeft;
      this.panState.scrollTop = viewport.scrollTop;
    });

    viewport.addEventListener('mouseleave', () => {
      this.panState.isPanning = false;
      viewport.style.cursor = 'grab';
    });

    viewport.addEventListener('mouseup', () => {
      this.panState.isPanning = false;
      viewport.style.cursor = 'grab';
    });

    viewport.addEventListener('mousemove', (e) => {
      if (!this.panState.isPanning) return;
      e.preventDefault();
      const x = e.pageX - viewport.offsetLeft;
      const y = e.pageY - viewport.offsetTop;
      const walkX = (x - this.panState.startX) * 1.5;
      const walkY = (y - this.panState.startY) * 1.5;
      viewport.scrollLeft = this.panState.scrollLeft - walkX;
      viewport.scrollTop = this.panState.scrollTop - walkY;
    });

    viewport.addEventListener('wheel', (e) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        let current = StudioState.currentZoom || 1.0;
        if (e.deltaY < 0) {
          current = Math.min(2.5, current + 0.1);
        } else {
          current = Math.max(0.4, current - 0.1);
        }
        StudioState.setZoom(current);
      }
    }, { passive: false });
  }

  async renderCurrentDiagram() {
    if (!this.renderContainer) return;
    
    const type = StudioState.currentActiveTab || 'class';
    const pumlCode = StudioState.generatedDiagrams[type] || `@startuml\n' ${type} Diagram\n@enduml`;
    const mode = StudioState.currentViewMode || 'visual';

    Logger.log(`[VIEWER] Rendering projection '${type}' in mode '${mode}'...`);

    if (mode === 'code') {
      this.renderContainer.innerHTML = `
        <div class="plantuml-canvas" style="height:100%; overflow:auto;">
          <pre class="plantuml-code-editor"><code>${this.escapeHtml(pumlCode)}</code></pre>
        </div>`;
    } else if (mode === 'matrix') {
      const traceItems = StudioState.activeTraceabilityList || [];
      this.renderContainer.innerHTML = `
        <div class="plantuml-canvas" style="padding: 1.5rem; overflow: auto; background: #0d0d0d; height:100%;">
          <h4 style="color: #00ff66; margin-top:0; font-family: monospace;">UML SYMBOL & SPAN MATRIX (${type.toUpperCase()})</h4>
          <table style="width: 100%; border-collapse: collapse; color: #ccc; font-family: monospace; font-size: 0.85rem;">
            <thead>
              <tr style="border-bottom: 2px solid #333; text-align: left;">
                <th style="padding: 8px;">TOKEN ID</th>
                <th style="padding: 8px;">FILE PATH</th>
                <th style="padding: 8px;">LINE SPAN</th>
                <th style="padding: 8px;">SCPG HASH</th>
              </tr>
            </thead>
            <tbody>
              ${traceItems.length > 0 ? traceItems.map(item => `
                <tr style="border-bottom: 1px solid #222;">
                  <td style="padding: 8px; color: #00ff66;">#${item.tid}</td>
                  <td style="padding: 8px;">${this.escapeHtml(item.file)}</td>
                  <td style="padding: 8px;">${this.escapeHtml(item.span)}</td>
                  <td style="padding: 8px; color: #888;">${item.hash}</td>
                </tr>
              `).join('') : `
                <tr><td colspan="4" style="padding: 1rem; color: #888;">No traceability symbols extracted for this projection.</td></tr>
              `}
            </tbody>
          </table>
        </div>`;
    } else {
      // 'visual' Vector SVG mode via Kroki
      this.renderContainer.innerHTML = `
        <div class="plantuml-canvas" style="height:100%; display:flex; flex-direction:column; overflow:hidden;">
          <div id="visual-loading" style="padding: 2rem; color: #00ff66; font-family: monospace;">
            > RENDERING INTERACTIVE VECTOR SVG DIAGRAM VIA KROKI ENGINE...
          </div>
          <div id="visual-viewport" class="plantuml-visual-viewport" style="display:none; flex:1; overflow:auto; padding:2rem; justify-content:center; align-items:center;"></div>
        </div>`;

      try {
        const svgText = await APIClient.renderKrokiSVG(pumlCode);
        const viewport = document.getElementById("visual-viewport");
        const loading = document.getElementById("visual-loading");
        if (viewport && loading) {
          loading.style.display = "none";
          viewport.style.display = "flex";
          viewport.innerHTML = svgText;

          const svg = viewport.querySelector('svg');
          if (svg) {
            svg.removeAttribute('height');
            svg.style.maxWidth = '100%';
            svg.style.height = 'auto';
            svg.style.display = 'block';
            svg.style.margin = 'auto';
            svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
          }

          this.applyZoom(StudioState.currentZoom);
          this.setupInteractivePanAndZoom(viewport);
        }
      } catch (err) {
        Logger.warn(`[KROKI FALLBACK] ${err.message}. Rendering scrollable PlantUML code.`);
        this.renderContainer.innerHTML = `
          <div class="plantuml-canvas" style="height:100%; overflow:auto;">
            <div style="padding: 0.75rem 1.5rem; background: #1a1a1a; color: #ffaa00; font-family: monospace; font-size: 0.8rem; border-bottom: 1px solid #333;">
              ⚠️ VECTOR SVG GENERATOR OFFLINE. DISPLAYING 100% SCROLLABLE PLANTUML SOURCE CODE.
            </div>
            <pre class="plantuml-code-editor"><code>${this.escapeHtml(pumlCode)}</code></pre>
          </div>`;
      }
    }

    // Update Traceability Footer Cards
    if (StudioState.activeTraceabilityList && StudioState.activeTraceabilityList.length > 0) {
      const idx = Math.abs(type.length) % StudioState.activeTraceabilityList.length;
      const item = StudioState.activeTraceabilityList[idx];
      const tidEl = document.getElementById("trace-tid");
      const fileEl = document.getElementById("trace-file");
      const spanEl = document.getElementById("trace-span");
      const hashEl = document.getElementById("trace-hash");
      if (tidEl) tidEl.textContent = `#${item.tid}`;
      if (fileEl) fileEl.textContent = item.file;
      if (spanEl) spanEl.textContent = item.span;
      if (hashEl) hashEl.textContent = item.hash;
    }
  }
}

export const DiagramViewer = new DiagramViewerModule();
