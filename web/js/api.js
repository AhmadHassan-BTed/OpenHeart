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
    try {
      const res = await fetch('https://kroki.io/plantuml/svg', {
        method: 'POST',
        headers: { 'Content-Type': 'text/plain; charset=utf-8' },
        body: pumlCode
      });

      if (res.ok) {
        return await res.text();
      }
    } catch (err) {
      Logger.warn(`[KROKI OFFLINE] ${err.message}. Switching to local OpenHeart SVG Vector Engine.`);
    }

    return this.generateLocalVectorSVG(pumlCode);
  }

  generateLocalVectorSVG(pumlCode) {
    const lines = pumlCode.split('\n');
    const classes = [];
    const objects = [];
    const packages = [];

    lines.forEach(line => {
      const trimmed = line.trim();
      if (trimmed.startsWith('class ')) {
        const match = trimmed.match(/class\s+([a-zA-Z0-9_]+)/);
        if (match) classes.push(match[1]);
      } else if (trimmed.startsWith('object ')) {
        const match = trimmed.match(/object\s+"([^"]+)"/);
        if (match) objects.push(match[1]);
      } else if (trimmed.startsWith('package ')) {
        const match = trimmed.match(/package\s+"([^"]+)"/);
        if (match) packages.push(match[1]);
      }
    });

    const totalNodes = classes.length + objects.length + packages.length;
    const width = Math.max(900, Math.ceil(Math.sqrt(totalNodes || 1)) * 220 + 100);
    const height = Math.max(600, Math.ceil((totalNodes || 1) / 4) * 140 + 140);

    let svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" preserveAspectRatio="xMidYMid meet" style="background:#0d0d11; font-family: monospace; max-width: 100%; height: auto; display: block; margin: auto;">`;
    svg += `<defs><linearGradient id="cardGrad" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="#181820"/><stop offset="100%" stop-color="#111116"/></linearGradient></defs>`;
    
    let x = 40, y = 40;
    const cols = 4;
    let col = 0;

    classes.forEach(cls => {
      svg += `
        <g transform="translate(${x},${y})">
          <rect width="190" height="90" rx="6" fill="url(#cardGrad)" stroke="#00ff66" stroke-width="1.5" />
          <rect width="190" height="26" rx="6" fill="#00ff66" opacity="0.15" />
          <text x="12" y="18" fill="#00ff66" font-size="12" font-weight="bold">class ${cls}</text>
          <line x1="0" y1="26" x2="190" y2="26" stroke="#333" stroke-width="1" />
          <text x="12" y="46" fill="#aaa" font-size="10">+ execute()</text>
          <text x="12" y="64" fill="#aaa" font-size="10">+ status: String</text>
        </g>`;
      col++;
      x += 210;
      if (col >= cols) {
        col = 0;
        x = 40;
        y += 120;
      }
    });

    objects.forEach(obj => {
      svg += `
        <g transform="translate(${x},${y})">
          <rect width="190" height="85" rx="6" fill="url(#cardGrad)" stroke="#00e5ff" stroke-width="1.5" />
          <rect width="190" height="26" rx="6" fill="#00e5ff" opacity="0.15" />
          <text x="12" y="18" fill="#00e5ff" font-size="11" font-weight="bold">object ${obj.split(' ')[0]}</text>
          <line x1="0" y1="26" x2="190" y2="26" stroke="#333" stroke-width="1" />
          <text x="12" y="46" fill="#888" font-size="10">state = "active"</text>
        </g>`;
      col++;
      x += 210;
      if (col >= cols) {
        col = 0;
        x = 40;
        y += 120;
      }
    });

    packages.forEach(pkg => {
      svg += `
        <g transform="translate(${x},${y})">
          <rect width="190" height="75" rx="6" fill="url(#cardGrad)" stroke="#ffaa00" stroke-width="1.5" />
          <text x="12" y="20" fill="#ffaa00" font-size="11" font-weight="bold">package ${pkg}</text>
          <line x1="0" y1="28" x2="190" y2="28" stroke="#333" stroke-width="1" />
        </g>`;
      col++;
      x += 210;
      if (col >= cols) {
        col = 0;
        x = 40;
        y += 120;
      }
    });

    if (totalNodes === 0) {
      svg += `<text x="40" y="80" fill="#00ff66" font-size="14">> LOCAL VECTOR SVG ENGINE :: READY</text>`;
    }

    svg += `</svg>`;
    return svg;
  }
}

export const APIClient = new RESTAPIClient();
