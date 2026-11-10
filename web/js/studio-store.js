/**
 * OpenHeart Studio Session Store
 * The single source of truth for what the studio is showing:
 *   { source: { kind, name, url, ... }, ir, engine, stats, provenance, analysis }
 * kinds: 'bundled' (sample files in web/diagrams), 'engine' (Rust analysis server response),
 *        'github' | 'folder' | 'zip' (files read in the browser: the in-browser declaration
 *        extractor's class model in `ir`, plus, when web/engine/openheart.wasm is present, the
 *        engine module's result in `analysis`).
 * `analysis` ({ transport: 'wasm', diagrams, summary, warnings, provenance }) and the program
 * model that answers inspector queries live in memory only: an engine instance cannot survive
 * a reload, so a restored session falls back to the class model and says so.
 * Every import replaces the whole session atomically, so names, IR, sources, provenance and
 * the program model can never belong to different codebases. Persistence goes through the
 * guarded adapter.
 */

import { sessionStore } from './storage.js';
import { validateGraphIr } from './graph-ir-validate.js';
import { createServerProgramModel } from './engine/program-model.js';

const SESSION_KEY = 'openheart_session_v2';
const FILES_KEY = 'openheart_session_files_v2';
const LEGACY_KEYS = ['openheart_custom_graph_ir', 'openheart_custom_name', 'openheart_custom_files'];
const MAX_PERSISTED_SOURCE_CHARS = 4 * 1024 * 1024;
const PERSISTABLE_KINDS = new Set(['engine', 'github', 'folder', 'zip']);
const LOCAL_KINDS = new Set(['folder', 'zip']);

export function bundledSession() {
  return {
    source: { kind: 'bundled', name: 'Bundled sample diagrams', url: null },
    ir: null,
    engine: null,
    stats: null,
    analysis: null,
    provenance: {
      engine: 'Bundled sample files (web/diagrams)',
      approximate: false,
      content_hash: null
    }
  };
}

/** The persisted form of a session: everything except the in-memory engine result. */
function persistableSession(session) {
  const copy = Object.assign({}, session);
  delete copy.analysis;
  if (session.analysis) {
    copy.analysis_note = {
      transport: session.analysis.transport,
      functions: session.analysis.summary && Number.isFinite(session.analysis.summary.functions) ? session.analysis.summary.functions : null
    };
  }
  return copy;
}

function cleanString(value, max) {
  return typeof value === 'string' ? value.slice(0, max) : null;
}

class StudioStore {
  constructor() {
    this.session = bundledSession();
    this.files = new Map();
    this.listeners = new Set();
    this.lastPersistOk = true;
    this.programModel = null;
    this.engineModule = { state: 'unknown', reason: null, mode: null };
  }

  get() {
    return this.session;
  }

  /**
   * The program model that answers inspector queries for the active session, or null:
   * the in-browser engine session of a GitHub/folder/ZIP import, or the analysis server's
   * session for an engine import (created on first use, also after a reload).
   */
  getProgramModel() {
    if (this.programModel && !this.programModel.closed) return this.programModel;
    const s = this.session;
    if (s.source.kind === 'engine' && s.engine && s.engine.session_id) {
      this.programModel = createServerProgramModel(s.engine.session_id, { label: `OpenHeart analysis server (session ${s.engine.session_id})` });
      return this.programModel;
    }
    return null;
  }

  /** Why the active session has no program model (for the inspector), or null. */
  programModelGap() {
    if (this.getProgramModel()) return null;
    const s = this.session;
    if (s.source.kind === 'bundled') return 'The bundled sample diagrams are static files; there is no analysis to query. Analyze a repository, folder or ZIP first.';
    if (s.source.kind === 'engine') return 'The analysis server did not report a session id for this analysis, so it cannot be queried.';
    if (s.analysis_note) return 'The in-browser engine session ended when the page was reloaded; analyze the codebase again to query it.';
    const em = this.engineModule;
    if (em.state === 'absent' || em.state === 'failed') return `Relations need the OpenHeart engine, and the engine module is not available: ${em.reason}. The in-browser class model has no call or data-flow facts.`;
    return 'Relations need the OpenHeart engine (the WebAssembly module or the analysis server); this session was analyzed by the in-browser declaration extractor only.';
  }

  /** True when the active session carries the engine module's diagrams. */
  hasEngineDiagrams() {
    return Boolean(this.session.analysis && this.session.analysis.diagrams);
  }

  /** Records the engine module's availability ({ state, reason, mode }) for the UI. */
  setEngineModuleStatus(status) {
    this.engineModule = Object.assign({ state: 'unknown', reason: null, mode: null }, status || {});
  }

  getFiles() {
    return this.files;
  }

  get codebaseName() {
    return this.session.source.name;
  }

  get sourceKind() {
    return this.session.source.kind;
  }

  isInBrowserModel() {
    return Boolean(this.session.ir) && (this.session.source.kind === 'github' || LOCAL_KINDS.has(this.session.source.kind));
  }

  subscribe(callback) {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  }

  emit() {
    this.listeners.forEach(cb => {
      try {
        cb(this.session);
      } catch (err) {
        console.error('[OpenHeart Store] listener failed:', err);
      }
    });
  }

  /**
   * Replaces the active session atomically.
   * @param {object} session
   * @param {Map<string,string>} files source text by path (local imports and fetched GitHub files)
   * @param {{ programModel?: object|null }} [options] the program model of an in-browser engine session
   */
  replace(session, files = new Map(), { programModel = null } = {}) {
    this.closeProgramModel();
    if (session.analysis === undefined) session.analysis = null;
    this.session = session;
    this.files = files instanceof Map ? files : new Map();
    this.programModel = programModel;
    this.persist();
    this.emit();
  }

  reset() {
    this.closeProgramModel();
    this.session = bundledSession();
    this.files = new Map();
    this.clearPersisted();
    this.emit();
  }

  closeProgramModel() {
    if (this.programModel) this.programModel.close();
    this.programModel = null;
  }

  persist() {
    LEGACY_KEYS.forEach(k => sessionStore.remove(k));
    if (!PERSISTABLE_KINDS.has(this.session.source.kind)) {
      this.clearPersisted();
      this.lastPersistOk = true;
      return;
    }
    const ok = sessionStore.setJson(SESSION_KEY, { v: 2, session: persistableSession(this.session) });
    let filesOk = true;
    if (LOCAL_KINDS.has(this.session.source.kind) && this.files.size > 0) {
      // Local sources cannot be re-fetched, so they are kept for this tab only (sessionStorage),
      // within a size budget. GitHub sources are re-fetched from raw URLs instead.
      let total = 0;
      const entries = [];
      for (const [path, content] of this.files) {
        total += content.length;
        if (total > MAX_PERSISTED_SOURCE_CHARS) break;
        entries.push([path, content]);
      }
      filesOk = total <= MAX_PERSISTED_SOURCE_CHARS && sessionStore.setJson(FILES_KEY, entries);
      if (!filesOk) sessionStore.remove(FILES_KEY);
    } else {
      sessionStore.remove(FILES_KEY);
    }
    this.lastPersistOk = ok && filesOk;
  }

  clearPersisted() {
    sessionStore.remove(SESSION_KEY);
    sessionStore.remove(FILES_KEY);
    LEGACY_KEYS.forEach(k => sessionStore.remove(k));
  }

  /**
   * Restores the session saved for this tab. Legacy keys from older studio versions are
   * deleted rather than migrated, because they were stored without validation.
   * @returns {{ restored: boolean, warnings: string[] }}
   */
  restore() {
    LEGACY_KEYS.forEach(k => sessionStore.remove(k));
    const saved = sessionStore.getJson(SESSION_KEY, null);
    if (!saved || saved.v !== 2 || !saved.session || typeof saved.session !== 'object' || !saved.session.source) {
      return { restored: false, warnings: [] };
    }
    const s = saved.session;
    const kind = s.source.kind;
    if (!PERSISTABLE_KINDS.has(kind)) {
      this.clearPersisted();
      return { restored: false, warnings: [] };
    }
    const warnings = [];
    const session = {
      source: {
        kind,
        name: cleanString(s.source.name, 200) || 'Restored codebase',
        url: cleanString(s.source.url, 2048),
        owner: cleanString(s.source.owner, 100),
        repo: cleanString(s.source.repo, 200),
        ref: cleanString(s.source.ref, 300),
        ref_kind: cleanString(s.source.ref_kind, 40),
        commit: cleanString(s.source.commit, 64),
        sub_path: cleanString(s.source.sub_path, 1024),
        private_access: s.source.private_access === true
      },
      ir: null,
      engine: null,
      analysis: null,
      stats: s.stats && typeof s.stats === 'object' ? s.stats : null,
      provenance: s.provenance && typeof s.provenance === 'object' ? s.provenance : null
    };
    if (s.analysis_note && typeof s.analysis_note === 'object' && s.analysis_note.transport === 'wasm') {
      session.analysis_note = { transport: 'wasm', functions: Number.isFinite(s.analysis_note.functions) ? s.analysis_note.functions : null };
      warnings.push('This codebase was analyzed by the in-browser engine module, whose results live in memory only. After the reload the studio shows the in-browser class model; analyze the codebase again for all views and relations.');
    }
    if (s.ir) {
      const v = validateGraphIr(s.ir, {});
      if (!v.ok) {
        this.clearPersisted();
        return { restored: false, warnings: v.errors };
      }
      session.ir = v.ir;
      warnings.push(...v.warnings);
    }
    if (kind === 'engine') {
      const e = s.engine && typeof s.engine === 'object' ? s.engine : {};
      const puml = {};
      if (e.puml_by_type && typeof e.puml_by_type === 'object') {
        Object.entries(e.puml_by_type).forEach(([k, v]) => {
          if (/^[a-z_]{1,32}$/.test(k) && typeof v === 'string') puml[k] = v;
        });
      }
      session.engine = {
        session_id: cleanString(e.session_id, 200),
        artifact_base_url: typeof e.artifact_base_url === 'string' && e.artifact_base_url.startsWith('/') && !e.artifact_base_url.startsWith('//') ? e.artifact_base_url.slice(0, 300) : null,
        puml_by_type: puml,
        errors: Array.isArray(e.errors) ? e.errors.filter(x => typeof x === 'string').slice(0, 50) : [],
        logs: Array.isArray(e.logs) ? e.logs.filter(x => typeof x === 'string').slice(0, 200) : []
      };
    } else if (!session.ir) {
      this.clearPersisted();
      return { restored: false, warnings };
    }
    const files = new Map();
    if (LOCAL_KINDS.has(kind)) {
      const entries = sessionStore.getJson(FILES_KEY, []);
      if (Array.isArray(entries)) {
        entries.forEach(entry => {
          if (Array.isArray(entry) && typeof entry[0] === 'string' && typeof entry[1] === 'string') files.set(entry[0], entry[1]);
        });
      }
    }
    this.session = session;
    this.files = files;
    this.emit();
    return { restored: true, warnings };
  }
}

export const studioStore = new StudioStore();
