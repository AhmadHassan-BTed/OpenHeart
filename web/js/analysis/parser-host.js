/**
 * OpenHeart Parser Host
 * Serialises parse requests to a module Web Worker with a per-file time budget.
 * A file that exceeds the budget is abandoned by terminating the worker (a fresh one is
 * spawned for the next file). When module workers are unavailable, parsing runs inline;
 * the extractor is linear-time and inputs are size-capped, so inline parsing stays bounded.
 */

import { parseSource } from './source-parser.js';

export class ParserHost {
  constructor({ timeoutMs = 4000 } = {}) {
    this.timeoutMs = timeoutMs;
    this.worker = null;
    this.workerUnavailable = false;
    this.seq = 0;
    this.queue = Promise.resolve();
    this.inlineParses = 0;
    this.workerParses = 0;
  }

  spawn() {
    if (this.worker) return this.worker;
    if (this.workerUnavailable || typeof Worker === 'undefined') {
      this.workerUnavailable = true;
      return null;
    }
    try {
      this.worker = new Worker(new URL('./parser-worker.js', import.meta.url), { type: 'module', name: 'openheart-parser' });
    } catch (_) {
      this.worker = null;
      this.workerUnavailable = true;
    }
    return this.worker;
  }

  terminate() {
    if (this.worker) {
      this.worker.terminate();
      this.worker = null;
    }
  }

  /**
   * Parses one file. Resolves (never rejects) to one of:
   *   { ok: true, result }                         parsed
   *   { ok: false, error }                         the extractor reported an error
   *   { ok: false, timedOut: true, error }         abandoned after the time budget
   *   { ok: false, aborted: true, error }          cancelled through the AbortSignal
   */
  parse(path, code, signal = null) {
    const run = () => this.parseNow(path, code, signal);
    const pending = this.queue.then(run, run);
    this.queue = pending.then(() => undefined, () => undefined);
    return pending;
  }

  parseNow(path, code, signal) {
    if (signal && signal.aborted) {
      return Promise.resolve({ ok: false, aborted: true, error: 'Analysis cancelled.' });
    }
    const worker = this.spawn();
    if (!worker) return Promise.resolve(this.parseInline(path, code));

    const id = ++this.seq;
    return new Promise((resolve) => {
      let settled = false;
      let timer = null;
      const finish = (value) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        worker.removeEventListener('message', onMessage);
        worker.removeEventListener('error', onError);
        if (signal) signal.removeEventListener('abort', onAbort);
        resolve(value);
      };
      const onMessage = (event) => {
        const data = event.data;
        if (!data || data.id !== id) return;
        this.workerParses++;
        finish(data.ok ? { ok: true, result: data.result } : { ok: false, error: data.error });
      };
      const onError = (event) => {
        if (event && typeof event.preventDefault === 'function') event.preventDefault();
        // A module worker that cannot load (no module-worker support, CSP) reports here.
        this.terminate();
        this.workerUnavailable = true;
        finish(this.parseInline(path, code));
      };
      const onAbort = () => {
        this.terminate();
        finish({ ok: false, aborted: true, error: 'Analysis cancelled.' });
      };
      timer = setTimeout(() => {
        this.terminate();
        finish({ ok: false, timedOut: true, error: `Parsing exceeded the ${this.timeoutMs} ms budget and was abandoned.` });
      }, this.timeoutMs);
      worker.addEventListener('message', onMessage);
      worker.addEventListener('error', onError);
      if (signal) signal.addEventListener('abort', onAbort, { once: true });
      worker.postMessage({ id, path, code });
    });
  }

  parseInline(path, code) {
    this.inlineParses++;
    try {
      return { ok: true, result: parseSource(path, code), inline: true };
    } catch (err) {
      return { ok: false, error: String((err && err.message) || err) };
    }
  }

  get mode() {
    if (this.workerParses > 0 && this.inlineParses === 0) return 'worker';
    if (this.workerParses === 0 && this.inlineParses > 0) return 'main-thread';
    if (this.workerParses > 0) return 'mixed';
    return 'none';
  }
}
