/**
 * OpenHeart In-Browser C-Family Declaration Extractor (C, C++, C# / .NET)
 *
 * Plugs into GitHubEngine.parseSourceFile: every parse function takes the same
 * (filePath, code, classes, packages, relations, owner, repo, branch) arguments and pushes
 * class, package and relation records in exactly the shape that engine produces, so
 * GitHubEngine.buildGraphIr consumes them unchanged. Language ids mirror the Rust engine:
 * LangId::C, LangId::Cpp and LangId::CSharp.
 *
 * Every stage is a single forward pass and no regular expression is used anywhere, so the
 * running time is linear in the size of the file:
 *   1. lex: comments, strings (C++ raw, C# verbatim / interpolated / raw), char literals,
 *      digit separators and preprocessor lines are consumed by hand-written scanners.
 *      Conditional groups keep their first live branch (an `#if 0` / `#if false` branch is
 *      dropped in favour of its `#else`), so braces stay balanced across `#ifdef` variants.
 *   2. pair (), [] and {} with a stack, and template / generic angle brackets with a
 *      depth-aware stack, so any group is skipped in O(1).
 *   3. walk scopes (file, namespace, linkage block, type body) and classify each declaration
 *      head; function bodies, initialisers and accessor bodies are jumped over, never read.
 *   4. map the declaration model onto UML class records and relations.
 */

// ── 1. Languages and extensions ────────────────────────────────────────────────

export const LANG_C = 'c';
export const LANG_CPP = 'cpp';
export const LANG_CSHARP = 'csharp';

/** Rust-side identifiers (LangId variants) for each language key. */
export const LANG_IDS = Object.freeze({ c: 'C', cpp: 'Cpp', csharp: 'CSharp' });

export const C_EXTENSIONS = Object.freeze(['.c', '.h']);
export const CPP_EXTENSIONS = Object.freeze([
  '.cpp', '.cc', '.cxx', '.c++', '.hpp', '.hh', '.hxx', '.h++', '.ipp', '.tpp', '.inl', '.ixx', '.cppm'
]);
export const CSHARP_EXTENSIONS = Object.freeze(['.cs', '.csx']);
export const C_FAMILY_EXTENSIONS = Object.freeze([...C_EXTENSIONS, ...CPP_EXTENSIONS, ...CSHARP_EXTENSIONS]);

const C_EXT_SET = new Set(C_EXTENSIONS);
const CPP_EXT_SET = new Set(CPP_EXTENSIONS);
const CSHARP_EXT_SET = new Set(CSHARP_EXTENSIONS);

const ORIGIN = 'c-family';

function extensionOf(path) {
  const p = String(path || '');
  let sep = -1;
  for (let i = p.length - 1; i >= 0; i--) {
    const c = p.charCodeAt(i);
    if (c === 47 || c === 92) { sep = i; break; }
  }
  const dot = p.lastIndexOf('.');
  if (dot <= sep) return '';
  return p.slice(dot).toLowerCase();
}

function normalizePath(path) {
  const p = String(path || '');
  let out = '';
  for (let i = 0; i < p.length; i++) {
    const c = p.charCodeAt(i);
    out += c === 92 ? '/' : p[i];
  }
  return out;
}

/** True when the path has a C, C++ or C# extension. */
export function isCFamilyPath(path) {
  const ext = extensionOf(path);
  return C_EXT_SET.has(ext) || CPP_EXT_SET.has(ext) || CSHARP_EXT_SET.has(ext);
}

/**
 * True when any path in the list is a C++ source or header (.cpp, .hpp, ...). A project that
 * contains C++ treats its .h headers as C++.
 */
export function projectHasCppSources(paths) {
  if (!paths) return false;
  for (const p of paths) {
    const path = typeof p === 'string' ? p : (p && p.path);
    if (CPP_EXT_SET.has(extensionOf(path))) return true;
  }
  return false;
}

/**
 * Language key for a path: 'c', 'cpp', 'csharp', or null when the extension is not part of the
 * C family. A .h header maps to 'cpp' when options.projectHasCpp is set and to 'c' otherwise;
 * parseCFamilySource additionally promotes a .h header to C++ when it uses C++-only syntax.
 */
export function cFamilyLanguageForPath(path, options = {}) {
  const ext = extensionOf(path);
  if (ext === '.h') return options && options.projectHasCpp ? LANG_CPP : LANG_C;
  if (C_EXT_SET.has(ext)) return LANG_C;
  if (CPP_EXT_SET.has(ext)) return LANG_CPP;
  if (CSHARP_EXT_SET.has(ext)) return LANG_CSHARP;
  return null;
}

// ── 2. Character classes ───────────────────────────────────────────────────────

const CH_TAB = 9;
const CH_LF = 10;
const CH_VT = 11;
const CH_FF = 12;
const CH_CR = 13;
const CH_SPACE = 32;
const CH_BANG = 33;
const CH_DQUOTE = 34;
const CH_HASH = 35;
const CH_DOLLAR = 36;
const CH_PERCENT = 37;
const CH_AMP = 38;
const CH_SQUOTE = 39;
const CH_LPAREN = 40;
const CH_RPAREN = 41;
const CH_STAR = 42;
const CH_PLUS = 43;
const CH_MINUS = 45;
const CH_DOT = 46;
const CH_SLASH = 47;
const CH_COLON = 58;
const CH_LT = 60;
const CH_EQ = 61;
const CH_GT = 62;
const CH_QUESTION = 63;
const CH_AT = 64;
const CH_LBRACKET = 91;
const CH_BACKSLASH = 92;
const CH_RBRACKET = 93;
const CH_CARET = 94;
const CH_UNDERSCORE = 95;
const CH_LBRACE = 123;
const CH_PIPE = 124;
const CH_RBRACE = 125;

function isDigit(c) {
  return c >= 48 && c <= 57;
}

function isAsciiAlnum(c) {
  return (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122);
}

function isIdentStart(c, allowDollar) {
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === CH_UNDERSCORE ||
    (allowDollar && c === CH_DOLLAR) || (c > 127 && c !== 0xFEFF && c !== 0xA0);
}

function isIdentPart(c, allowDollar) {
  return isIdentStart(c, allowDollar) || (c >= 48 && c <= 57);
}

function isHorizontalSpace(c) {
  return c === CH_SPACE || c === CH_TAB || c === CH_CR || c === CH_VT || c === CH_FF || c === 0xFEFF || c === 0xA0;
}

/** An identifier written entirely in upper case (digits and underscores allowed): macro style. */
function isMacroStyle(name) {
  let letters = 0;
  if (!name || name.length < 2) return false;
  for (let i = 0; i < name.length; i++) {
    const c = name.charCodeAt(i);
    if (c >= 97 && c <= 122) return false;
    if (c >= 65 && c <= 90) letters++;
    else if (c !== CH_UNDERSCORE && !(c >= 48 && c <= 57)) return false;
  }
  return letters > 0;
}

function endsWith(s, suffix) {
  return s.length >= suffix.length && s.slice(s.length - suffix.length) === suffix;
}

/** Trims ASCII whitespace from both ends without a regular expression. */
function trimText(s) {
  let a = 0;
  let b = s.length;
  while (a < b && (isHorizontalSpace(s.charCodeAt(a)) || s.charCodeAt(a) === CH_LF)) a++;
  while (b > a && (isHorizontalSpace(s.charCodeAt(b - 1)) || s.charCodeAt(b - 1) === CH_LF)) b--;
  return s.slice(a, b);
}

/** Replaces every character outside [A-Za-z0-9_] with '_' (package / module ids). */
function sanitizeId(s) {
  let out = '';
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    out += isAsciiAlnum(c) || c === CH_UNDERSCORE ? s[i] : '_';
  }
  return out;
}

function clip(text, max) {
  if (text === null || text === undefined) return null;
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
