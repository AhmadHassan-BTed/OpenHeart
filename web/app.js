/**
 * OpenHeart Web Studio — Application Entry Point
 * High-cohesion ES module orchestrator linking Logger, StudioState, OrbEngine, APIClient, DiagramViewer, and UIController.
 * Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).
 */

import { Logger } from './js/logger.js';
import { StudioState } from './js/state.js';
import { OrbEngine } from './js/orb.js';
import { APIClient } from './js/api.js';
import { DiagramViewer } from './js/viewer.js';
import { UIController } from './js/ui.js';

function initApp() {
  Logger.init('status-logs');
  Logger.step('INITIALIZING OPENHEART STUDIO');

  // Initialize 3D Spiky Orb WebGL Engine
  OrbEngine.init('orb-canvas');

  // Initialize Diagram Viewer & Interactive SVG Engine
  DiagramViewer.init('plantuml-render-container');

  // Initialize Studio UI Controller & DOM Bindings
  UIController.init();

  // Check Backend Server Health
  APIClient.checkHealth();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initApp);
} else {
  initApp();
}
