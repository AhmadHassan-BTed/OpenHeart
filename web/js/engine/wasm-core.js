/**
 * OpenHeart engine core: instantiation and JSON marshalling for web/engine/openheart.wasm.
 *
 * The module is a WASI reactor with a plain C ABI over linear memory
 * (docs/engine-query-protocol.md, section 4):
 *   oh_alloc(len) -> ptr, oh_free(ptr, len), oh_analyze(ptr, len) -> session (0 = failure),
 *   oh_query(session, ptr, len) -> 0 | error status, oh_result_ptr() / oh_result_len()
 *   (the JSON result of the last call), oh_release(session), oh_version() -> 1.
 *
 * Shared by the engine Web Worker and the main-thread fallback; it touches no DOM.
 */

import { createWasiShim, WasiExit } from './wasi-shim.js';

export const PROTOCOL_VERSION = 1;

const REQUIRED_EXPORTS = ['memory', 'oh_alloc', 'oh_free', 'oh_analyze', 'oh_query', 'oh_result_ptr', 'oh_result_len'];

/** A failed engine call. `status` follows the protocol's http-like statuses. */
export class EngineCallError extends Error {
  constructor(status, message, body = null) {
    super(message);
    this.name = 'EngineCallError';
    this.status = status;
    this.body = body;
  }
}

/** The module trapped or exited; its state (and every session in it) is gone. */
export class EngineCrashError extends Error {
  constructor(message, cause = null) {
    super(message);
    this.name = 'EngineCrashError';
    this.cause = cause;
  }
}

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: false });

function describeFault(err) {
  if (err instanceof WasiExit) return `the engine module exited (code ${err.code})`;
  if (typeof WebAssembly !== 'undefined' && err instanceof WebAssembly.RuntimeError) return `the engine module trapped: ${err.message}`;
  return `the engine module failed: ${(err && err.message) || err}`;
}

/**
 * Copies `bytes` into a fresh oh_alloc buffer. Returns { ptr, size } for oh_free.
 */
export function writeBytes(exports, bytes) {
  const size = Math.max(1, bytes.length);
  const ptr = exports.oh_alloc(size) >>> 0;
  const memory = exports.memory;
  if (ptr === 0) throw new EngineCallError(507, `the engine could not allocate ${size} bytes for the request`);
  if (ptr + bytes.length > memory.buffer.byteLength) {
    throw new EngineCrashError(`oh_alloc returned a buffer outside linear memory (ptr ${ptr}, ${bytes.length} bytes)`);
  }
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, size };
}

/**
 * Reads the JSON result of the last oh_analyze / oh_query call. Returns the parsed value,
 * or null when the module produced no result.
 */
export function readResult(exports) {
  const ptr = exports.oh_result_ptr() >>> 0;
  const len = exports.oh_result_len() >>> 0;
  if (len === 0) return null;
  const memory = exports.memory;
  if (ptr === 0 || ptr + len > memory.buffer.byteLength) {
    throw new EngineCrashError(`the engine reported a result outside linear memory (ptr ${ptr}, ${len} bytes)`);
  }
  // slice() copies, so the text stays valid after the next call reuses the buffer.
  const text = decoder.decode(new Uint8Array(memory.buffer, ptr, len).slice());
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new EngineCallError(500, `the engine returned malformed JSON (${len} bytes): ${(err && err.message) || err}`);
  }
}

/** Normalises a protocol failure body to { status, message }. */
export function failureOf(body, fallbackStatus, fallbackMessage) {
  const err = body && typeof body === 'object' ? body.error : null;
  if (err && typeof err === 'object') {
    const status = Number.isInteger(err.status) ? err.status : fallbackStatus;
    const message = typeof err.message === 'string' && err.message ? err.message : fallbackMessage;
    return { status, message };
  }
  if (typeof err === 'string' && err) return { status: fallbackStatus, message: err };
  return { status: fallbackStatus, message: fallbackMessage };
}

/**
 * Wraps an instantiated engine module. Every call marshals JSON in and out and frees what it
 * allocated, also on failure. A trap or proc_exit marks the core as crashed.
 */
export class EngineCore {
  constructor(instance, shim = null) {
    const exports = instance && instance.exports;
    const missing = REQUIRED_EXPORTS.filter(name => !exports || exports[name] === undefined);
    if (missing.length) {
      throw new EngineCallError(500, `the engine module does not export ${missing.join(', ')}; it was not built for protocol version ${PROTOCOL_VERSION}`);
    }
    this.exports = exports;
    this.shim = shim;
    this.crashed = null;
    this.version = typeof exports.oh_version === 'function' ? exports.oh_version() >>> 0 : null;
    if (this.version !== null && this.version !== PROTOCOL_VERSION) {
      throw new EngineCallError(500, `the engine module speaks protocol version ${this.version}; this studio speaks version ${PROTOCOL_VERSION}`);
    }
  }

  /** Runs `fn(ptr, len)` with `value` serialised as JSON in module memory. */
  withJson(value, fn) {
    if (this.crashed) throw new EngineCrashError(`the engine module is unusable after an earlier failure (${this.crashed}); load it again`);
    const text = typeof value === 'string' ? value : JSON.stringify(value);
    const bytes = encoder.encode(text);
    let buffer = null;
    try {
      buffer = writeBytes(this.exports, bytes);
      return fn(buffer.ptr, bytes.length);
    } catch (err) {
      if (err instanceof EngineCallError) throw err;
      this.crashed = err instanceof EngineCrashError ? err.message : describeFault(err);
      if (this.shim) this.shim.flush();
      throw err instanceof EngineCrashError ? err : new EngineCrashError(this.crashed, err);
    } finally {
      if (buffer && !this.crashed) {
        try {
          this.exports.oh_free(buffer.ptr, buffer.size);
        } catch (err) {
          this.crashed = describeFault(err);
        }
      }
    }
  }

  /**
   * Runs the whole pipeline on an analyze request.
   * @returns {{ session: number, result: object }}
   */
  analyze(request) {
    return this.withJson(request, (ptr, len) => {
      const session = this.exports.oh_analyze(ptr, len) >>> 0;
      const body = readResult(this.exports);
      if (session === 0) {
        const f = failureOf(body, 500, 'the engine could not analyze the files');
        throw new EngineCallError(f.status, f.message, body);
      }
      if (!body || typeof body !== 'object') throw new EngineCallError(500, 'the engine returned no analysis result');
      return { session, result: body };
    });
  }

  /** Answers a protocol request (section 2) against a session. */
  query(session, request) {
    const handle = Number(session) >>> 0;
    if (!handle) throw new EngineCallError(400, 'no engine session');
    return this.withJson(request, (ptr, len) => {
      const status = this.exports.oh_query(handle, ptr, len) >>> 0;
      const body = readResult(this.exports);
      if (status !== 0) {
        const f = failureOf(body, status, `the engine answered with status ${status}`);
        throw new EngineCallError(f.status || status, f.message, body);
      }
      if (body && typeof body === 'object' && body.error && typeof body.error === 'object') {
        const f = failureOf(body, 500, 'the engine reported an error');
        throw new EngineCallError(f.status, f.message, body);
      }
      return body;
    });
  }

  release(session) {
    const handle = Number(session) >>> 0;
    if (!handle || this.crashed || typeof this.exports.oh_release !== 'function') return;
    try {
      this.exports.oh_release(handle);
    } catch (err) {
      this.crashed = describeFault(err);
    }
  }
}

/**
 * Compiles and instantiates the module with the WASI shim, binds memory and runs the
 * reactor's _initialize.
 * @param {ArrayBuffer|Uint8Array|WebAssembly.Module} source
 * @param {{ log?: Function, stdout?: Function, stderr?: Function }} [options]
 * @returns {Promise<EngineCore>}
 */
export async function instantiateEngine(source, options = {}) {
  const module = source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
  const shim = createWasiShim(options);
  const { imports, foreign } = shim.importsFor(module);
  if (foreign.length) {
    throw new EngineCallError(500, `the engine module imports ${foreign.slice(0, 6).join(', ')}${foreign.length > 6 ? ` and ${foreign.length - 6} more` : ''}, which the studio does not provide (only WASI preview1 is supported)`);
  }
  const instance = await WebAssembly.instantiate(module, imports);
  shim.bindInstance(instance);
  if (typeof instance.exports._initialize === 'function') {
    try {
      instance.exports._initialize();
    } catch (err) {
      throw new EngineCrashError(`the engine module failed to initialise: ${describeFault(err)}`, err);
    }
  }
  return new EngineCore(instance, shim);
}

/** Checks the WebAssembly magic number ("\0asm"). */
export function looksLikeWasm(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  return b.length >= 8 && b[0] === 0x00 && b[1] === 0x61 && b[2] === 0x73 && b[3] === 0x6d;
}
