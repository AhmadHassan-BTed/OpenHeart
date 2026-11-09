/**
 * OpenHeart In-Browser Declaration Extractor (linear time, no backtracking regexes)
 *
 * This is an approximate, declaration-level extractor used when the Rust engine is not
 * reachable (for example on GitHub Pages). It recovers type declarations, their members,
 * supertypes, imports and exact declaration positions. It does not build control flow,
 * SSA, call graphs or BDDs; those views require the Rust engine.
 *
 * Pipeline per file, each stage a single forward pass:
 *   1. mask comments and string/char literals (positions and newlines preserved)
 *   2. tokenize the masked text (literal spans become 'str' tokens)
 *   3. pair (), [] and {} with a stack
 *   4. walk scopes to find declarations; members are read only at the top level of each
 *      type body, so method-local variables and call statements never become members.
 */

const EXTENSION_LANGUAGE = {
  '.java': 'java',
  '.kt': 'kotlin',
  '.kts': 'kotlin',
  '.scala': 'scala',
  '.cs': 'csharp',
  '.ts': 'typescript',
  '.tsx': 'typescript',
  '.mts': 'typescript',
  '.cts': 'typescript',
  '.js': 'javascript',
  '.jsx': 'javascript',
  '.mjs': 'javascript',
  '.cjs': 'javascript',
  '.py': 'python',
  '.rs': 'rust',
  '.go': 'go',
  '.swift': 'swift',
  '.c': 'cpp',
  '.h': 'cpp',
  '.cc': 'cpp',
  '.cpp': 'cpp',
  '.cxx': 'cpp',
  '.hh': 'cpp',
  '.hpp': 'cpp',
  '.hxx': 'cpp'
};

export const SUPPORTED_EXTENSIONS = Object.keys(EXTENSION_LANGUAGE);

export function languageForPath(path) {
  const lower = String(path || '').toLowerCase();
  const dot = lower.lastIndexOf('.');
  if (dot < 0 || dot < lower.lastIndexOf('/')) return null;
  return EXTENSION_LANGUAGE[lower.slice(dot)] || null;
}

const LANG = {
  java: { slash: true, single: 'char', backtick: 'none', triple: true, nl: false },
  kotlin: { slash: true, single: 'char', backtick: 'ident', triple: true, nl: true },
  scala: { slash: true, single: 'char', backtick: 'ident', triple: true, nl: true },
  csharp: { slash: true, single: 'char', backtick: 'none', triple: true, verbatim: true, nl: false },
  typescript: { slash: true, single: 'string', backtick: 'string', triple: false, regex: true, nl: true },
  javascript: { slash: true, single: 'string', backtick: 'string', triple: false, regex: true, nl: true },
  python: { hash: true, single: 'string', backtick: 'none', triple: true, nl: true },
  rust: { slash: true, single: 'rust', backtick: 'none', triple: false, rustRaw: true, multiline: true, nl: false },
  go: { slash: true, single: 'char', backtick: 'string', triple: false, nl: true },
  swift: { slash: true, single: 'none', backtick: 'ident', triple: true, nl: true },
  cpp: { slash: true, single: 'char', backtick: 'none', triple: false, cppRaw: true, nl: false }
};

const LITERAL_MARK = 1;

function isIdentStart(c) {
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 95 || c === 36 || c > 127;
}

function isIdentPart(c) {
  return isIdentStart(c) || (c >= 48 && c <= 57);
}

function isDigit(c) {
  return c >= 48 && c <= 57;
}

// ── 1. Literal and comment masking ─────────────────────────────────────────────

function scanQuoted(code, i, quote, allowNewline) {
  const n = code.length;
  let j = i + 1;
  while (j < n) {
    const ch = code.charCodeAt(j);
    if (ch === 92) { j += 2; continue; }
    if (ch === quote) return j + 1;
    if (ch === 10 && !allowNewline) return j;
    j++;
  }
  return n;
}

function scanCharLiteral(code, i) {
  // 'x', '\n', 'A', '\u{1F600}' — anything else is not a char literal.
  const n = code.length;
  if (i + 2 < n && code.charCodeAt(i + 1) === 92) {
    const limit = Math.min(n, i + 14);
    for (let j = i + 2; j < limit; j++) {
      const ch = code.charCodeAt(j);
      if (ch === 39) return j + 1;
      if (ch === 10) return -1;
    }
    return -1;
  }
  if (i + 2 < n && code.charCodeAt(i + 1) !== 10 && code.charCodeAt(i + 2) === 39) return i + 3;
  // Astral character encoded as a surrogate pair.
  if (i + 3 < n && code.charCodeAt(i + 1) >= 0xD800 && code.charCodeAt(i + 1) <= 0xDBFF && code.charCodeAt(i + 3) === 39) return i + 4;
  return -1;
}

function literalEnd(code, i, lang, conf, prevSignificant) {
  const n = code.length;
  const c = code.charCodeAt(i);

  if (c === 34) { // "
    if (conf.triple && code.startsWith('"""', i)) {
      const end = code.indexOf('"""', i + 3);
      return end === -1 ? n : end + 3;
    }
    if (conf.verbatim && i > 0 && (code.charCodeAt(i - 1) === 64 || (i > 1 && code.charCodeAt(i - 2) === 64 && code.charCodeAt(i - 1) === 36))) {
      let j = i + 1;
      while (j < n) {
        if (code.charCodeAt(j) === 34) {
          if (code.charCodeAt(j + 1) === 34) { j += 2; continue; }
          return j + 1;
        }
        j++;
      }
      return n;
    }
    if (conf.rustRaw) {
      let k = i - 1;
      let hashes = 0;
      while (k >= 0 && code.charCodeAt(k) === 35) { hashes++; k--; }
      if (k >= 0 && code.charCodeAt(k) === 114 && (k === 0 || !isIdentPart(code.charCodeAt(k - 1)) || code.charCodeAt(k - 1) === 98)) {
        const terminator = '"' + '#'.repeat(hashes);
        const end = code.indexOf(terminator, i + 1);
        return end === -1 ? n : end + terminator.length;
      }
    }
    if (conf.cppRaw && i > 0 && code.charCodeAt(i - 1) === 82) {
      const paren = code.indexOf('(', i + 1);
      if (paren !== -1 && paren - i <= 17) {
        const delim = code.slice(i + 1, paren);
        if (!/[\s()\\]/.test(delim)) {
          const terminator = ')' + delim + '"';
          const end = code.indexOf(terminator, paren + 1);
          return end === -1 ? n : end + terminator.length;
        }
      }
    }
    return scanQuoted(code, i, 34, Boolean(conf.multiline));
  }

  if (c === 39) { // '
    if (conf.single === 'string') {
      if (conf.triple && code.startsWith("'''", i)) {
        const end = code.indexOf("'''", i + 3);
        return end === -1 ? n : end + 3;
      }
      return scanQuoted(code, i, 39, false);
    }
    if (conf.single === 'char' || conf.single === 'rust') {
      return scanCharLiteral(code, i);
    }
    return -1;
  }

  if (c === 96 && conf.backtick === 'string') { // `
    return scanQuoted(code, i, 96, true);
  }

  if (c === 47 && conf.regex) { // possible regex literal in JS/TS
    if (prevSignificant === -1 || '(,=:[!&|?{};+-*%<>~^'.includes(String.fromCharCode(prevSignificant))) {
      let j = i + 1;
      let inClass = false;
      while (j < n) {
        const ch = code.charCodeAt(j);
        if (ch === 10) return -1;
        if (ch === 92) { j += 2; continue; }
        if (ch === 91) inClass = true;
        else if (ch === 93) inClass = false;
        else if (ch === 47 && !inClass) {
          j++;
          while (j < n && isIdentPart(code.charCodeAt(j))) j++;
          return j;
        }
        j++;
      }
      return -1;
    }
  }

  return -1;
}

function blank(segment, mark) {
  const spaced = segment.replace(/[^\n]/g, ' ');
  if (!mark || spaced.length === 0) return spaced;
  return String.fromCharCode(LITERAL_MARK) + spaced.slice(1);
}

function maskSource(code, lang) {
  const conf = LANG[lang];
  const n = code.length;
  const parts = [];
  const spanStarts = [];
  const spanEnds = [];
  let last = 0;
  let i = 0;
  let prevSignificant = -1;

  while (i < n) {
    const c = code.charCodeAt(i);

    if (conf.slash && c === 47) {
      const d = code.charCodeAt(i + 1);
      if (d === 47) {
        let end = code.indexOf('\n', i);
        if (end === -1) end = n;
        parts.push(code.slice(last, i), blank(code.slice(i, end), false));
        i = last = end;
        continue;
      }
      if (d === 42) {
        const close = code.indexOf('*/', i + 2);
        const end = close === -1 ? n : close + 2;
        parts.push(code.slice(last, i), blank(code.slice(i, end), false));
        i = last = end;
        continue;
      }
    }

    if (conf.hash && c === 35) {
      let end = code.indexOf('\n', i);
      if (end === -1) end = n;
      parts.push(code.slice(last, i), blank(code.slice(i, end), false));
      i = last = end;
      continue;
    }

    if (c === 34 || c === 39 || c === 96 || (c === 47 && conf.regex)) {
      const end = literalEnd(code, i, lang, conf, prevSignificant);
      if (end > i) {
        parts.push(code.slice(last, i), blank(code.slice(i, end), true));
        spanStarts.push(i);
        spanEnds.push(end);
        i = last = end;
        prevSignificant = 34;
        continue;
      }
    }

    if (c !== 32 && c !== 9 && c !== 10 && c !== 13) prevSignificant = c;
    i++;
  }
  parts.push(code.slice(last));
  return { masked: parts.join(''), spanStarts, spanEnds };
}

// ── 2. Tokenizer ───────────────────────────────────────────────────────────────

const TWO_CHAR_PUNCT = new Set(['->', '=>', '::', '?.', '?:', '!!', '==', '!=', '&&', '||', '++', '--', '+=', '-=', '*=', '/=', '%=', '|=', '&=', '^=', ':=']);
const THREE_CHAR_PUNCT = new Set(['...', '===', '!==']);

function buildLineStarts(code) {
  const starts = [0];
  for (let i = 0; i < code.length; i++) {
    if (code.charCodeAt(i) === 10) starts.push(i + 1);
  }
  return starts;
}

function tokenize(code, masked, spanStarts, spanEnds, lang) {
  const conf = LANG[lang];
  const tokens = [];
  const n = masked.length;
  const lineStarts = buildLineStarts(code);
  let lineIdx = 0;
  let spanPtr = 0;
  let lastWasNl = true;

  const push = (t, v, s, e) => {
    while (lineIdx + 1 < lineStarts.length && lineStarts[lineIdx + 1] <= s) lineIdx++;
    tokens.push({ t, v, s, e, line: lineIdx + 1, col: s - lineStarts[lineIdx] + 1 });
  };

  let i = 0;
  while (i < n) {
    const c = masked.charCodeAt(i);
    if (c === 10) {
      if (!lastWasNl) {
        push('nl', '\n', i, i + 1);
        lastWasNl = true;
      }
      i++;
      continue;
    }
    if (c === 32 || c === 9 || c === 13 || c === 12 || c === 11 || c === 0xFEFF) {
      i++;
      continue;
    }
    lastWasNl = false;

    if (c === LITERAL_MARK) {
      while (spanPtr < spanStarts.length && spanStarts[spanPtr] < i) spanPtr++;
      const end = (spanPtr < spanStarts.length && spanStarts[spanPtr] === i) ? spanEnds[spanPtr] : i + 1;
      push('str', '', i, end);
      i = end;
      continue;
    }

    if (isIdentStart(c)) {
      let j = i + 1;
      while (j < n && isIdentPart(masked.charCodeAt(j))) j++;
      push('id', masked.slice(i, j), i, j);
      i = j;
      continue;
    }

    if (isDigit(c)) {
      let j = i + 1;
      while (j < n) {
        const d = masked.charCodeAt(j);
        if (isIdentPart(d) || (d === 46 && j + 1 < n && isDigit(masked.charCodeAt(j + 1)))) j++;
        else break;
      }
      push('num', masked.slice(i, j), i, j);
      i = j;
      continue;
    }

    if (c === 96 && conf.backtick === 'ident') {
      const close = masked.indexOf('`', i + 1);
      const eol = masked.indexOf('\n', i + 1);
      if (close > i && (eol === -1 || close < eol)) {
        push('id', masked.slice(i + 1, close), i, close + 1);
        i = close + 1;
        continue;
      }
    }

    if (c === 35 && (lang === 'javascript' || lang === 'typescript') && i + 1 < n && isIdentStart(masked.charCodeAt(i + 1))) {
      let j = i + 2;
      while (j < n && isIdentPart(masked.charCodeAt(j))) j++;
      push('id', masked.slice(i, j), i, j);
      i = j;
      continue;
    }

    const three = masked.substr(i, 3);
    if (THREE_CHAR_PUNCT.has(three)) {
      push('p', three, i, i + 3);
      i += 3;
      continue;
    }
    const two = masked.substr(i, 2);
    if (TWO_CHAR_PUNCT.has(two)) {
      push('p', two, i, i + 2);
      i += 2;
      continue;
    }
    push('p', masked[i], i, i + 1);
    i++;
  }
  return { tokens, lineCount: lineStarts.length };
}

function matchBrackets(tokens) {
  const match = new Int32Array(tokens.length).fill(-1);
  const stack = [];
  for (let i = 0; i < tokens.length; i++) {
    const tk = tokens[i];
    if (tk.t !== 'p') continue;
    const v = tk.v;
    if (v === '(' || v === '[' || v === '{') {
      stack.push(i);
    } else if (v === ')' || v === ']') {
      const want = v === ')' ? '(' : '[';
      if (stack.length && tokens[stack[stack.length - 1]].v === want) {
        const open = stack.pop();
        match[open] = i;
        match[i] = open;
      }
    } else if (v === '}') {
      while (stack.length && tokens[stack[stack.length - 1]].v !== '{') stack.pop();
      if (stack.length) {
        const open = stack.pop();
        match[open] = i;
        match[i] = open;
      }
    }
  }
  return match;
}

// ── 3. Shared vocabulary ───────────────────────────────────────────────────────

const VISIBILITY_WORDS = {
  public: '+',
  pub: '+',
  open: null,
  private: '-',
  fileprivate: '-',
  protected: '#',
  internal: '~',
  package: '~'
};

const MODIFIERS = new Set([
  'public', 'private', 'protected', 'internal', 'fileprivate', 'pub', 'static', 'final', 'abstract',
  'open', 'override', 'readonly', 'const', 'async', 'suspend', 'inline', 'virtual', 'sealed', 'transient',
  'volatile', 'synchronized', 'native', 'default', 'lateinit', 'operator', 'infix', 'tailrec', 'external',
  'expect', 'actual', 'data', 'value', 'companion', 'extern', 'unsafe', 'mutating', 'nonmutating', 'dynamic',
  'optional', 'required', 'convenience', 'weak', 'unowned', 'lazy', 'implicit', 'partial', 'explicit',
  'constexpr', 'friend', 'mutable', 'register', 'thread_local', 'declare', 'export', 'strictfp', 'noinline',
  'crossinline', 'reified', 'event', 'unsafe', 'extern', 'nonisolated', 'isolated', 'consteval', 'constinit'
]);

const NESTED_DECLARATION_WORDS = new Set([
  'enum', 'annotation', 'typealias', 'extension', 'impl', 'namespace', 'import', 'package', 'using', 'use',
  'typedef', 'friend', 'static_assert', 'mod', 'macro_rules', 'deinit', 'subscript'
]);

const DECL_KEYWORDS = new Set(['val', 'var', 'let']);
const FUN_KEYWORDS = new Set(['fun', 'fn', 'def', 'func', 'function']);

const CONTROL_WORDS = new Set([
  'if', 'for', 'while', 'switch', 'catch', 'when', 'match', 'synchronized', 'return', 'new', 'throw', 'else',
  'do', 'try', 'finally', 'foreach', 'using', 'lock', 'yield', 'await', 'delete', 'typeof', 'sizeof', 'assert',
  'super', 'this', 'guard', 'defer', 'select', 'loop', 'go', 'sizeof', 'alignof', 'decltype', 'static_assert'
]);

const NOT_A_TYPE_NAME = new Set([
  'extends', 'implements', 'where', 'func', 'fun', 'var', 'let', 'val', 'new', 'class', 'interface', 'struct',
  'enum', 'object', 'trait', 'record', 'with', 'permits', 'fn', 'def', 'is', 'as', 'in', 'const', 'static'
]);

const COLLECTION_TYPES = new Set([
  'List', 'ArrayList', 'LinkedList', 'MutableList', 'Set', 'HashSet', 'TreeSet', 'LinkedHashSet', 'MutableSet',
  'SortedSet', 'NavigableSet', 'Map', 'HashMap', 'TreeMap', 'LinkedHashMap', 'MutableMap', 'SortedMap',
  'ConcurrentHashMap', 'ConcurrentMap', 'Collection', 'Iterable', 'Queue', 'Deque', 'ArrayDeque', 'Stack',
  'Vector', 'Array', 'Sequence', 'Seq', 'Vec', 'VecDeque', 'HashMap', 'BTreeMap', 'BTreeSet', 'Dictionary',
  'IEnumerable', 'ICollection', 'IList', 'IDictionary', 'IReadOnlyList', 'IReadOnlyCollection', 'ISet',
  'ReadonlyArray', 'Record', 'Iterator', 'Stream', 'Flow', 'Observable', 'Multimap', 'ImmutableList',
  'ImmutableSet', 'ImmutableMap', 'Page', 'Slice', 'Tuple', 'list', 'dict', 'set', 'tuple', 'frozenset',
  'Sequence', 'Mapping', 'MutableMapping', 'deque', 'vector', 'map', 'unordered_map', 'unordered_set',
  'multimap', 'multiset', 'deque', 'array', 'span'
]);

const TRANSPARENT_WRAPPERS = new Set([
  'Optional', 'Option', 'Nullable', 'Lazy', 'Box', 'Rc', 'Arc', 'Ref', 'RefCell', 'Cell', 'Mutex', 'RwLock',
  'Weak', 'AtomicReference', 'Future', 'CompletableFuture', 'Promise', 'Task', 'Mono', 'Supplier',
  'Provider', 'unique_ptr', 'shared_ptr', 'weak_ptr', 'optional', 'Optional', 'Readonly', 'Partial',
  'Required', 'ReadOnly', 'Cow', 'Pin', 'ClassVar', 'Final', 'Annotated', 'Union', 'Deferred', 'LiveData',
  'StateFlow', 'MutableStateFlow', 'MutableLiveData', 'Signal', 'Rx', 'Single', 'Maybe'
]);

export const BUILTIN_TYPE_NAMES = new Set([
  'String', 'Object', 'Integer', 'Long', 'Short', 'Byte', 'Double', 'Float', 'Boolean', 'Character', 'Number',
  'Void', 'Any', 'Unit', 'Nothing', 'Int', 'Char', 'UInt', 'ULong', 'BigDecimal', 'BigInteger', 'CharSequence',
  'Enum', 'Exception', 'RuntimeException', 'Throwable', 'Error', 'Class', 'Date', 'LocalDate', 'LocalDateTime',
  'Instant', 'Duration', 'UUID', 'URI', 'URL', 'Path', 'File', 'Thread', 'Runnable', 'Callable', 'Comparable',
  'Cloneable', 'Serializable', 'AutoCloseable', 'Closeable', 'Iterable', 'Function', 'BiFunction', 'Consumer',
  'Predicate', 'Supplier', 'StringBuilder', 'Pattern', 'Matcher', 'Random', 'Locale', 'Charset', 'ValueType',
  'Guid', 'DateTime', 'TimeSpan', 'Decimal', 'Task', 'Action', 'Func', 'EventHandler', 'IDisposable',
  'Self', 'Vec', 'Option', 'Result', 'Box', 'HashMap', 'HashSet', 'str', 'Some', 'None', 'Ok', 'Err',
  'Promise', 'Array', 'Map', 'Set', 'Record', 'Partial', 'Date', 'RegExp', 'Error', 'Symbol', 'BigInt',
  'object', 'string', 'number', 'boolean', 'void', 'never', 'unknown', 'any', 'undefined', 'null',
  'int', 'long', 'short', 'byte', 'double', 'float', 'boolean', 'char', 'bool', 'usize', 'isize', 'u8', 'u16',
  'u32', 'u64', 'u128', 'i8', 'i16', 'i32', 'i64', 'i128', 'f32', 'f64', 'uint', 'ulong', 'sbyte', 'ushort',
  'decimal', 'dynamic', 'auto', 'size_t', 'uint8_t', 'uint16_t', 'uint32_t', 'uint64_t', 'int8_t',
  'int16_t', 'int32_t', 'int64_t', 'std', 'unsigned', 'signed', 'rune', 'error', 'uintptr', 'float32',
  'float64', 'complex64', 'complex128', 'int16', 'int32', 'int64', 'int8', 'uint16', 'uint32', 'uint64',
  'uint8', 'bytes', 'dict', 'list', 'set', 'tuple', 'frozenset', 'type', 'Type', 'Callable', 'Iterator',
  'Generator', 'Awaitable', 'Coroutine', 'Protocol', 'ABC', 'Generic', 'TypeVar', 'NamedTuple',
  'TypedDict', 'Mapping', 'Sequence', 'Optional', 'Union', 'Literal', 'Final', 'ClassVar', 'Annotated'
]);

function newlineTerminated(lang) {
  return LANG[lang].nl === true;
}

const CONTINUATION_TAIL = new Set(['.', ',', '=', ':', '->', '=>', '(', '[', '<', '+', '-', '*', '/', '&&', '||', '?:', '?', '|', '&', '?.', '::']);
const CONTINUATION_HEAD = new Set(['.', '?.', ':', '=', '->', '=>', '{', ')', ']', ',', '&&', '||', '?:', '|', '::', '>']);
const CONTINUATION_HEAD_WORDS = new Set(['where', 'extends', 'implements', 'throws', 'with', 'by', 'else', 'catch', 'finally']);

// ── 4. Parser context ──────────────────────────────────────────────────────────

class ParseContext {
  constructor(path, code, lang) {
    this.path = path;
    this.code = code;
    this.lang = lang;
    const { masked, spanStarts, spanEnds } = maskSource(code, lang);
    this.masked = masked;
    const { tokens, lineCount } = tokenize(code, masked, spanStarts, spanEnds, lang);
    this.tokens = tokens;
    this.lineCount = lineCount;
    this.match = matchBrackets(tokens);
    this.result = {
      path,
      language: lang,
      packageDecl: null,
      fileNamespace: [],
      imports: [],
      wildcardImports: [],
      classes: [],
      extensions: [],
      lineCount,
      diagnostics: []
    };
  }

  tok(i) {
    return i >= 0 && i < this.tokens.length ? this.tokens[i] : null;
  }

  nextSig(i) {
    let j = i + 1;
    while (j < this.tokens.length && this.tokens[j].t === 'nl') j++;
    return j;
  }

  prevSig(i) {
    let j = i - 1;
    while (j >= 0 && this.tokens[j].t === 'nl') j--;
    return j;
  }

  isP(i, v) {
    const t = this.tok(i);
    return Boolean(t && t.t === 'p' && t.v === v);
  }

  isId(i, v) {
    const t = this.tok(i);
    return Boolean(t && t.t === 'id' && (v === undefined || t.v === v));
  }

  closeOf(i) {
    const m = this.match[i];
    return m >= 0 ? m : this.tokens.length;
  }

  /** Skips a generic parameter list starting at '<'; returns the index after the closing '>'. */
  skipAngles(i, limit) {
    if (!this.isP(i, '<')) return i;
    let depth = 0;
    const end = Math.min(limit, this.tokens.length, i + 600);
    for (let k = i; k < end; k++) {
      const t = this.tokens[k];
      if (t.t !== 'p') continue;
      if (t.v === '<') depth++;
      else if (t.v === '>') {
        depth--;
        if (depth === 0) return k + 1;
      } else if (t.v === '->' && depth > 0) {
        continue;
      } else if ((t.v === '(' || t.v === '[') && this.match[k] > k) {
        k = this.match[k];
      } else if (t.v === '{' || t.v === '}' || t.v === ';') {
        return i;
      }
    }
    return i;
  }

  text(s, e) {
    return this.code.slice(s, e);
  }

  /** Reconstructs a compact type expression from tokens [a, b). Returns null when empty. */
  typeText(a, b) {
    let out = '';
    let prev = null;
    for (let k = a; k < b; k++) {
      const t = this.tokens[k];
      if (t.t === 'nl') continue;
      let piece = t.t === 'str' ? this.code.slice(t.s, t.e) : t.v;
      if (piece.length > 80) piece = piece.slice(0, 80);
      if (prev) {
        const wordy = (x) => x.t === 'id' || x.t === 'num' || x.t === 'str';
        if (wordy(prev) && wordy(t)) out += ' ';
        else if (prev.t === 'p' && prev.v === ',') out += ' ';
        else if (t.t === 'p' && (t.v === '|' || t.v === '->' || t.v === '=>')) out += ' ';
        else if (prev.t === 'p' && (prev.v === '|' || prev.v === '->' || prev.v === '=>')) out += ' ';
      }
      out += piece;
      prev = t;
      if (out.length > 160) {
        out = out.slice(0, 157) + '...';
        break;
      }
    }
    return out.trim() || null;
  }

  /**
   * Collects type names referenced in tokens [a, b) for relation extraction.
   * Each entry: { name: 'pkg.Type' | 'Type', collection: boolean }.
   * Names inside collection generics (List<Foo>), array types (Foo[]) and slices ([]Foo)
   * are flagged as collection elements; transparent wrappers (Optional<Foo>) are unwrapped.
   */
  typeRefs(a, b) {
    const refs = [];
    const stack = []; // { collection: boolean, close: index of ']' or -1 for '<' }
    const bracketGenerics = this.lang === 'python' || this.lang === 'scala' || this.lang === 'go';
    let sliceNext = false;
    let k = a;
    while (k < b) {
      const t = this.tokens[k];
      if (t.t === 'id') {
        let name = t.v;
        let m = k;
        while (m + 2 < b && (this.isP(m + 1, '.') || this.isP(m + 1, '::')) && this.isId(m + 2)) {
          name += '.' + this.tokens[m + 2].v;
          m += 2;
        }
        const simple = name.split('.').pop();
        const after = m + 1;
        const arraySuffix = this.isP(after, '[') && this.match[after] === after + 1;
        const opensAngle = this.isP(after, '<');
        const opensBracket = bracketGenerics && this.isP(after, '[') && this.match[after] > after + 1;
        if (MODIFIERS.has(simple) || NOT_A_TYPE_NAME.has(simple)) {
          k = m + 1;
          continue;
        }
        if (this.lang === 'go' && name === 'map' && opensBracket) {
          k = this.match[after] + 1;
          sliceNext = true;
          continue;
        }
        const insideCollection = sliceNext || stack.some(s => s.collection);
        const isCollection = COLLECTION_TYPES.has(simple) && !name.includes('.');
        const isTransparent = TRANSPARENT_WRAPPERS.has(simple);
        if (!isCollection && !isTransparent) {
          refs.push({ name, collection: insideCollection || arraySuffix });
          sliceNext = false;
        }
        if (opensAngle) {
          stack.push({ collection: isCollection, close: -1 });
          k = after + 1;
          continue;
        }
        if (opensBracket) {
          stack.push({ collection: isCollection, close: this.match[after] });
          k = after + 1;
          continue;
        }
        k = m + 1;
        continue;
      }
      if (t.t === 'p') {
        if (t.v === '<') {
          stack.push({ collection: false, close: -1 });
        } else if (t.v === '>') {
          for (let s = stack.length - 1; s >= 0; s--) {
            if (stack[s].close === -1) { stack.splice(s, 1); break; }
          }
        } else if (t.v === ']') {
          if (stack.length && stack[stack.length - 1].close === k) stack.pop();
        } else if (t.v === '[' && this.match[k] === k + 1) {
          // Prefix slice type: []Foo
          sliceNext = true;
          k += 2;
          continue;
        }
      }
      k++;
    }
    return refs;
  }
}

function visibilityFromModifiers(mods) {
  for (const m of mods) {
    if (Object.prototype.hasOwnProperty.call(VISIBILITY_WORDS, m) && VISIBILITY_WORDS[m]) return VISIBILITY_WORDS[m];
  }
  return null;
}

function defaultMemberVisibility(lang, rec, name) {
  switch (lang) {
    case 'java':
      return rec && (rec.kind === 'interface') ? '+' : '~';
    case 'csharp':
      return rec && rec.kind === 'interface' ? '+' : '-';
    case 'cpp':
      return rec && rec.cppDefault ? rec.cppDefault : '-';
    case 'rust':
      return rec && rec.kind === 'interface' ? '+' : '-';
    case 'go':
      return name && /^[A-Z]/.test(name) ? '+' : '-';
    case 'swift':
      return '~';
    case 'python':
      if (!name) return '+';
      if (name.startsWith('__') && !name.endsWith('__')) return '-';
      if (name.startsWith('_')) return '#';
      return '+';
    default:
      return '+';
  }
}

function isCollectionType(typeStr) {
  if (!typeStr) return false;
  if (/\[\]$/.test(typeStr) || /^\[\]/.test(typeStr)) return true;
  const head = typeStr.replace(/^[&*\s]+/, '').split(/[<\[(\s?]/)[0].split(/[.:]/).pop();
  return COLLECTION_TYPES.has(head);
}

// ── 5. Brace-language parser ───────────────────────────────────────────────────

const CLASS_KEYWORDS = {
  java: new Set(['class', 'interface', 'enum', 'record']),
  kotlin: new Set(['class', 'interface', 'object']),
  scala: new Set(['class', 'trait', 'object']),
  csharp: new Set(['class', 'interface', 'enum', 'struct', 'record']),
  typescript: new Set(['class', 'interface', 'enum']),
  javascript: new Set(['class']),
  swift: new Set(['class', 'struct', 'enum', 'protocol', 'actor']),
  rust: new Set(['struct', 'enum', 'trait', 'union']),
  cpp: new Set(['class', 'struct', 'union', 'enum']),
  go: new Set()
};

function kindForKeyword(keyword, prevWord, lang) {
  if (prevWord === 'enum') return { kind: 'enum', stereotype: 'enum' };
  if (prevWord === 'annotation' || prevWord === '@') return { kind: 'interface', stereotype: 'annotation' };
  switch (keyword) {
    case 'interface':
    case 'protocol':
      return { kind: 'interface', stereotype: keyword === 'protocol' ? 'protocol' : 'interface' };
    case 'trait':
      return lang === 'scala' || lang === 'rust' ? { kind: 'interface', stereotype: 'trait' } : { kind: 'class', stereotype: 'trait' };
    case 'enum':
      return { kind: 'enum', stereotype: 'enum' };
    case 'record':
      return { kind: 'class', stereotype: 'record' };
    case 'struct':
      return { kind: 'class', stereotype: 'struct' };
    case 'union':
      return { kind: 'class', stereotype: 'union' };
    case 'object':
      return { kind: 'class', stereotype: 'object' };
    case 'actor':
      return { kind: 'class', stereotype: 'actor' };
    default:
      return { kind: 'class', stereotype: 'class' };
  }
}

function collectLeadingModifiers(ctx, keywordIndex) {
  const mods = [];
  let k = ctx.prevSig(keywordIndex);
  let guard = 0;
  while (k >= 0 && guard < 24) {
    const t = ctx.tokens[k];
    if (t.t === 'id' && (MODIFIERS.has(t.v) || t.v === 'case' || t.v === 'enum' || t.v === 'annotation' || t.v === 'record' || t.v === 'fun')) {
      mods.push(t.v);
      k = ctx.prevSig(k);
      guard++;
      continue;
    }
    if (t.t === 'p' && t.v === ')' && ctx.match[k] >= 0) {
      // annotation argument list, e.g. @Entity(name = "x")
      const open = ctx.match[k];
      const before = ctx.prevSig(open);
      if (before >= 0 && ctx.tokens[before].t === 'id' && ctx.isP(ctx.prevSig(before), '@')) {
        k = ctx.prevSig(ctx.prevSig(before));
        guard++;
        continue;
      }
      break;
    }
    if (t.t === 'id' && ctx.isP(ctx.prevSig(k), '@')) {
      k = ctx.prevSig(ctx.prevSig(k));
      guard++;
      continue;
    }
    if (t.t === 'p' && t.v === ']' && (ctx.lang === 'csharp' || ctx.lang === 'rust') && ctx.match[k] >= 0) {
      k = ctx.prevSig(ctx.match[k]);
      if (ctx.lang === 'rust' && ctx.isP(k, '#')) k = ctx.prevSig(k);
      guard++;
      continue;
    }
    break;
  }
  return mods;
}

function parseHeritage(ctx, s, e) {
  const items = [];
  let role = 'unknown';
  let skipUntilComma = false;
  for (let k = s; k < e; k++) {
    const t = ctx.tokens[k];
    if (t.t === 'nl') continue;
    if (t.t === 'p') {
      if (t.v === ',') { skipUntilComma = false; continue; }
      if (skipUntilComma) {
        if ((t.v === '(' || t.v === '[') && ctx.match[k] > k) k = ctx.match[k];
        continue;
      }
      if (t.v === ':') { role = 'colon'; continue; }
      if (t.v === '+' || t.v === '&') continue;
      if ((t.v === '(' || t.v === '[') && ctx.match[k] > k) { k = ctx.match[k]; continue; }
      if (t.v === '<') { k = ctx.skipAngles(k, e) - 1; continue; }
      continue;
    }
    if (t.t !== 'id') continue;
    if (skipUntilComma) continue;
    const v = t.v;
    if (v === 'extends') { role = 'extends'; continue; }
    if (v === 'implements') { role = 'implements'; continue; }
    if (v === 'with') { role = 'implements'; continue; }
    if (v === 'permits' || v === 'where' || v === 'throws' || v === 'requires') break;
    if (v === 'by') { skipUntilComma = true; continue; }
    if (v === 'public' || v === 'private' || v === 'protected' || v === 'virtual' || v === 'sealed' ||
        v === 'abstract' || v === 'final' || v === 'open' || v === 'override' || v === 'out' || v === 'in' ||
        v === 'dyn' || v === 'impl' || v === 'unsafe' || v === 'new') continue;
    if (role === 'unknown' && ctx.lang !== 'rust') continue;
    let name = v;
    let k2 = k;
    while ((ctx.isP(k2 + 1, '.') || ctx.isP(k2 + 1, '::')) && ctx.isId(k2 + 2)) {
      name += '.' + ctx.tokens[k2 + 2].v;
      k2 += 2;
    }
    k = k2;
    let isCall = false;
    if (ctx.isP(k + 1, '<')) {
      k = ctx.skipAngles(k + 1, e) - 1;
    }
    if (ctx.isP(k + 1, '(') && ctx.match[k + 1] > k + 1) {
      isCall = true;
      k = ctx.match[k + 1];
    }
    items.push({ name, isCall, role, line: t.line });
  }
  return items;
}

function namespacePath(scopes) {
  const out = [];
  for (const sc of scopes) {
    if (sc.kind === 'namespace') out.push(...sc.ns);
  }
  return out;
}

function enclosingClass(scopes) {
  for (let k = scopes.length - 1; k >= 0; k--) {
    if (scopes[k].kind === 'class') return scopes[k].cls;
    if (scopes[k].kind === 'block' || scopes[k].kind === 'extension') return null;
  }
  return null;
}

function readQualified(ctx, i, separators) {
  let name = ctx.tokens[i].v;
  let k = i;
  while (separators.some(sep => ctx.isP(k + 1, sep)) && ctx.isId(k + 2)) {
    name += '.' + ctx.tokens[k + 2].v;
    k += 2;
  }
  return { name, end: k };
}

function statementEnd(ctx, i) {
  // Index of the token ending a directive statement (';' or newline), bounded.
  const limit = Math.min(ctx.tokens.length, i + 400);
  for (let k = i; k < limit; k++) {
    const t = ctx.tokens[k];
    if (t.t === 'p' && t.v === ';') return k;
    if (t.t === 'nl') return k;
    if (t.t === 'p' && t.v === '{' && ctx.match[k] > k) {
      k = ctx.match[k];
    }
  }
  return limit;
}

function resolveRelativeModule(fromPath, spec) {
  const fromDir = fromPath.includes('/') ? fromPath.slice(0, fromPath.lastIndexOf('/')) : '';
  const parts = fromDir ? fromDir.split('/') : [];
  for (const seg of spec.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') parts.pop();
    else parts.push(seg);
  }
  return parts.join('/');
}

function tryDirective(ctx, i, scope, nsAtOpen) {
  const t = ctx.tokens[i];
  const lang = ctx.lang;
  const res = ctx.result;
  const prev = ctx.tok(ctx.prevSig(i));
  if (prev && prev.t === 'p' && (prev.v === '.' || prev.v === '::')) return -1;

  if (t.v === 'package' && (lang === 'java' || lang === 'kotlin' || lang === 'scala' || lang === 'go') && scope.kind === 'file') {
    const j = ctx.nextSig(i);
    if (!ctx.isId(j)) return -1;
    const q = readQualified(ctx, j, ['.']);
    if (!res.packageDecl) res.packageDecl = q.name;
    return q.end + 1;
  }

  if (t.v === 'import' && (lang === 'java' || lang === 'kotlin' || lang === 'scala')) {
    let j = ctx.nextSig(i);
    if (ctx.isId(j, 'static')) j = ctx.nextSig(j);
    if (!ctx.isId(j)) return -1;
    const q = readQualified(ctx, j, ['.']);
    let k = q.end + 1;
    if (ctx.isP(k, '.') && (ctx.isP(k + 1, '*') || ctx.isId(k + 1, '_'))) {
      res.wildcardImports.push(q.name);
      return k + 2;
    }
    if (ctx.isP(k, '.') && ctx.isP(k + 1, '{') && lang === 'scala') {
      const close = ctx.closeOf(k + 1);
      for (let m = k + 2; m < close; m++) {
        if (ctx.isId(m)) {
          const original = ctx.tokens[m].v;
          let alias = original;
          if (ctx.isP(m + 1, '=>') && ctx.isId(m + 2)) { alias = ctx.tokens[m + 2].v; m += 2; }
          if (original === '_') res.wildcardImports.push(q.name);
          else res.imports.push({ alias, target: `${q.name}.${original}` });
        }
      }
      return close + 1;
    }
    let alias = q.name.split('.').pop();
    if (ctx.isId(k, 'as') && ctx.isId(k + 1)) alias = ctx.tokens[k + 1].v;
    res.imports.push({ alias, target: q.name });
    return k;
  }

  if (t.v === 'using' && lang === 'csharp') {
    let j = ctx.nextSig(i);
    if (ctx.isId(j, 'static')) return statementEnd(ctx, j);
    if (!ctx.isId(j)) return -1;
    if (ctx.isP(j + 1, '=') && ctx.isId(j + 2)) {
      const q = readQualified(ctx, j + 2, ['.']);
      res.imports.push({ alias: ctx.tokens[j].v, target: q.name });
      return q.end + 1;
    }
    const q = readQualified(ctx, j, ['.']);
    if (ctx.isP(q.end + 1, ';')) res.wildcardImports.push(q.name);
    return q.end + 1;
  }

  if (t.v === 'use' && lang === 'rust') {
    const end = statementEnd(ctx, i);
    const walk = (s, e, prefix) => {
      let k = s;
      while (k < e) {
        const tk = ctx.tokens[k];
        if (tk.t === 'id') {
          let path = prefix;
          let m = k;
          let segment = tk.v;
          while (true) {
            if (segment !== 'crate' && segment !== 'self' && segment !== 'super') path = path ? `${path}.${segment}` : segment;
            if (ctx.isP(m + 1, '::') && ctx.isId(m + 2)) { m += 2; segment = ctx.tokens[m].v; continue; }
            break;
          }
          if (ctx.isP(m + 1, '::') && ctx.isP(m + 2, '{')) {
            const close = ctx.closeOf(m + 2);
            walk(m + 3, close, path);
            k = close + 1;
            continue;
          }
          if (ctx.isP(m + 1, '::') && ctx.isP(m + 2, '*')) {
            res.wildcardImports.push(path);
            k = m + 3;
            continue;
          }
          let alias = path.split('.').pop();
          if (ctx.isId(m + 1, 'as') && ctx.isId(m + 2)) { alias = ctx.tokens[m + 2].v; m += 2; }
          if (path) res.imports.push({ alias, target: path });
          k = m + 1;
          continue;
        }
        k++;
      }
    };
    walk(ctx.nextSig(i), end, '');
    return end;
  }

  if (t.v === 'import' && (lang === 'typescript' || lang === 'javascript')) {
    const end = Math.min(ctx.tokens.length, i + 400);
    const names = [];
    let fromIdx = -1;
    for (let k = i + 1; k < end; k++) {
      const tk = ctx.tokens[k];
      if (tk.t === 'id' && tk.v === 'from') { fromIdx = k; break; }
      if (tk.t === 'p' && tk.v === ';') break;
      if (tk.t === 'str') break;
    }
    if (fromIdx === -1) return -1;
    let k = ctx.nextSig(i);
    if (ctx.isId(k, 'type')) k = ctx.nextSig(k);
    while (k < fromIdx) {
      const tk = ctx.tokens[k];
      if (tk.t === 'p' && tk.v === '*') {
        // Namespace import `* as NS`: members are referenced qualified, nothing to bind.
        k = ctx.nextSig(ctx.nextSig(ctx.nextSig(k)));
        continue;
      }
      if (tk.t === 'p' && tk.v === '{') {
        const close = Math.min(ctx.closeOf(k), fromIdx);
        splitTopLevel(ctx, k + 1, close, ',').forEach(([ss, se]) => {
          let m = ss;
          while (m < se && (ctx.tokens[m].t === 'nl' || ctx.isId(m, 'type'))) m++;
          if (!ctx.isId(m)) return;
          const original = ctx.tokens[m].v;
          let alias = original;
          const asIdx = ctx.nextSig(m);
          if (asIdx < se && ctx.isId(asIdx, 'as') && ctx.isId(ctx.nextSig(asIdx))) alias = ctx.tokens[ctx.nextSig(asIdx)].v;
          names.push({ original, alias });
        });
        k = close + 1;
        continue;
      }
      if (tk.t === 'id') {
        // Default import binds the module's default export under a local name.
        names.push({ original: tk.v, alias: tk.v });
      }
      k = ctx.nextSig(k);
    }
    const specTok = ctx.tok(ctx.nextSig(fromIdx));
    if (!specTok || specTok.t !== 'str') return fromIdx + 1;
    const spec = ctx.code.slice(specTok.s + 1, specTok.e - 1);
    if (spec.startsWith('.')) {
      const modulePath = resolveRelativeModule(ctx.path, spec);
      const moduleDir = modulePath.includes('/') ? modulePath.slice(0, modulePath.lastIndexOf('/')) : '';
      const pkg = moduleDir.split('/').filter(Boolean).join('.');
      names.forEach(({ original, alias }) => {
        res.imports.push({ alias, target: pkg ? `${pkg}.${original}` : original, modulePath });
      });
    }
    return ctx.nextSig(fromIdx) + 1;
  }

  if ((t.v === 'namespace' && (lang === 'csharp' || lang === 'cpp' || lang === 'typescript')) ||
      (t.v === 'module' && lang === 'typescript') || (t.v === 'mod' && lang === 'rust')) {
    let j = ctx.nextSig(i);
    if (t.v === 'namespace' && lang === 'cpp' && ctx.isP(j, '{')) {
      nsAtOpen.set(j, []);
      return j;
    }
    if (!ctx.isId(j)) return -1;
    const q = readQualified(ctx, j, lang === 'cpp' ? ['::', '.'] : ['.']);
    const after = ctx.nextSig(q.end);
    if (ctx.isP(after, '{')) {
      nsAtOpen.set(after, q.name.split('.'));
      return after;
    }
    if (ctx.isP(after, ';') && lang === 'csharp' && scope.kind === 'file') {
      res.fileNamespace = q.name.split('.');
      return after + 1;
    }
    return -1;
  }

  return -1;
}

function newClassRecord(ctx, fields) {
  return Object.assign({
    name: null,
    kind: 'class',
    stereotype: 'class',
    modifiers: [],
    visibility: null,
    outer: null,
    namespace: [],
    line: 1,
    col: 1,
    endLine: 1,
    endCol: 1,
    superTypes: [],
    fields: [],
    methods: [],
    constants: [],
    cppDefault: null
  }, fields);
}

function headerScan(ctx, start) {
  // From the first token after the declared name, find the body '{' or the end of a bodiless declaration.
  const N = ctx.tokens.length;
  const nlLang = newlineTerminated(ctx.lang);
  let angle = 0;
  const limit = Math.min(N, start + 2000);
  for (let k = start; k < limit; k++) {
    const t = ctx.tokens[k];
    if (t.t === 'p') {
      if (t.v === '{' && angle === 0) return { bodyOpen: k, end: k };
      if ((t.v === '(' || t.v === '[') && ctx.match[k] > k) { k = ctx.match[k]; continue; }
      if (t.v === '<') { angle++; continue; }
      if (t.v === '>') { if (angle > 0) angle--; continue; }
      if (t.v === ';' || t.v === '}' || (t.v === '=' && angle === 0)) return { bodyOpen: -1, end: k };
      if (t.v === '{') return { bodyOpen: k, end: k };
      continue;
    }
    if (t.t === 'nl' && nlLang && angle === 0) {
      const prev = ctx.tok(ctx.prevSig(k));
      const next = ctx.tok(ctx.nextSig(k));
      const prevContinues = prev && prev.t === 'p' && CONTINUATION_TAIL.has(prev.v);
      const nextContinues = next && ((next.t === 'p' && (CONTINUATION_HEAD.has(next.v) || next.v === '(')) ||
        (next.t === 'id' && CONTINUATION_HEAD_WORDS.has(next.v)));
      if (!prevContinues && !nextContinues) return { bodyOpen: -1, end: k };
    }
  }
  return { bodyOpen: -1, end: limit };
}

function tryClassHeader(ctx, i, scopes) {
  const lang = ctx.lang;
  const t = ctx.tokens[i];
  const keywords = CLASS_KEYWORDS[lang];
  if (!keywords || !keywords.has(t.v)) return null;
  const prevIdx = ctx.prevSig(i);
  const prev = ctx.tok(prevIdx);
  if (prev && prev.t === 'p' && (prev.v === '.' || prev.v === '::' || prev.v === '?.')) return null;
  const nextIdx = ctx.nextSig(i);
  const next = ctx.tok(nextIdx);
  if (!next) return null;

  if (lang === 'csharp' && t.v === 'record' && next.t === 'id' && (next.v === 'class' || next.v === 'struct')) return null;
  if (lang === 'cpp' && t.v === 'enum' && next.t === 'id' && (next.v === 'class' || next.v === 'struct')) return null;
  if (lang === 'java' && t.v === 'record') {
    const after = ctx.tok(ctx.nextSig(nextIdx));
    if (!(next.t === 'id' && after && after.t === 'p' && (after.v === '(' || after.v === '<'))) return null;
  }

  let prevWord = prev ? prev.v : null;
  if (prev && prev.t === 'p' && prev.v === '@') prevWord = '@';
  if (lang === 'csharp' && prevWord === 'record') prevWord = null;

  let nameIdx = nextIdx;
  let name = null;
  if (lang === 'kotlin' && t.v === 'object' && next.t === 'p' && next.v === '{' && prevWord === 'companion') {
    name = 'Companion';
    nameIdx = i;
  } else if (next.t === 'id' && !NOT_A_TYPE_NAME.has(next.v) && !MODIFIERS.has(next.v) && !FUN_KEYWORDS.has(next.v)) {
    name = next.v;
  } else {
    return null;
  }

  const enclosing = enclosingClass(scopes);
  const scope = scopes[scopes.length - 1];

  let k = nameIdx + 1;
  if (ctx.isP(k, '<')) k = ctx.skipAngles(k, ctx.tokens.length);
  if ((lang === 'scala' || lang === 'go') && ctx.isP(k, '[') && ctx.match[k] > k) k = ctx.match[k] + 1;
  let ctorOpen = -1;
  // Kotlin `class A private constructor(...)`
  while (ctx.isId(k) && (MODIFIERS.has(ctx.tokens[k].v) || ctx.tokens[k].v === 'constructor')) k++;
  if (ctx.isP(k, '(') && ctx.match[k] > k && (lang === 'kotlin' || lang === 'scala' || lang === 'java' || lang === 'csharp' || lang === 'rust')) {
    ctorOpen = k;
    k = ctx.match[k] + 1;
  }
  const heritageStart = k;
  const { bodyOpen, end } = headerScan(ctx, k);
  if (lang === 'cpp' && bodyOpen === -1) return null;
  if ((lang === 'javascript' || lang === 'typescript') && bodyOpen === -1) return null;
  if (lang === 'cpp') {
    // `struct stat buf;` and `template<class T>` never reach here (no body); require ':' or '{' after name.
    const afterName = ctx.tok(heritageStart);
    if (afterName && !(afterName.t === 'p' && (afterName.v === '{' || afterName.v === ':')) && !(afterName.t === 'id' && afterName.v === 'final')) {
      return null;
    }
  }

  const { kind: baseKind, stereotype } = kindForKeyword(t.v, prevWord, lang);
  const modifiers = collectLeadingModifiers(ctx, i);
  let kind = baseKind;
  if (kind === 'class' && (modifiers.includes('abstract') || modifiers.includes('sealed') && lang === 'kotlin')) kind = 'abstract';

  const rec = newClassRecord(ctx, {
    name,
    kind,
    stereotype: modifiers.includes('data') ? 'data class' : stereotype,
    modifiers,
    visibility: visibilityFromModifiers(modifiers),
    outer: enclosing ? enclosing.index : null,
    namespace: namespacePath(scopes),
    line: ctx.tokens[nameIdx].line,
    col: ctx.tokens[nameIdx].col,
    cppDefault: lang === 'cpp' ? (t.v === 'class' ? '-' : '+') : null
  });
  rec.superTypes = kind === 'enum' && (lang === 'csharp' || lang === 'cpp') ? [] : parseHeritage(ctx, heritageStart, bodyOpen >= 0 ? bodyOpen : end);
  if (lang === 'rust' && t.v === 'trait') {
    rec.superTypes = parseHeritage(ctx, heritageStart, bodyOpen >= 0 ? bodyOpen : end).map(s => Object.assign(s, { role: 'extends' }));
  }

  if (ctorOpen >= 0) {
    parsePrimaryConstructor(ctx, rec, ctorOpen);
  }

  if (bodyOpen >= 0) {
    const close = ctx.closeOf(bodyOpen);
    const closeTok = ctx.tok(Math.min(close, ctx.tokens.length - 1));
    rec.endLine = closeTok ? closeTok.line : rec.line;
    rec.endCol = closeTok ? closeTok.col : rec.col;
  } else {
    const endTok = ctx.tok(Math.min(end, ctx.tokens.length - 1)) || ctx.tokens[nameIdx];
    rec.endLine = endTok.line;
    rec.endCol = endTok.col;
  }
  rec.bodyOpen = bodyOpen;
  return { rec, bodyOpen, scope };
}

function splitTopLevel(ctx, s, e, separator) {
  const parts = [];
  let start = s;
  let angle = 0;
  for (let k = s; k < e; k++) {
    const t = ctx.tokens[k];
    if (t.t !== 'p') continue;
    if ((t.v === '(' || t.v === '[' || t.v === '{') && ctx.match[k] > k) { k = Math.min(ctx.match[k], e - 1); continue; }
    if (t.v === '<') angle++;
    else if (t.v === '>' && angle > 0) angle--;
    else if (t.v === separator && angle === 0) {
      parts.push([start, k]);
      start = k + 1;
    }
  }
  if (start < e) parts.push([start, e]);
  return parts;
}

function skipAnnotations(ctx, s, e) {
  let k = s;
  while (k < e) {
    const t = ctx.tokens[k];
    if (t.t === 'nl') { k++; continue; }
    if (t.t === 'p' && t.v === '@' && ctx.isId(k + 1)) {
      k += 2;
      while ((ctx.isP(k, '.') || ctx.isP(k, ':')) && ctx.isId(k + 1)) k += 2;
      if (ctx.isP(k, '(') && ctx.match[k] > k) k = ctx.match[k] + 1;
      continue;
    }
    if (t.t === 'p' && t.v === '[' && ctx.lang === 'csharp' && ctx.match[k] > k) { k = ctx.match[k] + 1; continue; }
    if (t.t === 'p' && t.v === '#' && ctx.lang === 'rust' && ctx.isP(k + 1, '[') && ctx.match[k + 1] > k) { k = ctx.match[k + 1] + 1; continue; }
    if (t.t === 'p' && t.v === '#' && ctx.lang === 'rust' && ctx.isP(k + 1, '!') && ctx.isP(k + 2, '[')) { k = ctx.closeOf(k + 2) + 1; continue; }
    break;
  }
  return k;
}

function parseParams(ctx, open) {
  const close = ctx.closeOf(open);
  const params = [];
  const lang = ctx.lang;
  const segments = splitTopLevel(ctx, open + 1, close, ',');
  const raw = [];
  for (const [s0, e0] of segments) {
    let s = skipAnnotations(ctx, s0, e0);
    const mods = [];
    while (s < e0 && ctx.isId(s) && (MODIFIERS.has(ctx.tokens[s].v) || ['val', 'var', 'vararg', 'ref', 'out', 'in', 'params', 'this', 'mut', 'let', 'inout', 'borrowing', 'consuming'].includes(ctx.tokens[s].v))) {
      mods.push(ctx.tokens[s].v);
      s++;
    }
    while (s < e0 && ctx.tokens[s].t === 'nl') s++;
    if (s >= e0) continue;
    // Strip default value.
    let e = e0;
    let defaultIdx = -1;
    for (let k = s; k < e0; k++) {
      const tk = ctx.tokens[k];
      if (tk.t === 'p' && tk.v === '=' ) { defaultIdx = k; e = k; break; }
      if (tk.t === 'p' && (tk.v === '(' || tk.v === '[' || tk.v === '{') && ctx.match[k] > k) k = ctx.match[k];
    }
    let colon = -1;
    let angle = 0;
    for (let k = s; k < e; k++) {
      const tk = ctx.tokens[k];
      if (tk.t !== 'p') continue;
      if (tk.v === '<') angle++;
      else if (tk.v === '>' && angle > 0) angle--;
      else if (tk.v === ':' && angle === 0) { colon = k; break; }
      else if ((tk.v === '(' || tk.v === '[') && ctx.match[k] > k) k = ctx.match[k];
    }
    raw.push({ s, e, colon, mods, hasDefault: defaultIdx >= 0 });
  }

  if (lang === 'go') {
    // Go groups names before a shared type: (a, b int, c string); unnamed params are types only.
    const allSingle = raw.every(r => r.e - r.s === 1 || (r.e - r.s === 2 && ctx.isP(r.s, '*')));
    let pendingNames = [];
    for (const r of raw) {
      if (allSingle) {
        params.push({ name: null, type: ctx.typeText(r.s, r.e) });
        continue;
      }
      if (r.e - r.s === 1 && ctx.isId(r.s)) {
        pendingNames.push(ctx.tokens[r.s].v);
        continue;
      }
      const name = ctx.isId(r.s) ? ctx.tokens[r.s].v : null;
      const type = ctx.typeText(r.s + (name ? 1 : 0), r.e);
      pendingNames.forEach(n => params.push({ name: n, type }));
      pendingNames = [];
      params.push({ name, type });
    }
    pendingNames.forEach(n => params.push({ name: n, type: null }));
    return { params, close, raw: [] };
  }

  for (const r of raw) {
    if (r.colon >= 0) {
      let nameIdx = r.colon - 1;
      while (nameIdx >= r.s && !ctx.isId(nameIdx)) nameIdx--;
      const name = nameIdx >= r.s ? ctx.tokens[nameIdx].v : null;
      params.push({ name, type: ctx.typeText(r.colon + 1, r.e), modifiers: r.mods, refs: ctx.typeRefs(r.colon + 1, r.e) });
      continue;
    }
    // Prefix style: Type name — the name is the last identifier.
    let nameIdx = r.e - 1;
    while (nameIdx >= r.s && ctx.tokens[nameIdx].t === 'nl') nameIdx--;
    if (nameIdx < r.s) continue;
    const last = ctx.tokens[nameIdx];
    if (r.e - r.s === 1 || (last.t !== 'id')) {
      if (lang === 'cpp' || lang === 'csharp' || lang === 'java') {
        params.push({ name: null, type: ctx.typeText(r.s, r.e), modifiers: r.mods });
      } else {
        const name = last.t === 'id' ? last.v : ctx.typeText(r.s, r.e);
        params.push({ name, type: null, modifiers: r.mods });
      }
      continue;
    }
    if (lang === 'javascript' || lang === 'typescript' || lang === 'python') {
      const name = ctx.typeText(r.s, r.e);
      params.push({ name, type: null, modifiers: r.mods });
      continue;
    }
    params.push({ name: last.v, type: ctx.typeText(r.s, nameIdx), modifiers: r.mods, refs: ctx.typeRefs(r.s, nameIdx) });
  }
  return { params, close, raw };
}

function parsePrimaryConstructor(ctx, rec, open) {
  const lang = ctx.lang;
  const tok = ctx.tokens[open];
  if (lang === 'rust') {
    // Tuple struct: fields are positional (0, 1, ...).
    const segments = splitTopLevel(ctx, open + 1, ctx.closeOf(open), ',');
    segments.forEach(([s0, e0], index) => {
      let s = skipAnnotations(ctx, s0, e0);
      let vis = '-';
      if (ctx.isId(s, 'pub')) {
        vis = '+';
        s++;
        if (ctx.isP(s, '(') && ctx.match[s] > s) s = ctx.match[s] + 1;
      }
      if (s >= e0) return;
      rec.fields.push(makeField(ctx, rec, String(index), ctx.typeText(s, e0), vis, {
        line: ctx.tokens[s].line,
        col: ctx.tokens[s].col,
        type_refs: ctx.typeRefs(s, e0),
        origin: 'tuple-field'
      }));
    });
    return;
  }
  const { params } = parseParams(ctx, open);
  for (const p of params) {
    const mods = p.modifiers || [];
    const declares = lang === 'java' || lang === 'csharp' || lang === 'rust' ||
      mods.includes('val') || mods.includes('var') || (lang === 'scala' && rec.modifiers.includes('case'));
    if (!declares || !p.name) continue;
    const vis = visibilityFromModifiers(mods) || (lang === 'java' ? '-' : '+');
    rec.fields.push(makeField(ctx, rec, p.name, p.type, vis, {
      is_final: lang === 'java' || mods.includes('val') || lang === 'scala',
      line: tok.line,
      col: tok.col,
      type_refs: p.refs || [],
      origin: 'primary-constructor'
    }));
  }
}

function makeField(ctx, rec, name, type, visibility, extra = {}) {
  return {
    name,
    type: type || null,
    visibility: visibility || defaultMemberVisibility(ctx.lang, rec, name),
    is_static: Boolean(extra.is_static),
    is_final: Boolean(extra.is_final),
    is_collection: isCollectionType(type),
    default_value: extra.default_value || null,
    line: extra.line || null,
    col: extra.col || null,
    type_refs: extra.type_refs || [],
    origin: extra.origin || 'declaration'
  };
}

function makeMethod(ctx, rec, data) {
  return {
    name: data.name,
    return_type: data.return_type === undefined ? null : data.return_type,
    visibility: data.visibility || defaultMemberVisibility(ctx.lang, rec, data.name),
    is_static: Boolean(data.is_static),
    is_abstract: Boolean(data.is_abstract),
    is_async: Boolean(data.is_async),
    is_constructor: Boolean(data.is_constructor),
    parameters: data.parameters || [],
    line: data.line || null,
    col: data.col || null
  };
}

function analyzeMember(ctx, rec, a, b, terminator) {
  const lang = ctx.lang;
  let s = skipAnnotations(ctx, a, b);
  // C++ access labels: `public:`
  while (lang === 'cpp' && ctx.isId(s) && ['public', 'private', 'protected'].includes(ctx.tokens[s].v) && ctx.isP(ctx.nextSig(s), ':')) {
    rec.cppDefault = VISIBILITY_WORDS[ctx.tokens[s].v];
    s = ctx.nextSig(ctx.nextSig(s)) ;
    s = skipAnnotations(ctx, s, b);
  }
  const mods = [];
  let declKeyword = null;
  let funKeyword = null;
  let accessor = null;
  while (s < b) {
    const t = ctx.tokens[s];
    if (t.t === 'nl') { s++; continue; }
    if (t.t !== 'id') break;
    if (lang === 'cpp' && t.v === 'template' && ctx.isP(s + 1, '<')) { s = ctx.skipAngles(s + 1, b); continue; }
    if (lang === 'csharp' && t.v === 'new' && ctx.isId(s + 1)) { mods.push('new'); s++; continue; }
    if (MODIFIERS.has(t.v)) {
      mods.push(t.v);
      s++;
      if ((t.v === 'pub' || t.v === 'private' || t.v === 'protected' || t.v === 'internal') && ctx.isP(s, '(') && ctx.match[s] > s && lang !== 'java') {
        s = ctx.match[s] + 1; // pub(crate), private(set)
      }
      if (t.v === 'private' && ctx.isP(s, '[') && ctx.match[s] > s) s = ctx.match[s] + 1; // scala private[pkg]
      continue;
    }
    if (DECL_KEYWORDS.has(t.v)) { declKeyword = t.v; s++; break; }
    if (FUN_KEYWORDS.has(t.v)) { funKeyword = t.v; s++; break; }
    if ((lang === 'typescript' || lang === 'javascript') && (t.v === 'get' || t.v === 'set') && ctx.isId(s + 1)) {
      accessor = t.v; s++; break;
    }
    if (lang === 'kotlin' && (t.v === 'constructor' || t.v === 'init')) break;
    if (lang === 'swift' && (t.v === 'init' || t.v === 'deinit' || t.v === 'subscript' || t.v === 'case')) break;
    break;
  }
  while (s < b && ctx.tokens[s].t === 'nl') s++;
  if (s >= b) return;
  const first = ctx.tokens[s];
  // Nested type declarations are handled by the scope walker; directives are not members.
  if (first.t === 'id' && !declKeyword && !funKeyword && !accessor) {
    const keywords = CLASS_KEYWORDS[lang];
    if ((keywords && keywords.has(first.v)) || NESTED_DECLARATION_WORDS.has(first.v) ||
        (first.v === 'type' && (lang === 'rust' || lang === 'scala' || lang === 'go'))) {
      return;
    }
    if (lang === 'kotlin' && (first.v === 'get' || first.v === 'set') && ctx.isP(s + 1, '(')) return;
    if (lang === 'swift' && first.v === 'case' && rec.kind === 'enum') {
      splitTopLevel(ctx, s + 1, b, ',').forEach(([cs]) => {
        let k = cs;
        while (k < b && ctx.tokens[k].t === 'nl') k++;
        if (ctx.isId(k)) rec.constants.push({ name: ctx.tokens[k].v, line: ctx.tokens[k].line, col: ctx.tokens[k].col });
      });
      return;
    }
  }
  const visibility = visibilityFromModifiers(mods);
  const isStatic = mods.includes('static') || mods.includes('companion') || (lang === 'rust' && mods.includes('const'));

  // Find first top-level '(' and '=' / '=>' in the statement.
  let parenIdx = -1;
  let equalsIdx = -1;
  let colonIdx = -1;
  let angle = 0;
  for (let k = s; k < b; k++) {
    const t = ctx.tokens[k];
    if (t.t !== 'p') continue;
    if (t.v === '<') { angle++; continue; }
    if (t.v === '>' && angle > 0) { angle--; continue; }
    if (angle > 0) {
      if ((t.v === '(' || t.v === '[') && ctx.match[k] > k) k = ctx.match[k];
      continue;
    }
    if (t.v === '(' && parenIdx === -1 && equalsIdx === -1) parenIdx = k;
    if ((t.v === '=' || t.v === '=>') && equalsIdx === -1) equalsIdx = k;
    if (t.v === ':' && colonIdx === -1 && parenIdx === -1 && equalsIdx === -1) colonIdx = k;
    if ((t.v === '(' || t.v === '[' || t.v === '{') && ctx.match[k] > k) k = ctx.match[k];
  }

  const isMethodShape = parenIdx !== -1 && (equalsIdx === -1 || parenIdx < equalsIdx);

  if (funKeyword || accessor || isMethodShape) {
    if (parenIdx === -1) return;
    let nameIdx = ctx.prevSig(parenIdx);
    // Skip generic parameters right before '(' (TS/Java style `foo<T>(`)
    if (ctx.isP(nameIdx, '>')) {
      let depth = 0;
      let k = nameIdx;
      for (; k >= s; k--) {
        const tk = ctx.tokens[k];
        if (tk.t === 'p' && tk.v === '>') depth++;
        else if (tk.t === 'p' && tk.v === '<') { depth--; if (depth === 0) break; }
      }
      nameIdx = ctx.prevSig(k);
    }
    if (nameIdx < s || !ctx.isId(nameIdx)) return;
    let name = ctx.tokens[nameIdx].v;
    if (CONTROL_WORDS.has(name) && !(name === 'init' || name === 'constructor')) return;
    const beforeName = ctx.tok(ctx.prevSig(nameIdx));
    const isDestructor = (lang === 'cpp' || lang === 'csharp') && beforeName && beforeName.t === 'p' && beforeName.v === '~';
    if (beforeName && nameIdx > s && beforeName.t === 'p' && (beforeName.v === '.' || beforeName.v === '=' || beforeName.v === '->') && !funKeyword) return;
    if (isDestructor) name = `~${name}`;

    const isConstructor = !isDestructor && (name === rec.name || name === 'constructor' || name === 'init' || name === '__init__');
    // Prefix return type for C-family languages.
    let returnType = null;
    let typeStart = s;
    if (funKeyword && ctx.isP(typeStart, '<')) typeStart = ctx.skipAngles(typeStart, nameIdx);
    if (!funKeyword && !isDestructor && (lang === 'java' || lang === 'csharp' || lang === 'cpp')) {
      if (ctx.isP(typeStart, '<')) typeStart = ctx.skipAngles(typeStart, nameIdx);
      if (typeStart < nameIdx) {
        returnType = ctx.typeText(typeStart, nameIdx);
      } else if (!isConstructor) {
        // An identifier followed by '(' with no declared type is a call or enum constant, not a member.
        return;
      }
    }
    const { params, close } = parseParams(ctx, parenIdx);
    // Postfix return type.
    let k = close + 1;
    while (k < b && ctx.tokens[k].t === 'nl') k++;
    if (lang === 'go') {
      if (k < b && !ctx.isP(k, '{')) {
        let endType = b;
        returnType = ctx.typeText(k, endType);
      }
    } else if (ctx.isP(k, ':') || ctx.isP(k, '->')) {
      let endType = b;
      for (let m = k + 1; m < b; m++) {
        const tm = ctx.tokens[m];
        if (tm.t === 'p' && (tm.v === '=' || tm.v === '{' || tm.v === '=>')) { endType = m; break; }
        if (tm.t === 'id' && (tm.v === 'where' || tm.v === 'throws')) { endType = m; break; }
        if (tm.t === 'p' && tm.v === '<') { m = ctx.skipAngles(m, b) - 1; continue; }
      }
      returnType = ctx.typeText(k + 1, endType);
    }
    if (ctx.lang === 'kotlin' && isConstructor && name === 'init') return; // init block, not a member

    const bodyless = terminator === ';' || terminator === 'nl' || terminator === 'end';
    // C++ pure virtual: `virtual void f() = 0;`
    let pureVirtual = false;
    if (lang === 'cpp') {
      for (let m = close + 1; m < b - 1; m++) {
        if (ctx.isP(m, '=') && ctx.tok(m + 1) && ctx.tokens[m + 1].t === 'num' && ctx.tokens[m + 1].v === '0') { pureVirtual = true; break; }
      }
    }
    const isAbstract = mods.includes('abstract') || pureVirtual ||
      (rec.kind === 'interface' && bodyless && !mods.includes('default') && !mods.includes('static'));
    const declaredParams = params
      .filter(p => !(lang === 'rust' && p.name === 'self'))
      .map(p => ({ name: p.name, type: p.type }));
    rec.methods.push(makeMethod(ctx, rec, {
      name,
      return_type: isConstructor || isDestructor ? null : returnType,
      visibility,
      is_static: isStatic || (lang === 'rust' && !params.some(p => p.name === 'self')),
      is_abstract: isAbstract,
      is_async: mods.includes('async') || mods.includes('suspend'),
      is_constructor: isConstructor,
      parameters: declaredParams,
      line: ctx.tokens[nameIdx].line,
      col: ctx.tokens[nameIdx].col
    }));
    // TypeScript constructor parameter properties declare fields.
    if ((lang === 'typescript') && name === 'constructor') {
      params.forEach(p => {
        const pm = p.modifiers || [];
        if (pm.some(m => m === 'public' || m === 'private' || m === 'protected' || m === 'readonly') && p.name) {
          rec.fields.push(makeField(ctx, rec, p.name, p.type, visibilityFromModifiers(pm) || '+', {
            is_final: pm.includes('readonly'),
            line: ctx.tokens[nameIdx].line,
            col: ctx.tokens[nameIdx].col,
            origin: 'constructor-parameter'
          }));
        }
      });
    }
    return;
  }

  // ── Fields ──
  const defaultValue = equalsIdx >= 0 ? ctx.text(ctx.tokens[equalsIdx].e, ctx.tokens[b - 1] ? ctx.tokens[b - 1].e : ctx.tokens[equalsIdx].e).trim().slice(0, 60) || null : null;
  const declEnd = equalsIdx >= 0 ? equalsIdx : b;
  const isFinal = mods.includes('final') || mods.includes('readonly') || mods.includes('const') || declKeyword === 'val' || declKeyword === 'let' && lang === 'swift';

  if (lang === 'go') {
    if (rec.kind === 'interface') {
      // Embedded interface (io.Reader / Reader)
      if (first.t === 'id') {
        const q = readQualified(ctx, s, ['.']);
        if (q.end >= declEnd - 1) rec.superTypes.push({ name: q.name, isCall: false, role: 'extends', line: first.line });
      }
      return;
    }
    // Struct tags (`json:"name"`) are string literals after the type; they are not part of it.
    let goEnd = declEnd;
    while (goEnd > s && (ctx.tokens[goEnd - 1].t === 'str' || ctx.tokens[goEnd - 1].t === 'nl')) goEnd--;
    const segments = splitTopLevel(ctx, s, goEnd, ',');
    const lastSeg = segments[segments.length - 1];
    if (!lastSeg) return;
    const lastLen = lastSeg[1] - lastSeg[0];
    if (segments.length === 1 && (lastLen === 1 || (ctx.isP(lastSeg[0], '*') && lastLen === 2) || (lastLen === 3 && ctx.isP(lastSeg[0] + 1, '.')))) {
      // Embedded field
      let k = lastSeg[0];
      if (ctx.isP(k, '*')) k++;
      if (ctx.isId(k)) {
        const q = readQualified(ctx, k, ['.']);
        rec.superTypes.push({ name: q.name, isCall: false, role: 'embeds', line: first.line });
      }
      return;
    }
    const typeStartIdx = lastSeg[0] + 1;
    const type = ctx.typeText(typeStartIdx, lastSeg[1]);
    const refs = ctx.typeRefs(typeStartIdx, lastSeg[1]);
    segments.forEach(([ss]) => {
      if (ctx.isId(ss)) {
        rec.fields.push(makeField(ctx, rec, ctx.tokens[ss].v, type, null, { line: ctx.tokens[ss].line, col: ctx.tokens[ss].col, type_refs: refs }));
      }
    });
    return;
  }

  if (declKeyword || colonIdx !== -1 || lang === 'rust' || lang === 'swift' || lang === 'scala' || lang === 'kotlin') {
    // name [: Type] [= init]
    if (!ctx.isId(s)) return;
    if (!declKeyword && lang === 'kotlin') return; // Kotlin members always use val/var/fun
    const nameTok = ctx.tokens[s];
    if (CONTROL_WORDS.has(nameTok.v)) return;
    let type = null;
    let refs = [];
    if (colonIdx !== -1 && colonIdx < declEnd) {
      let typeEnd = declEnd;
      for (let m = colonIdx + 1; m < declEnd; m++) {
        const tm = ctx.tokens[m];
        if ((tm.t === 'id' && tm.v === 'by' && (lang === 'kotlin' || lang === 'scala')) || (tm.t === 'p' && tm.v === '{')) { typeEnd = m; break; }
        if (tm.t === 'p' && tm.v === '<') { m = ctx.skipAngles(m, declEnd) - 1; continue; }
        if (tm.t === 'p' && (tm.v === '(' || tm.v === '[') && ctx.match[m] > m) m = ctx.match[m];
      }
      type = ctx.typeText(colonIdx + 1, typeEnd);
      refs = ctx.typeRefs(colonIdx + 1, typeEnd);
    } else if (lang === 'typescript' || lang === 'javascript' || lang === 'python') {
      if (s + 1 < declEnd && !ctx.isP(s + 1, '?') && !ctx.isP(s + 1, '!')) return;
    } else if (!declKeyword) {
      return;
    }
    if (lang === 'swift' && terminator === '{' && !type) return;
    rec.fields.push(makeField(ctx, rec, nameTok.v, type, visibility, {
      is_static: isStatic,
      is_final: isFinal,
      default_value: defaultValue,
      line: nameTok.line,
      col: nameTok.col,
      type_refs: refs
    }));
    return;
  }

  if (lang === 'typescript' || lang === 'javascript') {
    // `name;` or `name = value` or `name?: T` handled above (colon); here no colon.
    if (!ctx.isId(s)) return;
    const nameTok = ctx.tokens[s];
    if (declEnd - s > 1 && !ctx.isP(s + 1, '!') && !ctx.isP(s + 1, '?')) return;
    rec.fields.push(makeField(ctx, rec, nameTok.v, null, visibility, {
      is_static: isStatic,
      is_final: isFinal,
      default_value: defaultValue,
      line: nameTok.line,
      col: nameTok.col
    }));
    return;
  }

  if (terminator === '{' && lang !== 'csharp') return; // initializer blocks and the like
  // Prefix style: Type name [= v] [, name2 [= v]]
  const segments = splitTopLevel(ctx, s, b, ',');
  if (segments.length === 0) return;
  const [fs, fe] = segments[0];
  let firstEnd = fe;
  for (let k = fs; k < fe; k++) {
    const tk = ctx.tokens[k];
    if (tk.t === 'p' && (tk.v === '=' || tk.v === '=>')) { firstEnd = k; break; }
    if (tk.t === 'p' && tk.v === '<') { k = ctx.skipAngles(k, fe) - 1; continue; }
    if (tk.t === 'p' && (tk.v === '(' || tk.v === '[') && ctx.match[k] > k) {
      // C/C++ array declarators `int buf[16];` keep the name; function-pointer fields are skipped.
      if (tk.v === '(') return;
      firstEnd = k;
      break;
    }
  }
  let nameIdx = firstEnd - 1;
  while (nameIdx >= fs && ctx.tokens[nameIdx].t === 'nl') nameIdx--;
  if (nameIdx <= fs || !ctx.isId(nameIdx)) return;
  const typeEnd = nameIdx;
  let typeStart = fs;
  while (typeStart < typeEnd && ctx.isId(typeStart) && MODIFIERS.has(ctx.tokens[typeStart].v)) typeStart++;
  if (typeStart >= typeEnd) return;
  const type = ctx.typeText(typeStart, typeEnd);
  const refs = ctx.typeRefs(typeStart, typeEnd);
  const nameTok = ctx.tokens[nameIdx];
  if (CONTROL_WORDS.has(nameTok.v) || nameTok.v === 'operator') return;
  const isInterfaceConstant = rec.kind === 'interface' && lang === 'java';
  rec.fields.push(makeField(ctx, rec, nameTok.v, type, visibility || (isInterfaceConstant ? '+' : null), {
    is_static: isStatic || isInterfaceConstant,
    is_final: isFinal || isInterfaceConstant,
    default_value: firstEnd < fe && ctx.isP(firstEnd, '=') ? ctx.text(ctx.tokens[firstEnd].e, ctx.tokens[fe - 1].e).trim().slice(0, 60) || null : null,
    line: nameTok.line,
    col: nameTok.col,
    type_refs: refs
  }));
  for (let si = 1; si < segments.length; si++) {
    const [ss, se] = segments[si];
    let k = ss;
    while (k < se && ctx.tokens[k].t === 'nl') k++;
    if (ctx.isId(k) && !(se - k > 1 && !ctx.isP(k + 1, '=') && !ctx.isP(k + 1, '['))) {
      const nt = ctx.tokens[k];
      rec.fields.push(makeField(ctx, rec, nt.v, type, visibility, {
        is_static: isStatic,
        is_final: isFinal,
        line: nt.line,
        col: nt.col,
        type_refs: refs
      }));
    }
  }
}

function parseEnumConstants(ctx, rec, open, close) {
  const lang = ctx.lang;
  // Java/Kotlin enums: constants run until the first top-level ';'. Others: the whole body.
  let end = close;
  if (lang === 'java' || lang === 'kotlin') {
    for (let k = open + 1; k < close; k++) {
      const t = ctx.tokens[k];
      if (t.t === 'p' && t.v === ';') { end = k; break; }
      if (t.t === 'p' && (t.v === '(' || t.v === '[' || t.v === '{') && ctx.match[k] > k) k = ctx.match[k];
      if (t.t === 'id' && (t.v === 'fun' || t.v === 'val' || t.v === 'var' || t.v === 'override' || t.v === 'abstract') && lang === 'kotlin') {
        end = k - 1;
        break;
      }
    }
  }
  if (lang === 'swift') {
    // Swift enum cases are declared with `case a, b` statements; members follow normally.
    return open + 1;
  }
  const segments = splitTopLevel(ctx, open + 1, end, ',');
  for (const [s0, e0] of segments) {
    const s = skipAnnotations(ctx, s0, e0);
    let k = s;
    while (k < e0 && ctx.tokens[k].t === 'nl') k++;
    if (k < e0 && ctx.isId(k)) {
      const t = ctx.tokens[k];
      rec.constants.push({ name: t.v, line: t.line, col: t.col });
    }
  }
  return (lang === 'java' || lang === 'kotlin') ? Math.min(close, end + 1) : close;
}

function scanBody(ctx, rec, open, close, classAtOpen, extensionAtOpen) {
  const lang = ctx.lang;
  const nlLang = newlineTerminated(lang);
  let i = open + 1;
  if (rec.kind === 'enum') {
    i = parseEnumConstants(ctx, rec, open, close);
  }
  let stmtStart = i;
  let sawEquals = false;
  let angle = 0;
  const commaTerminates = (lang === 'rust' || (lang === 'swift' && false));

  const finish = (end, terminator) => {
    if (end > stmtStart) analyzeMember(ctx, rec, stmtStart, end, terminator);
  };

  while (i < close) {
    const t = ctx.tokens[i];
    if (t.t === 'p') {
      if (t.v === '{') {
        const blockClose = ctx.match[i] >= 0 ? Math.min(ctx.match[i], close) : close;
        if (classAtOpen.has(i) || extensionAtOpen.has(i)) {
          i = blockClose + 1;
          stmtStart = i;
          sawEquals = false;
          angle = 0;
          continue;
        }
        if (!sawEquals) {
          finish(i, '{');
          i = blockClose + 1;
          stmtStart = i;
          angle = 0;
          continue;
        }
        i = blockClose + 1;
        continue;
      }
      if ((t.v === '(' || t.v === '[') && ctx.match[i] > i) {
        i = ctx.match[i] + 1;
        continue;
      }
      if (t.v === '<') angle++;
      else if (t.v === '>' && angle > 0) angle--;
      if (t.v === ';' || (t.v === ',' && commaTerminates && angle === 0)) {
        finish(i, ';');
        i++;
        stmtStart = i;
        sawEquals = false;
        angle = 0;
        continue;
      }
      if (t.v === '=' || t.v === '=>') sawEquals = true;
      i++;
      continue;
    }
    if (t.t === 'nl' && nlLang && angle === 0 && i > stmtStart) {
      const prev = ctx.tok(ctx.prevSig(i));
      const next = ctx.tok(ctx.nextSig(i));
      const prevContinues = prev && prev.t === 'p' && CONTINUATION_TAIL.has(prev.v);
      const nextContinues = next && ((next.t === 'p' && CONTINUATION_HEAD.has(next.v)) ||
        (next.t === 'id' && CONTINUATION_HEAD_WORDS.has(next.v)));
      if (!prevContinues && !nextContinues) {
        finish(i, 'nl');
        stmtStart = i + 1;
        sawEquals = false;
      }
    }
    i++;
  }
  finish(close, 'end');
}

function parseBraceLanguage(ctx) {
  const { tokens, lang } = ctx;
  const res = ctx.result;
  const N = tokens.length;
  const scopes = [{ kind: 'file', close: N, ns: [] }];
  const classAtOpen = new Map();
  const nsAtOpen = new Map();
  const extensionAtOpen = new Map();
  let goTypeGroupClose = -1;

  for (let i = 0; i < N; i++) {
    const t = tokens[i];
    if (t.t === 'p') {
      if (t.v === '{') {
        const close = ctx.closeOf(i);
        if (classAtOpen.has(i)) scopes.push({ kind: 'class', close, cls: classAtOpen.get(i) });
        else if (extensionAtOpen.has(i)) scopes.push({ kind: 'extension', close, ext: extensionAtOpen.get(i) });
        else if (nsAtOpen.has(i)) scopes.push({ kind: 'namespace', close, ns: nsAtOpen.get(i) });
        else scopes.push({ kind: 'block', close });
      } else if (t.v === '}') {
        if (scopes.length > 1 && scopes[scopes.length - 1].close === i) scopes.pop();
      }
      continue;
    }
    if (t.t !== 'id') continue;
    const scope = scopes[scopes.length - 1];
    if (scope.kind === 'block') continue;

    if (scope.kind === 'file' || scope.kind === 'namespace') {
      const consumed = tryDirective(ctx, i, scope, nsAtOpen);
      if (consumed > i) {
        i = consumed - 1;
        continue;
      }
    }

    // Go: type Name struct/interface { ... }, including grouped `type ( ... )`.
    if (lang === 'go' && scope.kind === 'file') {
      if (t.v === 'type' && ctx.isP(ctx.nextSig(i), '(')) {
        goTypeGroupClose = ctx.closeOf(ctx.nextSig(i));
        continue;
      }
      const prevTok = ctx.tok(ctx.prevSig(i));
      const inGroup = i < goTypeGroupClose;
      const declaredHere = (prevTok && prevTok.t === 'id' && prevTok.v === 'type') ||
        (inGroup && prevTok && ((prevTok.t === 'p' && (prevTok.v === '(' || prevTok.v === ';' || prevTok.v === '}')) || (i > 0 && tokens[i - 1].t === 'nl')));
      if (declaredHere && t.v !== 'type') {
        let k = ctx.nextSig(i);
        if (ctx.isP(k, '[') && ctx.match[k] > k) k = ctx.nextSig(ctx.match[k]);
        const kw = ctx.tok(k);
        if (kw && kw.t === 'id' && (kw.v === 'struct' || kw.v === 'interface') && ctx.isP(ctx.nextSig(k), '{')) {
          const bodyOpen = ctx.nextSig(k);
          const close = ctx.closeOf(bodyOpen);
          const closeTok = ctx.tok(Math.min(close, N - 1));
          const rec = newClassRecord(ctx, {
            name: t.v,
            kind: kw.v === 'interface' ? 'interface' : 'class',
            stereotype: kw.v,
            line: t.line,
            col: t.col,
            endLine: closeTok ? closeTok.line : t.line,
            endCol: closeTok ? closeTok.col : t.col,
            visibility: /^[A-Z]/.test(t.v) ? '+' : '-'
          });
          rec.index = res.classes.length;
          rec.bodyOpen = bodyOpen;
          res.classes.push(rec);
          classAtOpen.set(bodyOpen, rec);
          i = k;
          continue;
        }
      }
      // Go method with receiver: func (r *T) Name(params) results {
      if (t.v === 'func' && ctx.isP(ctx.nextSig(i), '(')) {
        const recvOpen = ctx.nextSig(i);
        const recvClose = ctx.closeOf(recvOpen);
        let target = null;
        for (let k = recvOpen + 1; k < recvClose; k++) {
          const tk = tokens[k];
          if (tk.t === 'p' && tk.v === '[') break;
          if (tk.t === 'id') target = tk.v;
        }
        const nameIdx = ctx.nextSig(recvClose);
        if (target && ctx.isId(nameIdx) && ctx.isP(ctx.nextSig(nameIdx), '(')) {
          const ext = { targetName: target, kind: 'receiver', superTypes: [], fields: [], methods: [], line: tokens[nameIdx].line, col: tokens[nameIdx].col, name: target };
          let end = ctx.nextSig(nameIdx);
          const paramsClose = ctx.closeOf(end);
          let bodyIdx = -1;
          for (let k = paramsClose + 1; k < Math.min(N, paramsClose + 200); k++) {
            if (tokens[k].t === 'p' && tokens[k].v === '{') { bodyIdx = k; break; }
            if (tokens[k].t === 'p' && (tokens[k].v === '(' || tokens[k].v === '[') && ctx.match[k] > k) k = ctx.match[k];
            if (tokens[k].t === 'nl') break;
          }
          const stmtEnd = bodyIdx >= 0 ? bodyIdx : paramsClose + 1;
          analyzeMember(ctx, ext, nameIdx, stmtEnd, bodyIdx >= 0 ? '{' : 'nl');
          // analyzeMember treats a bare name( as a call in prefix languages; Go has no prefix type so it is a method.
          res.extensions.push(ext);
          i = paramsClose;
          continue;
        }
      }
    }

    // Rust impl blocks and Swift extensions attach members to an existing type.
    if ((lang === 'rust' && t.v === 'impl') || (lang === 'swift' && t.v === 'extension')) {
      if (scope.kind !== 'file' && scope.kind !== 'namespace') continue;
      let k = ctx.nextSig(i);
      if (ctx.isP(k, '<')) k = ctx.skipAngles(k, N);
      if (ctx.isId(k, 'unsafe')) k = ctx.nextSig(k);
      if (!ctx.isId(k)) continue;
      const first = readQualified(ctx, k, ['::', '.']);
      k = first.end + 1;
      if (ctx.isP(k, '<')) k = ctx.skipAngles(k, N);
      let traitName = null;
      let targetName = first.name;
      const superTypes = [];
      if (lang === 'rust' && ctx.isId(ctx.nextSig(k - 1), 'for')) {
        const forIdx = ctx.nextSig(k - 1);
        const targetIdx = ctx.nextSig(forIdx);
        if (ctx.isId(targetIdx)) {
          traitName = first.name;
          const second = readQualified(ctx, targetIdx, ['::', '.']);
          targetName = second.name;
          k = second.end + 1;
        }
      }
      const { bodyOpen } = headerScan(ctx, k);
      if (lang === 'swift') {
        parseHeritage(ctx, k, bodyOpen >= 0 ? bodyOpen : k).forEach(s => superTypes.push(Object.assign(s, { role: 'colon' })));
      } else if (traitName) {
        superTypes.push({ name: traitName, isCall: false, role: 'implements', line: t.line });
      }
      if (bodyOpen >= 0) {
        const ext = {
          targetName: targetName.split('.').pop(),
          qualifiedTarget: targetName,
          kind: lang === 'rust' ? 'impl' : 'extension',
          name: targetName.split('.').pop(),
          superTypes,
          fields: [],
          methods: [],
          constants: [],
          line: t.line,
          col: t.col,
          bodyOpen
        };
        extensionAtOpen.set(bodyOpen, ext);
        res.extensions.push(ext);
      }
      continue;
    }

    if (scope.kind === 'extension') continue;

    const header = tryClassHeader(ctx, i, scopes);
    if (header) {
      header.rec.index = res.classes.length;
      res.classes.push(header.rec);
      if (header.bodyOpen >= 0) classAtOpen.set(header.bodyOpen, header.rec);
    }
  }

  res.classes.forEach(rec => {
    if (rec.bodyOpen >= 0) scanBody(ctx, rec, rec.bodyOpen, ctx.closeOf(rec.bodyOpen), classAtOpen, extensionAtOpen);
  });
  res.extensions.forEach(ext => {
    if (ext.bodyOpen >= 0 && ext.bodyOpen !== undefined) scanBody(ctx, ext, ext.bodyOpen, ctx.closeOf(ext.bodyOpen), classAtOpen, extensionAtOpen);
  });
}

// ── 6. Python (indentation scoped) ─────────────────────────────────────────────

function pyIndent(line) {
  let n = 0;
  for (let i = 0; i < line.length; i++) {
    const c = line.charCodeAt(i);
    if (c === 32) n++;
    else if (c === 9) n += 4 - (n % 4);
    else break;
  }
  return n;
}

function pyReadIdent(text, i) {
  let j = i;
  while (j < text.length && isIdentPart(text.charCodeAt(j))) j++;
  return j > i && isIdentStart(text.charCodeAt(i)) ? { name: text.slice(i, j), end: j } : null;
}

function pyBalanced(text) {
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    if (c === 40 || c === 91 || c === 123) depth++;
    else if (c === 41 || c === 93 || c === 125) depth--;
  }
  return depth <= 0;
}

function pySplitTopLevel(text) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') depth--;
    else if (c === ',' && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map(p => p.trim()).filter(Boolean);
}

function pyMatchParen(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const c = text[i];
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

function pyTypeRefs(typeText) {
  if (!typeText) return [];
  const refs = [];
  const re = /[A-Za-z_][A-Za-z0-9_.]*/g; // linear: a single character class repetition with no alternation
  let m;
  const outerHead = typeText.split('[')[0].trim().split('.').pop();
  const collection = COLLECTION_TYPES.has(outerHead);
  while ((m = re.exec(typeText)) !== null) {
    const name = m[0];
    const simple = name.split('.').pop();
    if (COLLECTION_TYPES.has(simple) || TRANSPARENT_WRAPPERS.has(simple) || BUILTIN_TYPE_NAMES.has(simple) || simple === 'None') continue;
    refs.push({ name, collection: collection && name !== outerHead });
  }
  return refs;
}

function parsePython(path, code) {
  const { masked } = maskSource(code, 'python');
  const maskedLines = masked.split('\n');
  const origLines = code.split('\n');
  const res = {
    path,
    language: 'python',
    packageDecl: null,
    fileNamespace: [],
    imports: [],
    wildcardImports: [],
    classes: [],
    extensions: [],
    lineCount: maskedLines.length,
    diagnostics: []
  };
  const moduleParts = path.replace(/\.py$/i, '').split('/').filter(Boolean);
  const isInit = moduleParts[moduleParts.length - 1] === '__init__';
  if (isInit) moduleParts.pop();
  const modulePackage = moduleParts.join('.');
  res.packageDecl = modulePackage || null;

  const stack = []; // { indent, type: 'class'|'def', rec?, bodyIndent }
  let decorators = [];
  let lastContentLine = 0;

  const closeScopes = (indent, lineNo) => {
    while (stack.length && indent <= stack[stack.length - 1].indent) {
      const sc = stack.pop();
      if (sc.type === 'class') {
        sc.rec.endLine = lastContentLine || sc.rec.line;
        sc.rec.endCol = 1;
      }
    }
  };

  for (let li = 0; li < maskedLines.length; li++) {
    let text = maskedLines[li];
    if (!text.trim()) continue;
    const indent = pyIndent(text);
    closeScopes(indent, li + 1);
    lastContentLine = li + 1;
    let logical = text.trim();
    const startLine = li + 1;
    // Join continuation lines for headers with open brackets or a trailing backslash.
    let guard = 0;
    while ((!pyBalanced(logical) || logical.endsWith('\\')) && li + 1 < maskedLines.length && guard < 60) {
      li++;
      guard++;
      logical = logical.replace(/\\$/, '') + ' ' + maskedLines[li].trim();
      lastContentLine = li + 1;
    }

    const top = stack[stack.length - 1];
    const inClassBody = top && top.type === 'class' && (top.bodyIndent === null || indent === top.bodyIndent);
    if (top && top.type === 'class' && top.bodyIndent === null) top.bodyIndent = indent;
    const inFunction = stack.some(s => s.type === 'def');

    if (logical.startsWith('@')) {
      const id = pyReadIdent(logical, 1);
      if (id) {
        let name = id.name;
        let e = id.end;
        while (logical[e] === '.') {
          const nx = pyReadIdent(logical, e + 1);
          if (!nx) break;
          name = nx.name;
          e = nx.end;
        }
        decorators.push(name);
      }
      continue;
    }

    if (logical.startsWith('from ') && !inFunction) {
      const importIdx = logical.indexOf(' import ');
      if (importIdx > 5) {
        let mod = logical.slice(5, importIdx).trim();
        if (mod.startsWith('.')) {
          let dots = 0;
          while (mod[dots] === '.') dots++;
          const base = (isInit ? moduleParts : moduleParts.slice(0, -1)).slice(0, Math.max(0, (isInit ? moduleParts.length : moduleParts.length - 1) - (dots - 1)));
          const rest = mod.slice(dots);
          mod = [...base, ...(rest ? rest.split('.') : [])].join('.');
        }
        let names = logical.slice(importIdx + 8).trim().replace(/^\(|\)$/g, '');
        if (names === '*') {
          res.wildcardImports.push(mod);
        } else {
          pySplitTopLevel(names).forEach(part => {
            const pieces = part.split(/\s+as\s+/);
            const original = pieces[0].trim();
            const alias = (pieces[1] || original).trim();
            if (original) res.imports.push({ alias, target: mod ? `${mod}.${original}` : original });
          });
        }
      }
      decorators = [];
      continue;
    }

    if (logical.startsWith('class ') && logical.includes(':')) {
      let nameAt = 6;
      while (logical[nameAt] === ' ') nameAt++;
      const id = pyReadIdent(logical, nameAt);
      if (!id || inFunction) { decorators = []; continue; }
      let bases = [];
      let k = id.end;
      while (logical[k] === ' ') k++;
      if (logical[k] === '[') {
        const close = pyMatchParen(logical, k);
        if (close > 0) k = close + 1;
      }
      if (logical[k] === '(') {
        const close = pyMatchParen(logical, k);
        if (close > 0) {
          bases = pySplitTopLevel(logical.slice(k + 1, close)).filter(b => !b.includes('='));
        }
      }
      const outerRec = top && top.type === 'class' ? top.rec : null;
      const baseNames = bases.map(b => b.split('[')[0].trim()).filter(Boolean);
      const simpleBases = baseNames.map(b => b.split('.').pop());
      let kind = 'class';
      let stereotype = 'class';
      if (simpleBases.some(b => ['Enum', 'IntEnum', 'StrEnum', 'Flag', 'IntFlag'].includes(b))) { kind = 'enum'; stereotype = 'enum'; }
      else if (simpleBases.includes('Protocol')) { kind = 'interface'; stereotype = 'protocol'; }
      else if (simpleBases.includes('ABC') || logical.includes('metaclass=ABCMeta')) { kind = 'abstract'; }
      if (decorators.includes('dataclass')) stereotype = 'dataclass';
      const col = (maskedLines[startLine - 1].indexOf(id.name, indent + 6) + 1) || 1;
      const rec = newClassRecord(null, {
        name: id.name,
        kind,
        stereotype,
        outer: outerRec ? outerRec.index : null,
        line: startLine,
        col,
        endLine: startLine,
        endCol: 1,
        superTypes: baseNames
          .filter(b => !['object', 'ABC', 'Protocol', 'Generic', 'Enum', 'IntEnum', 'StrEnum', 'Flag', 'IntFlag', 'NamedTuple', 'TypedDict'].includes(b.split('.').pop()))
          .map(name => ({ name, isCall: false, role: 'unknown-python', line: startLine })),
        visibility: defaultMemberVisibility('python', null, id.name),
        decorators: decorators.slice()
      });
      rec.index = res.classes.length;
      res.classes.push(rec);
      stack.push({ indent, type: 'class', rec, bodyIndent: null });
      decorators = [];
      continue;
    }

    const isAsyncDef = logical.startsWith('async def ');
    if (logical.startsWith('def ') || isAsyncDef) {
      const nameStart = isAsyncDef ? 10 : 4;
      const id = pyReadIdent(logical, nameStart);
      if (id) {
        const open = logical.indexOf('(', id.end);
        const close = open >= 0 ? pyMatchParen(logical, open) : -1;
        if (inClassBody && !inFunction && top && top.type === 'class') {
          const rec = top.rec;
          const paramsRaw = close > open ? pySplitTopLevel(logical.slice(open + 1, close)) : [];
          const isStatic = decorators.includes('staticmethod');
          const isClassMethod = decorators.includes('classmethod');
          const params = [];
          paramsRaw.forEach((p, idx) => {
            if (idx === 0 && !isStatic && (p === 'self' || p === 'cls' || p.startsWith('self:') || p.startsWith('cls:'))) return;
            if (p === '/' || p === '*') return;
            const noDefault = p.split('=')[0].trim();
            const colon = noDefault.indexOf(':');
            const pn = (colon >= 0 ? noDefault.slice(0, colon) : noDefault).trim();
            const pt = colon >= 0 ? noDefault.slice(colon + 1).trim() : '';
            params.push({ name: pn || null, type: pt || null });
          });
          let returnType = null;
          if (close > 0) {
            const after = logical.slice(close + 1);
            const arrow = after.indexOf('->');
            if (arrow >= 0) {
              const colon = after.lastIndexOf(':');
              returnType = after.slice(arrow + 2, colon > arrow ? colon : after.length).trim() || null;
            }
          }
          const col = (maskedLines[startLine - 1].indexOf(id.name) + 1) || 1;
          rec.methods.push({
            name: id.name,
            return_type: id.name === '__init__' ? null : returnType,
            visibility: defaultMemberVisibility('python', rec, id.name),
            is_static: isStatic || isClassMethod,
            is_abstract: decorators.includes('abstractmethod'),
            is_async: isAsyncDef,
            is_constructor: id.name === '__init__',
            parameters: params,
            line: startLine,
            col
          });
          if (decorators.includes('abstractmethod') && rec.kind === 'class') rec.kind = 'abstract';
          stack.push({ indent, type: 'def', cls: rec });
        } else {
          stack.push({ indent, type: 'def', cls: null });
        }
      }
      decorators = [];
      continue;
    }
    decorators = [];

    // Instance attributes: self.name = ... / self.name: T = ... inside a method of a class.
    const fnScope = [...stack].reverse().find(s => s.type === 'def');
    if (fnScope && fnScope.cls && logical.startsWith('self.')) {
      const id = pyReadIdent(logical, 5);
      if (id) {
        let rest = logical.slice(id.end).trim();
        let type = null;
        if (rest.startsWith(':')) {
          const eq = rest.indexOf('=');
          type = (eq >= 0 ? rest.slice(1, eq) : rest.slice(1)).trim() || null;
          rest = eq >= 0 ? rest.slice(eq) : '';
        }
        if (rest.startsWith('=') && !rest.startsWith('==')) {
          const rec = fnScope.cls;
          if (!rec.fields.some(f => f.name === id.name)) {
            const col = (maskedLines[startLine - 1].indexOf('self.' + id.name) + 6) || 1;
            const defaultValue = origLines[startLine - 1] ? origLines[startLine - 1].slice(origLines[startLine - 1].indexOf('=') + 1).trim().slice(0, 60) : null;
            rec.fields.push({
              name: id.name,
              type,
              visibility: defaultMemberVisibility('python', rec, id.name),
              is_static: false,
              is_final: false,
              is_collection: isCollectionType(type),
              default_value: defaultValue || null,
              line: startLine,
              col,
              type_refs: pyTypeRefs(type),
              origin: 'instance-attribute'
            });
          }
        }
      }
      continue;
    }

    // Class-level attributes.
    if (inClassBody && !inFunction && top && top.type === 'class') {
      const id = pyReadIdent(logical, 0);
      if (id && !['pass', 'return', 'if', 'for', 'while', 'with', 'try', 'raise', 'import', 'assert', 'del', 'global', 'nonlocal'].includes(id.name)) {
        const rest = logical.slice(id.end).trim();
        const rec = top.rec;
        const col = (maskedLines[startLine - 1].indexOf(id.name) + 1) || 1;
        if (rec.kind === 'enum' && rest.startsWith('=') && !rest.startsWith('==')) {
          rec.constants.push({ name: id.name, line: startLine, col });
        } else if (rest.startsWith(':') || (rest.startsWith('=') && !rest.startsWith('=='))) {
          let type = null;
          if (rest.startsWith(':')) {
            const eq = rest.indexOf('=');
            type = (eq >= 0 ? rest.slice(1, eq) : rest.slice(1)).trim() || null;
          }
          rec.fields.push({
            name: id.name,
            type,
            visibility: defaultMemberVisibility('python', rec, id.name),
            is_static: !rest.startsWith(':') || /^ClassVar\b/.test(type || ''),
            is_final: /^Final\b/.test(type || ''),
            is_collection: isCollectionType(type),
            default_value: rest.includes('=') ? (origLines[startLine - 1] || '').slice((origLines[startLine - 1] || '').indexOf('=') + 1).trim().slice(0, 60) || null : null,
            line: startLine,
            col,
            type_refs: pyTypeRefs(type),
            origin: 'class-attribute'
          });
        }
      }
    }
  }
  closeScopes(-1, maskedLines.length);
  return res;
}

// ── 7. Public entry point ──────────────────────────────────────────────────────

/**
 * Parses one source file. Returns a plain, structured-cloneable object:
 * { path, language, packageDecl, fileNamespace, imports, wildcardImports, classes, extensions, lineCount, diagnostics }
 * Throws only for unsupported languages.
 */
export function parseSource(path, code) {
  const lang = languageForPath(path);
  if (!lang) throw new Error(`Unsupported source language for ${path}`);
  const text = typeof code === 'string' ? code : '';
  if (lang === 'python') return parsePython(path, text);
  const ctx = new ParseContext(path, text, lang);
  parseBraceLanguage(ctx);
  const res = ctx.result;
  res.classes.forEach(rec => {
    delete rec.bodyOpen;
    delete rec.cppDefault;
  });
  res.extensions.forEach(ext => {
    delete ext.bodyOpen;
    delete ext.cppDefault;
  });
  return res;
}
