/**
 * OpenHeart Declaration Extractor Worker
 * Runs parseSource off the main thread so a pathological file can be abandoned
 * (the host terminates this worker on timeout) without freezing the studio.
 */

import { parseSource } from './source-parser.js';

self.addEventListener('message', (event) => {
  const { id, path, code } = event.data || {};
  try {
    const result = parseSource(path, code);
    self.postMessage({ id, ok: true, result });
  } catch (err) {
    self.postMessage({ id, ok: false, error: String((err && err.message) || err) });
  }
});
