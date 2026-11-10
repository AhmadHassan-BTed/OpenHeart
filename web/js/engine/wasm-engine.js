/**
 * OpenHeart WebAssembly engine loader (main thread).
 *
 * Loads web/engine/openheart.wasm (fetched from 'engine/openheart.wasm' relative to the page)
 * and runs it in a module Web Worker, so a long analysis never freezes the page. When a worker
 * cannot be created (no module-worker support, a Trusted Types or CSP refusal) the module runs
 * on the main thread instead, and `mode` says so.
 *
 * The module is optional. When it is absent every method fails with EngineUnavailableError
 * carrying a human-readable reason, and the studio keeps its in-browser class-model path.
 */

import * as dom from '../dom.js';
import { instantiateEngine, looksLikeWasm, EngineCallError, EngineCrashError } from './wasm-core.js';

export { EngineCallError, EngineCrashError };

export const ENGINE_MODULE_PATH = 'engine/openheart.wasm';
const MAX_MODULE_BYTES = 256 * 1024 * 1024;
const WORKER_INIT_TIMEOUT_MS = 60 * 1000;

/** The engine module cannot be used; `reason` says why in plain words. */
export class EngineUnavailableError extends Error {
  constructor(reason) {
    super(reason);
    this.name = 'EngineUnavailableError';
    this.reason = reason;
  }
}

function abortError() {
  const err = new Error('Analysis cancelled.');
  err.name = 'AbortError';
  return err;
}

function pageUrl(path) {
  const base = typeof document !== 'undefined' && document.baseURI ? document.baseURI : (typeof location !== 'undefined' ? location.href : 'http://localhost/');
  return new URL(path, base).href;
}

/**
 * Creates the engine worker. Under `require-trusted-types-for 'script'` the Worker constructor
 * needs a TrustedScriptURL: when dom.js offers a constant-URL policy (`trustedScriptUrl`) it is
 * used; otherwise the plain URL is passed and a refusal falls back to the main thread.
 */
function defaultCreateWorker() {
  if (typeof Worker === 'undefined') throw new Error('Web Workers are not available in this browser');
  const href = new URL('./wasm-worker.js', import.meta.url).href;
  const scriptUrl = typeof dom.trustedScriptUrl === 'function' ? dom.trustedScriptUrl(href) : href;
  return new Worker(scriptUrl, { type: 'module', name: 'openheart-engine' });
}

function forwardLog(msg) {
  const line = String(msg.line || '');
  if (msg.level === 'info') console.log('[OpenHeart engine]', line);
  else console.warn('[OpenHeart engine]', line);
}

function rebuildError(error) {
  const e = error || {};
  if (e.crashed) return new EngineCrashError(e.message || 'the engine module failed');
  return new EngineCallError(Number.isInteger(e.status) ? e.status : 500, e.message || 'the engine call failed');
}

class WorkerBackend {
  constructor(worker) {
    this.worker = worker;
    this.mode = 'worker';
    this.seq = 0;
    this.pending = new Map();
    this.dead = null;
    worker.addEventListener('message', (event) => {
      const msg = event.data;
      if (!msg || typeof msg !== 'object') return;
      if (msg.op === 'log') {
        forwardLog(msg);
        return;
      }
      const entry = this.pending.get(msg.id);
      if (!entry) return;
      this.pending.delete(msg.id);
      if (msg.ok) entry.resolve(msg.result);
      else entry.reject(rebuildError(msg.error));
    });
    worker.addEventListener('error', (event) => {
      if (event && typeof event.preventDefault === 'function') event.preventDefault();
      this.fail(new EngineCrashError(`the engine worker failed${event && event.message ? `: ${event.message}` : ''}`));
    });
    worker.addEventListener('messageerror', () => {
      this.fail(new EngineCrashError('the engine worker sent a message that could not be read'));
    });
  }

  fail(err) {
    if (!this.dead) this.dead = err;
    this.pending.forEach(entry => entry.reject(err));
    this.pending.clear();
  }

  call(op, payload = {}, { signal = null, transfer = [] } = {}) {
    if (this.dead) return Promise.reject(this.dead);
    if (signal && signal.aborted) return Promise.reject(abortError());
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        this.pending.delete(id);
        // The only way to stop a running analysis is to stop the worker; its sessions go with it.
        this.terminate(new EngineCrashError('the engine worker was stopped because the analysis was cancelled'));
        reject(abortError());
      };
      this.pending.set(id, {
        resolve: (v) => { if (signal) signal.removeEventListener('abort', onAbort); resolve(v); },
        reject: (e) => { if (signal) signal.removeEventListener('abort', onAbort); reject(e); }
      });
      if (signal) signal.addEventListener('abort', onAbort, { once: true });
      try {
        this.worker.postMessage(Object.assign({ id, op }, payload), transfer);
      } catch (err) {
        this.pending.delete(id);
        if (signal) signal.removeEventListener('abort', onAbort);
        reject(new EngineCallError(500, `could not send the request to the engine worker: ${(err && err.message) || err}`));
      }
    });
  }

  terminate(reason = null) {
    try {
      this.worker.terminate();
    } catch (_) { /* already gone */ }
    this.fail(reason || new EngineCrashError('the engine worker was stopped'));
  }
}

class InlineBackend {
  constructor(core, reason) {
    this.core = core;
    this.mode = 'main-thread';
    this.reason = reason;
    this.dead = null;
  }

  async call(op, payload = {}, { signal = null } = {}) {
    if (signal && signal.aborted) throw abortError();
    // Yield once so the progress UI can paint before a long synchronous call.
    await new Promise(resolve => setTimeout(resolve, 0));
    if (signal && signal.aborted) throw abortError();
    switch (op) {
      case 'analyze': return this.core.analyze(payload.request);
      case 'query': return this.core.query(payload.session, payload.request);
      case 'release': this.core.release(payload.session); return null;
      default: throw new EngineCallError(400, `unknown engine operation "${op}"`);
    }
  }

  terminate() {
    this.dead = new EngineCrashError('the engine was unloaded');
  }
}

export class WasmEngine {
  /**
   * @param {{ path?: string, preferWorker?: boolean, fetchImpl?: Function, createWorker?: Function }} [options]
   */
  constructor(options = {}) {
    this.path = options.path || ENGINE_MODULE_PATH;
    this.preferWorker = options.preferWorker !== false;
    this.fetchImpl = options.fetchImpl || ((...args) => fetch(...args));
    this.createWorker = options.createWorker || defaultCreateWorker;
    this.state = 'unknown';
    this.reason = null;
    this.bytes = null;
    this.backend = null;
    this.loading = null;
    this.probing = null;
    this.generation = 0;
    this.version = null;
    this.workerFallbackReason = null;
  }

  get url() {
    return pageUrl(this.path);
  }

  get mode() {
    return this.backend ? this.backend.mode : null;
  }

  /** Plain status for the UI: { state, reason, mode, modeReason }. */
  status() {
    return { state: this.state, reason: this.reason, mode: this.mode, modeReason: this.workerFallbackReason };
  }

  markAbsent(reason) {
    this.state = 'absent';
    this.reason = reason;
    return { present: false, reason };
  }

  /**
   * Cheap presence check (HEAD request; nothing is downloaded or compiled).
   * @returns {Promise<{ present: boolean|null, reason: string|null }>} null when undecidable
   */
  probe() {
    if (this.state === 'ready') return Promise.resolve({ present: true, reason: null });
    if (this.state === 'absent' || this.state === 'failed') return Promise.resolve({ present: false, reason: this.reason });
    if (this.probing) return this.probing;
    this.probing = (async () => {
      if (typeof WebAssembly === 'undefined') return this.markAbsent('this browser does not support WebAssembly');
      let res;
      try {
        res = await this.fetchImpl(this.url, { method: 'HEAD', cache: 'no-cache', credentials: 'same-origin' });
      } catch (err) {
        return { present: null, reason: `could not check for ${this.path}: ${(err && err.message) || err}` };
      }
      if (res.status === 404 || res.status === 410) {
        return this.markAbsent(`${this.path} is not present on this site (HTTP ${res.status}); build it with scripts/build-wasm.sh`);
      }
      if (!res.ok) return { present: null, reason: `${this.path} answered HTTP ${res.status} to a presence check` };
      const type = (res.headers && res.headers.get('content-type')) || '';
      if (/text\/html/i.test(type)) {
        return this.markAbsent(`${this.path} is not present on this site (the server answered with an HTML page)`);
      }
      return { present: true, reason: null };
    })().finally(() => { this.probing = null; });
    return this.probing;
  }

  /**
   * Downloads, validates and starts the module (worker first, main thread as fallback).
   * Rejects with EngineUnavailableError when the module is absent or unusable.
   */
  load() {
    if (this.backend && !this.backend.dead) return Promise.resolve(this.status());
    if (this.state === 'absent' || this.state === 'failed') return Promise.reject(new EngineUnavailableError(this.reason));
    if (this.loading) return this.loading;
    this.loading = this.loadNow().finally(() => { this.loading = null; });
    return this.loading;
  }

  async loadNow() {
    if (typeof WebAssembly === 'undefined') throw new EngineUnavailableError(this.markAbsent('this browser does not support WebAssembly').reason);
    if (!this.bytes) {
      this.state = 'loading';
      let res;
      try {
        res = await this.fetchImpl(this.url, { cache: 'no-cache', credentials: 'same-origin' });
      } catch (err) {
        this.state = 'unknown';
        throw new EngineUnavailableError(`could not download ${this.path}: ${(err && err.message) || err}`);
      }
      if (res.status === 404 || res.status === 410) {
        throw new EngineUnavailableError(this.markAbsent(`${this.path} is not present on this site (HTTP ${res.status}); build it with scripts/build-wasm.sh`).reason);
      }
      if (!res.ok) {
        this.state = 'unknown';
        throw new EngineUnavailableError(`${this.path} could not be downloaded (HTTP ${res.status})`);
      }
      const declared = parseInt((res.headers && res.headers.get('content-length')) || '', 10);
      if (Number.isFinite(declared) && declared > MAX_MODULE_BYTES) {
        throw new EngineUnavailableError(this.fail(`${this.path} is ${Math.round(declared / 1048576)} MB, above the ${MAX_MODULE_BYTES / 1048576} MB limit`));
      }
      const bytes = await res.arrayBuffer();
      if (bytes.byteLength > MAX_MODULE_BYTES) {
        throw new EngineUnavailableError(this.fail(`${this.path} is ${Math.round(bytes.byteLength / 1048576)} MB, above the ${MAX_MODULE_BYTES / 1048576} MB limit`));
      }
      if (!looksLikeWasm(bytes)) {
        throw new EngineUnavailableError(this.markAbsent(`${this.path} is not a WebAssembly module (the server returned ${bytes.byteLength} bytes of something else)`).reason);
      }
      this.bytes = bytes;
    }
    try {
      this.backend = await this.startBackend();
    } catch (err) {
      throw new EngineUnavailableError(this.fail(`the engine module could not be started: ${(err && err.message) || err}`));
    }
    this.generation++;
    this.state = 'ready';
    this.reason = null;
    return this.status();
  }

  fail(reason) {
    this.state = 'failed';
    this.reason = reason;
    return reason;
  }

  async startBackend() {
    let workerError = null;
    if (this.preferWorker) {
      let worker = null;
      try {
        worker = this.createWorker();
      } catch (err) {
        workerError = `a Web Worker could not be created (${(err && err.message) || err})`;
      }
      if (worker) {
        const backend = new WorkerBackend(worker);
        let timer = null;
        try {
          const timeout = new Promise((_, reject) => {
            timer = setTimeout(() => reject(new Error(`the worker did not start within ${WORKER_INIT_TIMEOUT_MS / 1000} s`)), WORKER_INIT_TIMEOUT_MS);
          });
          // The bytes are copied (not transferred) so the module can be restarted after a cancel.
          const info = await Promise.race([backend.call('init', { bytes: this.bytes }), timeout]);
          this.version = info && Number.isInteger(info.version) ? info.version : null;
          this.workerFallbackReason = null;
          return backend;
        } catch (err) {
          backend.terminate();
          // A module that loads but fails to instantiate fails the same way on the main thread.
          if (err instanceof EngineCallError) throw err;
          workerError = `the engine worker failed to start (${(err && err.message) || err})`;
        } finally {
          clearTimeout(timer);
        }
      }
    } else {
      workerError = 'Web Workers were disabled for the engine';
    }
    const core = await instantiateEngine(this.bytes);
    this.version = core.version;
    this.workerFallbackReason = workerError;
    if (workerError) console.warn(`[OpenHeart engine] Running on the main thread: ${workerError}.`);
    return new InlineBackend(core, workerError);
  }

  /**
   * Runs the whole pipeline in memory.
   * @param {Array<{path:string,text:string}>} files source files
   * @param {Array<{path:string,text:string}>} projectFiles build and deployment files
   * @param {object} [options] analyze options ({ diagrams: [...] })
   * @param {{ signal?: AbortSignal }} [control]
   * @returns {Promise<{ session: number, generation: number, result: object }>}
   */
  async analyze(files, projectFiles = [], options = {}, control = {}) {
    await this.load();
    const request = { files, project_files: projectFiles, options };
    const generation = this.generation;
    try {
      const out = await this.backend.call('analyze', { request }, { signal: control.signal || null });
      return { session: out.session, generation, result: out.result };
    } catch (err) {
      this.afterFailure(err);
      throw err;
    }
  }

  /** Answers a protocol request against a session from `analyze`. */
  async query(session, generation, request, control = {}) {
    if (!this.backend || this.backend.dead || generation !== this.generation) {
      throw new EngineCallError(410, 'the in-browser engine session ended (the engine module was restarted); analyze the codebase again');
    }
    try {
      return await this.backend.call('query', { session, request }, { signal: control.signal || null });
    } catch (err) {
      this.afterFailure(err);
      throw err;
    }
  }

  async release(session, generation) {
    if (!this.backend || this.backend.dead || generation !== this.generation) return;
    try {
      await this.backend.call('release', { session });
    } catch (_) { /* nothing to release */ }
  }

  /** A crash or a cancelled worker leaves no usable instance: the next call restarts it. */
  afterFailure(err) {
    if (err instanceof EngineCrashError || (err && err.name === 'AbortError') || (this.backend && this.backend.dead)) {
      if (this.backend) this.backend.terminate();
      this.backend = null;
      this.generation++;
    }
  }

  terminate() {
    if (this.backend) this.backend.terminate();
    this.backend = null;
    this.generation++;
  }
}

let shared = null;

/** The studio's single engine instance. */
export function getWasmEngine() {
  if (!shared) shared = new WasmEngine();
  return shared;
}
