/**
 * OpenHeart Guarded Web Storage Adapter
 * localStorage / sessionStorage throw when site data is blocked, in some private modes,
 * and on quota exhaustion. Every studio read and write goes through this adapter, which
 * never throws and falls back to an in-memory map so the studio always initialises.
 */

function probeStorage(kind) {
  try {
    const storage = window[kind];
    if (!storage) return null;
    const key = '__openheart_storage_probe__';
    storage.setItem(key, key);
    storage.removeItem(key);
    return storage;
  } catch (_) {
    return null;
  }
}

class GuardedStorage {
  constructor(kind) {
    this.kind = kind;
    this.backend = typeof window !== 'undefined' ? probeStorage(kind) : null;
    // Values the backend refused to persist (quota, blocked) live here for this page lifetime.
    this.memory = new Map();
  }

  get isPersistent() {
    return this.backend !== null;
  }

  get(key, fallback = null) {
    if (this.memory.has(key)) return this.memory.get(key);
    if (this.backend) {
      try {
        const value = this.backend.getItem(key);
        return value === null ? fallback : value;
      } catch (_) {
        return fallback;
      }
    }
    return fallback;
  }

  /**
   * Returns true when the value was persisted by the browser, false when it is only
   * held in memory for the current page.
   */
  set(key, value) {
    const str = String(value);
    if (this.backend) {
      try {
        this.backend.setItem(key, str);
        this.memory.delete(key);
        return true;
      } catch (_) {
        try { this.backend.removeItem(key); } catch (__) { /* storage unavailable */ }
      }
    }
    this.memory.set(key, str);
    return false;
  }

  remove(key) {
    this.memory.delete(key);
    if (this.backend) {
      try { this.backend.removeItem(key); } catch (_) { /* storage unavailable */ }
    }
  }

  getJson(key, fallback = null) {
    const raw = this.get(key, null);
    if (raw === null) return fallback;
    try {
      return JSON.parse(raw);
    } catch (_) {
      return fallback;
    }
  }

  setJson(key, value) {
    let serialized;
    try {
      serialized = JSON.stringify(value);
    } catch (_) {
      return false;
    }
    return this.set(key, serialized);
  }

  getNumber(key, fallback) {
    const raw = this.get(key, null);
    if (raw === null) return fallback;
    const n = parseFloat(raw);
    return Number.isFinite(n) ? n : fallback;
  }

  getBoolean(key, fallback) {
    const raw = this.get(key, null);
    if (raw === 'true') return true;
    if (raw === 'false') return false;
    return fallback;
  }
}

export const localStore = new GuardedStorage('localStorage');
export const sessionStore = new GuardedStorage('sessionStorage');
