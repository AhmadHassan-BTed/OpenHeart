/**
 * OpenHeart program model: one query interface over both engine transports.
 *
 *   { kind: 'server' | 'wasm', label, query(request) }
 *
 * - 'server': POST /api/sessions/<id>/query on the OpenHeart analysis server (same origin).
 * - 'wasm':   oh_query on the in-browser engine module (wasm-engine.js).
 *
 * Requests and responses follow docs/engine-query-protocol.md, section 2. A failure always
 * rejects with QueryError { status, message }: 404 no such symbol, 400 malformed request,
 * 422 not derivable for this element, 410 the session ended, 0 the transport failed.
 */

const SERVER_QUERY_TIMEOUT_MS = 2 * 60 * 1000;
const CACHE_LIMIT = 200;

export const DIAGRAM_VIEWS = Object.freeze([
  'class', 'package', 'component', 'composite', 'object', 'deployment', 'profile', 'sequence',
  'statemachine', 'activity', 'usecase', 'communication', 'interaction', 'timing', 'cfg', 'dfg',
  'cdg', 'callgraph', 'robdd'
]);

export class QueryError extends Error {
  constructor(status, message, body = null) {
    super(message);
    this.name = 'QueryError';
    this.status = status;
    this.body = body;
  }
}

function clampDepth(depth, fallback, max) {
  const n = Number.parseInt(depth, 10);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(1, n));
}

function requireSymbol(symbol) {
  if (typeof symbol !== 'string' && typeof symbol !== 'number') throw new QueryError(400, 'no symbol was given');
  const s = String(symbol).trim();
  if (!s) throw new QueryError(400, 'no symbol was given');
  if (s.length > 2048) throw new QueryError(400, 'the symbol is too long');
  return s;
}

/** Turns any error body or transport error into a QueryError. */
export function toQueryError(err, fallbackStatus = 500) {
  if (err instanceof QueryError) return err;
  if (err && err.name === 'AbortError') return err;
  const status = err && Number.isInteger(err.status) ? err.status : fallbackStatus;
  return new QueryError(status, String((err && err.message) || err || 'the query failed'), err && err.body ? err.body : null);
}

export class ProgramModel {
  /**
   * @param {{ kind: 'server'|'wasm', label: string, transport: (request: object, control: { signal?: AbortSignal }) => Promise<any>, release?: () => void }} options
   */
  constructor({ kind, label, transport, release = null }) {
    this.kind = kind;
    this.label = label;
    this.transport = transport;
    this.releaseFn = release;
    this.cache = new Map();
    this.closed = false;
  }

  /**
   * Sends one protocol request. Successful answers are cached per request (they are pure
   * functions of the analysis), so drilling back and forth does not ask the engine again.
   */
  async query(request, { signal = null, cache = true } = {}) {
    if (this.closed) throw new QueryError(410, 'this analysis session was closed');
    if (!request || typeof request !== 'object' || typeof request.query !== 'string') {
      throw new QueryError(400, 'a request needs a "query" field');
    }
    const key = JSON.stringify(request);
    if (cache && this.cache.has(key)) {
      const hit = this.cache.get(key);
      this.cache.delete(key);
      this.cache.set(key, hit);
      return hit;
    }
    let response;
    try {
      response = await this.transport(request, { signal });
    } catch (err) {
      throw toQueryError(err);
    }
    if (response && typeof response === 'object' && response.error && typeof response.error === 'object') {
      const e = response.error;
      throw new QueryError(Number.isInteger(e.status) ? e.status : 500, typeof e.message === 'string' ? e.message : 'the engine reported an error', response);
    }
    if (cache) {
      this.cache.set(key, response);
      if (this.cache.size > CACHE_LIMIT) this.cache.delete(this.cache.keys().next().value);
    }
    return response;
  }

  resolve(symbol, control) {
    return this.query({ query: 'resolve', symbol: requireSymbol(symbol) }, control);
  }

  functions(filter = null, limit = 200, control) {
    const request = { query: 'functions', limit: clampDepth(limit, 200, 5000) };
    if (typeof filter === 'string' && filter.trim()) request.filter = filter.trim().slice(0, 200);
    return this.query(request, control);
  }

  callers(symbol, depth = 1, control) {
    return this.query({ query: 'callers', symbol: requireSymbol(symbol), depth: clampDepth(depth, 1, 6) }, control);
  }

  callees(symbol, depth = 1, control) {
    return this.query({ query: 'callees', symbol: requireSymbol(symbol), depth: clampDepth(depth, 1, 6) }, control);
  }

  dependencies(symbol, control) {
    return this.query({ query: 'dependencies', symbol: requireSymbol(symbol) }, control);
  }

  dependents(symbol, control) {
    return this.query({ query: 'dependents', symbol: requireSymbol(symbol) }, control);
  }

  impact(symbol, control) {
    return this.query({ query: 'impact', symbol: requireSymbol(symbol) }, control);
  }

  /**
   * A diagram as Graph IR: the whole program (no symbol) or focused on one element.
   */
  view(viewId, symbol = null, depth = 2, control) {
    if (!DIAGRAM_VIEWS.includes(viewId)) return Promise.reject(new QueryError(400, `unknown diagram view "${String(viewId).slice(0, 40)}"`));
    const request = { query: 'view', view: viewId };
    if (symbol !== null && symbol !== undefined && symbol !== '') request.symbol = requireSymbol(symbol);
    request.depth = clampDepth(depth, 2, 6);
    return this.query(request, control);
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    this.cache.clear();
    if (this.releaseFn) {
      try {
        this.releaseFn();
      } catch (_) { /* best effort */ }
    }
  }
}

async function readJsonBody(res) {
  try {
    return await res.json();
  } catch (_) {
    return null;
  }
}

/**
 * A program model over the analysis server's query route.
 * @param {string} sessionId
 * @param {{ fetchImpl?: Function, timeoutMs?: number, label?: string }} [options]
 */
export function createServerProgramModel(sessionId, options = {}) {
  const fetchImpl = options.fetchImpl || ((...args) => fetch(...args));
  const timeoutMs = options.timeoutMs || SERVER_QUERY_TIMEOUT_MS;
  const path = `/api/sessions/${encodeURIComponent(sessionId)}/query`;
  const transport = async (request, { signal = null } = {}) => {
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; controller.abort(); }, timeoutMs);
    const onAbort = () => controller.abort();
    if (signal) {
      if (signal.aborted) controller.abort();
      else signal.addEventListener('abort', onAbort, { once: true });
    }
    let res;
    try {
      res = await fetchImpl(path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
        body: JSON.stringify(request),
        credentials: 'same-origin',
        cache: 'no-store',
        signal: controller.signal
      });
    } catch (err) {
      if (signal && signal.aborted) throw err;
      if (timedOut) throw new QueryError(504, `the analysis server did not answer within ${Math.round(timeoutMs / 1000)} s`);
      throw new QueryError(0, `could not reach the analysis server: ${(err && err.message) || err}`);
    } finally {
      clearTimeout(timer);
      if (signal) signal.removeEventListener('abort', onAbort);
    }
    const body = await readJsonBody(res);
    if (body && typeof body === 'object' && body.error && typeof body.error === 'object') {
      const e = body.error;
      throw new QueryError(Number.isInteger(e.status) ? e.status : res.status, typeof e.message === 'string' ? e.message : `HTTP ${res.status}`, body);
    }
    if (!res.ok) {
      let message = body && typeof body.error === 'string' ? body.error : `the analysis server answered HTTP ${res.status}`;
      if (res.status === 401 || res.status === 403) message = `the analysis server refused the query (HTTP ${res.status}): ${message}`;
      if (res.status === 404 && body && typeof body.error === 'string' && /session/i.test(body.error)) {
        throw new QueryError(410, `the analysis server no longer has this session (${body.error}); analyze the repository again`, body);
      }
      throw new QueryError(res.status, message, body);
    }
    if (body === null) throw new QueryError(502, 'the analysis server answered without a JSON body');
    return body;
  };
  return new ProgramModel({ kind: 'server', label: options.label || 'OpenHeart analysis server', transport });
}

/**
 * A program model over an in-browser engine session.
 * @param {import('./wasm-engine.js').WasmEngine} engine
 * @param {number} session handle from engine.analyze
 * @param {number} generation engine generation the session belongs to
 */
export function createWasmProgramModel(engine, session, generation, options = {}) {
  const transport = (request, control = {}) => engine.query(session, generation, request, control);
  return new ProgramModel({
    kind: 'wasm',
    label: options.label || 'OpenHeart engine (WebAssembly, in this tab)',
    transport,
    release: () => { engine.release(session, generation); }
  });
}
