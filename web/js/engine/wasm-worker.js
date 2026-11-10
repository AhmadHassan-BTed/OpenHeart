/**
 * OpenHeart engine Web Worker (module worker).
 * Owns one instance of web/engine/openheart.wasm so a long analysis never blocks the page.
 *
 * Messages in:  { id, op: 'init', bytes: ArrayBuffer }
 *               { id, op: 'analyze', request: { files, project_files, options } }
 *               { id, op: 'query', session, request }
 *               { id, op: 'release', session }
 * Messages out: { id, ok: true, result } | { id, ok: false, error: { name, status, message } }
 *               { op: 'log', level, line }   (engine stdout / stderr)
 */

import { instantiateEngine, EngineCallError, EngineCrashError } from './wasm-core.js';

let core = null;

const post = (message) => self.postMessage(message);

function errorPayload(err) {
  return {
    name: err && err.name ? err.name : 'Error',
    status: err instanceof EngineCallError ? err.status : 500,
    message: String((err && err.message) || err),
    crashed: err instanceof EngineCrashError
  };
}

async function handle(msg) {
  switch (msg.op) {
    case 'init': {
      core = await instantiateEngine(msg.bytes, {
        stdout: (line) => post({ op: 'log', level: 'info', line }),
        stderr: (line) => post({ op: 'log', level: 'warn', line }),
        log: (line) => post({ op: 'log', level: 'warn', line })
      });
      return { version: core.version };
    }
    case 'analyze':
      if (!core) throw new EngineCallError(503, 'the engine module is not loaded');
      return core.analyze(msg.request);
    case 'query':
      if (!core) throw new EngineCallError(503, 'the engine module is not loaded');
      return core.query(msg.session, msg.request);
    case 'release':
      if (core) core.release(msg.session);
      return null;
    default:
      throw new EngineCallError(400, `unknown engine worker operation "${String(msg.op).slice(0, 40)}"`);
  }
}

self.addEventListener('message', async (event) => {
  const msg = event.data;
  if (!msg || typeof msg !== 'object' || typeof msg.id !== 'number') return;
  try {
    const result = await handle(msg);
    post({ id: msg.id, ok: true, result });
  } catch (err) {
    post({ id: msg.id, ok: false, error: errorPayload(err) });
  }
});
