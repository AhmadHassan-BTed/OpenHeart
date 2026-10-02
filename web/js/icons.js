/**
 * OpenHeart Developer Studio Icon System
 * Standardized, high-precision SVG vector icons (Lucide / Codicons standard).
 * Zero emojis — 100% professional developer studio aesthetics.
 */

function svg(paths, viewBox = '0 0 16 16', size = 14) {
  return `<svg width="${size}" height="${size}" viewBox="${viewBox}" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" class="studio-icon" style="display:inline-block;vertical-align:middle;flex-shrink:0;">${paths}</svg>`;
}

export const Icons = {
  // Navigation & File Tree
  folder: svg('<path d="M1.5 3.5a1 1 0 0 1 1-1h3.2a1 1 0 0 1 .7.3l1.2 1.2h6.9a1 1 0 0 1 1 1v7.5a1 1 0 0 1-1 1h-12a1 1 0 0 1-1-1z"/>'),
  folderOpen: svg('<path d="M1.5 4a1 1 0 0 1 1-1h3.2l1.2 1.2h7.6a1 1 0 0 1 1 1v1.5h-11a1 1 0 0 0-1 1l-1 6z"/><path d="M3.5 13.5h11a1 1 0 0 0 1-.8l1.2-6.2a1 1 0 0 0-1-1.2H3.8a1 1 0 0 0-1 .8l-1.3 6.6z"/>'),
  fileCode: svg('<path d="M9.5 1.5H3.5a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1v-8.5z"/><polyline points="9.5 1.5 9.5 5 13.5 5"/><polyline points="6 8.5 4.5 10 6 11.5"/><polyline points="8.5 8.5 10 10 8.5 11.5"/>'),
  archive: svg('<rect x="1.5" y="2.5" width="13" height="3" rx="0.5"/><path d="M2.5 5.5v7.5a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1V5.5"/><line x1="6.5" y1="8" x2="9.5" y2="8"/>'),

  // Controls & Actions
  search: svg('<circle cx="7" cy="7" r="4.5"/><line x1="10.5" y1="10.5" x2="14" y2="14"/>'),
  radar: svg('<rect x="2" y="2" width="12" height="12" rx="2"/><circle cx="8" cy="8" r="3"/><line x1="8" y1="2" x2="8" y2="5"/><line x1="8" y1="11" x2="8" y2="14"/><line x1="2" y1="8" x2="5" y2="8"/><line x1="11" y1="8" x2="14" y2="8"/>'),
  loupe: svg('<circle cx="6.5" cy="6.5" r="4.5"/><line x1="10" y1="10" x2="14" y2="14"/><line x1="4.5" y1="6.5" x2="8.5" y2="6.5"/><line x1="6.5" y1="4.5" x2="6.5" y2="8.5"/>'),
  lock: svg('<rect x="3" y="7" width="10" height="7.5" rx="1.5"/><path d="M5.5 7V4.5a2.5 2.5 0 0 1 5 0V7"/>'),
  unlock: svg('<rect x="3" y="7" width="10" height="7.5" rx="1.5"/><path d="M5.5 7V4.5a2.5 2.5 0 0 1 5 0"/>'),
  layers: svg('<polygon points="8 1.5 14.5 5 8 8.5 1.5 5 8 1.5"/><polyline points="1.5 8 8 11.5 14.5 8"/><polyline points="1.5 11 8 14.5 14.5 11"/>'),

  // Export & IO
  save: svg('<path d="M12.5 14.5h-9a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1h7l3 3v9a1 1 0 0 1-1 1z"/><polyline points="11 14.5 11 9 5 9 5 14.5"/><polyline points="5 1.5 5 4.5 9.5 4.5"/>'),
  upload: svg('<path d="M13.5 10v3a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1v-3"/><polyline points="11 5.5 8 2.5 5 5.5"/><line x1="8" y1="2.5" x2="8" y2="10.5"/>'),
  download: svg('<path d="M13.5 10v3a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1v-3"/><polyline points="5 7.5 8 10.5 11 7.5"/><line x1="8" y1="10.5" x2="8" y2="2.5"/>'),
  image: svg('<rect x="2" y="2" width="12" height="12" rx="1.5"/><circle cx="5.5" cy="5.5" r="1.5"/><polyline points="14 10 10.5 6.5 4 13"/><polyline points="10 10 12 12"/>'),
  vector: svg('<rect x="2" y="2" width="4" height="4" rx="0.5"/><rect x="10" y="2" width="4" height="4" rx="0.5"/><rect x="10" y="10" width="4" height="4" rx="0.5"/><rect x="2" y="10" width="4" height="4" rx="0.5"/><line x1="6" y1="4" x2="10" y2="4"/><line x1="4" y1="6" x2="4" y2="10"/><line x1="12" y1="6" x2="12" y2="10"/><line x1="6" y1="12" x2="10" y2="12"/>'),
  copy: svg('<rect x="5.5" y="5.5" width="8" height="8" rx="1"/><path d="M2.5 10.5v-7a1 1 0 0 1 1-1h7"/>'),
  check: svg('<polyline points="2.5 8.5 6 12 13.5 4"/>'),

  // Themes & UI
  moon: svg('<path d="M13.5 8.8A6 6 0 1 1 7.2 2.5a4.7 4.7 0 0 0 6.3 6.3z"/>'),
  sun: svg('<circle cx="8" cy="8" r="3"/><line x1="8" y1="1.5" x2="8" y2="3"/><line x1="8" y1="13" x2="8" y2="14.5"/><line x1="1.5" y1="8" x2="3" y2="8"/><line x1="13" y1="8" x2="14.5" y2="8"/><line x1="3.4" y1="3.4" x2="4.5" y2="4.5"/><line x1="11.5" y1="11.5" x2="12.6" y2="12.6"/><line x1="3.4" y1="12.6" x2="4.5" y2="11.5"/><line x1="11.5" y1="4.5" x2="12.6" y2="3.4"/>'),
  chevronDown: svg('<polyline points="4 6 8 10 12 6"/>'),
  chevronRight: svg('<polyline points="6 4 10 8 6 12"/>'),
  zoomIn: svg('<line x1="8" y1="4" x2="8" y2="12"/><line x1="4" y1="8" x2="12" y2="8"/>'),
  zoomOut: svg('<line x1="4" y1="8" x2="12" y2="8"/>'),
  zoomReset: svg('<rect x="3" y="3" width="10" height="10" rx="1"/><circle cx="8" cy="8" r="1.5"/>'),
  close: svg('<line x1="3.5" y1="3.5" x2="12.5" y2="12.5"/><line x1="12.5" y1="3.5" x2="3.5" y2="12.5"/>'),
  slider: svg('<line x1="2" y1="4.5" x2="14" y2="4.5"/><circle cx="5" cy="4.5" r="1.5"/><line x1="2" y1="11.5" x2="14" y2="11.5"/><circle cx="11" cy="11.5" r="1.5"/>'),
  pulseDot: '<span class="status-indicator-dot"></span>',
  github: '<svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" class="studio-icon" style="display:inline-block;vertical-align:middle;flex-shrink:0;"><path fill-rule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>'
};
