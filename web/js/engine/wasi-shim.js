/**
 * OpenHeart in-memory WASI preview1 shim
 *
 * Implements exactly the WASI imports the engine module is allowed to use
 * (docs/engine-query-protocol.md, section 4), entirely in memory:
 *   fd_write (stdout and stderr only, to the console), environ_get / environ_sizes_get and
 *   args_get / args_sizes_get (empty), clock_time_get, random_get (crypto.getRandomValues),
 *   proc_exit (throws WasiExit), fd_close / fd_seek / fd_fdstat_get (stdio only, minimal),
 *   fd_prestat_get (no preopened directories: always EBADF).
 * Any other wasi_snapshot_preview1 import the module declares is satisfied by a stub that
 * returns ENOSYS (52) and logs once per name. There is no file system, network or process.
 *
 * Runs unchanged on the main thread, in a Web Worker and in Node (tests).
 */

export const WASI_MODULE = 'wasi_snapshot_preview1';

export const ERRNO = Object.freeze({
  SUCCESS: 0,
  BADF: 8,
  FAULT: 21,
  INVAL: 28,
  NOSYS: 52,
  SPIPE: 70
});

const CLOCK_REALTIME = 0;
const CLOCK_MONOTONIC = 1;
const CLOCK_PROCESS_CPUTIME = 2;
const CLOCK_THREAD_CPUTIME = 3;

const FILETYPE_CHARACTER_DEVICE = 2;
const FDFLAGS_APPEND = 1;
const RIGHT_FD_READ = 1n << 1n;
const RIGHT_FD_WRITE = 1n << 6n;

/** Largest buffer crypto.getRandomValues accepts in one call. */
const RANDOM_CHUNK = 65536;

/** Thrown by proc_exit: the module asked to terminate with `code`. */
export class WasiExit extends Error {
  constructor(code) {
    super(`The WebAssembly module exited with code ${code}.`);
    this.name = 'WasiExit';
    this.code = code;
  }
}

function nowNs() {
  const perf = typeof performance !== 'undefined' ? performance : null;
  if (perf && typeof perf.now === 'function') {
    const origin = typeof perf.timeOrigin === 'number' ? perf.timeOrigin : Date.now() - perf.now();
    return BigInt(Math.round((origin + perf.now()) * 1e6));
  }
  return BigInt(Date.now()) * 1000000n;
}

function monotonicNs() {
  const perf = typeof performance !== 'undefined' ? performance : null;
  if (perf && typeof perf.now === 'function') return BigInt(Math.round(perf.now() * 1e6));
  return BigInt(Date.now()) * 1000000n;
}

/**
 * Line-buffered console sink for one stream. Bytes are decoded as UTF-8 incrementally, so a
 * multi-byte character split across two fd_write calls is never mangled.
 */
function lineSink(emit) {
  const decoder = new TextDecoder('utf-8');
  let pending = '';
  return {
    write(bytes) {
      pending += decoder.decode(bytes, { stream: true });
      let nl = pending.indexOf('\n');
      while (nl >= 0) {
        emit(pending.slice(0, nl));
        pending = pending.slice(nl + 1);
        nl = pending.indexOf('\n');
      }
      // A runaway line without newlines is flushed rather than buffered without bound.
      if (pending.length > 16384) {
        emit(pending);
        pending = '';
      }
    },
    flush() {
      pending += decoder.decode();
      if (pending) emit(pending);
      pending = '';
    }
  };
}

/**
 * Creates a shim instance. Bind the module's exported memory with `bindMemory` (or
 * `bindInstance`) right after instantiation and before the first call into the module.
 *
 * @param {{ stdout?: (line:string)=>void, stderr?: (line:string)=>void,
 *           log?: (message:string)=>void }} [options]
 */
export function createWasiShim(options = {}) {
  const prefix = '[OpenHeart engine]';
  const stdoutEmit = options.stdout || ((line) => console.log(prefix, line));
  const stderrEmit = options.stderr || ((line) => console.warn(prefix, line));
  const log = options.log || ((message) => console.warn(message));
  const sinks = { 1: lineSink(stdoutEmit), 2: lineSink(stderrEmit) };
  const unsupportedLogged = new Set();
  let memory = null;

  const view = () => {
    if (!memory) throw new Error('WASI shim: the module memory is not bound.');
    return new DataView(memory.buffer);
  };
  const bytes = () => {
    if (!memory) throw new Error('WASI shim: the module memory is not bound.');
    return new Uint8Array(memory.buffer);
  };
  const inBounds = (ptr, len) => {
    const size = memory ? memory.buffer.byteLength : 0;
    return Number.isInteger(ptr) && Number.isInteger(len) && ptr >= 0 && len >= 0 && ptr + len <= size;
  };
  const u32 = (value) => value >>> 0;
  const isStdio = (fd) => fd === 0 || fd === 1 || fd === 2;

  const impl = {
    fd_write(fd, iovs, iovsLen, nwrittenPtr) {
      fd = u32(fd); iovs = u32(iovs); iovsLen = u32(iovsLen); nwrittenPtr = u32(nwrittenPtr);
      const sink = sinks[fd];
      if (!sink) return ERRNO.BADF;
      if (!inBounds(iovs, iovsLen * 8) || !inBounds(nwrittenPtr, 4)) return ERRNO.FAULT;
      const dv = view();
      const mem = bytes();
      let written = 0;
      for (let i = 0; i < iovsLen; i++) {
        const ptr = dv.getUint32(iovs + i * 8, true);
        const len = dv.getUint32(iovs + i * 8 + 4, true);
        if (!inBounds(ptr, len)) return ERRNO.FAULT;
        if (len) sink.write(mem.subarray(ptr, ptr + len));
        written += len;
      }
      dv.setUint32(nwrittenPtr, written >>> 0, true);
      return ERRNO.SUCCESS;
    },

    environ_sizes_get(countPtr, sizePtr) {
      countPtr = u32(countPtr); sizePtr = u32(sizePtr);
      if (!inBounds(countPtr, 4) || !inBounds(sizePtr, 4)) return ERRNO.FAULT;
      const dv = view();
      dv.setUint32(countPtr, 0, true);
      dv.setUint32(sizePtr, 0, true);
      return ERRNO.SUCCESS;
    },

    environ_get() {
      return ERRNO.SUCCESS;
    },

    args_sizes_get(countPtr, sizePtr) {
      countPtr = u32(countPtr); sizePtr = u32(sizePtr);
      if (!inBounds(countPtr, 4) || !inBounds(sizePtr, 4)) return ERRNO.FAULT;
      const dv = view();
      dv.setUint32(countPtr, 0, true);
      dv.setUint32(sizePtr, 0, true);
      return ERRNO.SUCCESS;
    },

    args_get() {
      return ERRNO.SUCCESS;
    },

    clock_time_get(clockId, _precision, timePtr) {
      clockId = u32(clockId); timePtr = u32(timePtr);
      if (!inBounds(timePtr, 8)) return ERRNO.FAULT;
      let value;
      if (clockId === CLOCK_REALTIME) value = nowNs();
      else if (clockId === CLOCK_MONOTONIC || clockId === CLOCK_PROCESS_CPUTIME || clockId === CLOCK_THREAD_CPUTIME) value = monotonicNs();
      else return ERRNO.INVAL;
      view().setBigUint64(timePtr, value, true);
      return ERRNO.SUCCESS;
    },

    random_get(bufPtr, bufLen) {
      bufPtr = u32(bufPtr); bufLen = u32(bufLen);
      if (!inBounds(bufPtr, bufLen)) return ERRNO.FAULT;
      const cryptoApi = typeof crypto !== 'undefined' ? crypto : null;
      if (!cryptoApi || typeof cryptoApi.getRandomValues !== 'function') return ERRNO.NOSYS;
      const mem = bytes();
      for (let off = 0; off < bufLen; off += RANDOM_CHUNK) {
        const len = Math.min(RANDOM_CHUNK, bufLen - off);
        // A copy keeps getRandomValues away from views it may refuse (shared or resizable buffers).
        const chunk = new Uint8Array(len);
        cryptoApi.getRandomValues(chunk);
        mem.set(chunk, bufPtr + off);
      }
      return ERRNO.SUCCESS;
    },

    proc_exit(code) {
      flushAll();
      throw new WasiExit(code | 0);
    },

    fd_close(fd) {
      fd = u32(fd);
      return isStdio(fd) ? ERRNO.SUCCESS : ERRNO.BADF;
    },

    fd_seek(fd, _offset, _whence, newOffsetPtr) {
      fd = u32(fd);
      if (!isStdio(fd)) return ERRNO.BADF;
      // The standard streams are character devices: they cannot seek.
      void newOffsetPtr;
      return ERRNO.SPIPE;
    },

    fd_fdstat_get(fd, statPtr) {
      fd = u32(fd); statPtr = u32(statPtr);
      if (!isStdio(fd)) return ERRNO.BADF;
      if (!inBounds(statPtr, 24)) return ERRNO.FAULT;
      const dv = view();
      // struct fdstat { u8 filetype; u16 flags @2; u64 rights_base @8; u64 rights_inheriting @16 }
      for (let i = 0; i < 24; i++) dv.setUint8(statPtr + i, 0);
      dv.setUint8(statPtr, FILETYPE_CHARACTER_DEVICE);
      dv.setUint16(statPtr + 2, fd === 0 ? 0 : FDFLAGS_APPEND, true);
      dv.setBigUint64(statPtr + 8, fd === 0 ? RIGHT_FD_READ : RIGHT_FD_WRITE, true);
      dv.setBigUint64(statPtr + 16, 0n, true);
      return ERRNO.SUCCESS;
    },

    fd_prestat_get() {
      // No preopened directories: the module sees an empty capability table.
      return ERRNO.BADF;
    }
  };

  function flushAll() {
    sinks[1].flush();
    sinks[2].flush();
  }

  function unsupported(name) {
    return function unsupportedWasiCall() {
      if (!unsupportedLogged.has(name)) {
        unsupportedLogged.add(name);
        log(`${prefix} The module called WASI "${name}", which the in-memory shim does not provide; it returned ENOSYS (52).`);
      }
      return ERRNO.NOSYS;
    };
  }

  /** The WASI namespace with the implemented functions only. */
  function namespace() {
    const ns = {};
    for (const [name, fn] of Object.entries(impl)) ns[name] = fn;
    return ns;
  }

  /**
   * An import object for a compiled module: every function the module imports from
   * wasi_snapshot_preview1 is provided (implemented or ENOSYS stub). Imports from any other
   * module are reported in `foreign`, because the shim cannot satisfy them.
   * @param {WebAssembly.Module} module
   * @returns {{ imports: object, foreign: string[], stubbed: string[] }}
   */
  function importsFor(module) {
    const ns = namespace();
    const foreign = [];
    const stubbed = [];
    for (const imp of WebAssembly.Module.imports(module)) {
      if (imp.module === WASI_MODULE && imp.kind === 'function') {
        if (!Object.prototype.hasOwnProperty.call(ns, imp.name)) {
          ns[imp.name] = unsupported(imp.name);
          stubbed.push(imp.name);
        }
      } else {
        foreign.push(`${imp.module}.${imp.name} (${imp.kind})`);
      }
    }
    return { imports: { [WASI_MODULE]: ns }, foreign, stubbed };
  }

  return {
    importsFor,
    namespace,
    unsupported,
    bindMemory(mem) {
      memory = mem || null;
    },
    bindInstance(instance) {
      const mem = instance && instance.exports ? instance.exports.memory : null;
      if (!mem || !(mem.buffer instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && mem.buffer instanceof SharedArrayBuffer))) {
        throw new Error('The engine module does not export its linear memory as "memory".');
      }
      memory = mem;
    },
    flush: flushAll
  };
}
