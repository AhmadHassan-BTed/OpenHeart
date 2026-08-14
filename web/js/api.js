/**
 * OpenHeart Web Studio — REST API Client
 * Encapsulates asynchronous HTTP requests to the Rust OpenHeartServer (§10.4 REST Interface).
 */

import { Logger } from './logger.js';

export class RESTAPIClient {
  constructor(baseUrl = '') {
    this.baseUrl = baseUrl;
  }

  async checkHealth() {
    try {
      Logger.log('[API] Checking backend server health (/api/health)...');
      const res = await fetch(`${this.baseUrl}/api/health`);
      if (res.ok) {
        const data = await res.json();
        Logger.log(`[API] Rust Backend Online: ${data.engine || 'OpenHeart SCPG'}`);
        return data;
      }
    } catch (e) {
      Logger.warn('[API] Local server offline or unreachable. Web studio running in standalone mode.');
    }
    return null;
  }

  async analyzeRepository(repoUrl, selectedTypes) {
    Logger.step('1. DISPATCHING REPOSITORY ANALYSIS');
    Logger.log(`Target Repo: ${repoUrl}`);
    Logger.log(`Selected Projections: ${Array.from(selectedTypes).join(', ')}`);

    try {
      const payload = {
        repo_url: repoUrl,
        diagram_types: Array.from(selectedTypes)
      };

      const startTime = performance.now();
      const res = await fetch(`${this.baseUrl}/api/analyze`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });

      if (res.ok) {
        const data = await res.json();
        const elapsed = Math.round(performance.now() - startTime);
        Logger.log(`[API] Received analysis payload from Rust engine in ${elapsed} ms.`);
        
        if (data.logs && Array.isArray(data.logs)) {
          data.logs.forEach(logLine => Logger.log(logLine));
        }

        return data;
      } else {
        throw new Error(`HTTP ${res.status} ${res.statusText}`);
      }
    } catch (err) {
      Logger.error(`[API ERROR] Analysis failed: ${err.message}. Using fallback client generator.`);
      return null;
    }
  }

  async renderKrokiSVG(pumlCode) {
    Logger.log('[API] Posting PlantUML payload to Kroki SVG rendering engine...');
    const res = await fetch('https://kroki.io/plantuml/svg', {
      method: 'POST',
      headers: { 'Content-Type': 'text/plain; charset=utf-8' },
      body: pumlCode
    });

    if (res.ok) {
      return await res.text();
    } else {
      throw new Error(`Kroki HTTP ${res.status}`);
    }
  }
}

export const APIClient = new RESTAPIClient();
