/**
 * OpenHeart Source Viewer & Line Synchronizer
 * Read-only code viewer with sticky line numbers, a single-pass tokenizer for highlighting
 * (each token becomes a text node, so highlighting can never produce markup), configurable
 * font size and word wrap.
 *
 * Sources are resolved only from data the session actually has: the in-memory source map of
 * an imported codebase, the node's raw_url (raw.githubusercontent.com or same-origin), or the
 * analysis server's /api/source for an engine session. Nothing is guessed or synthesised; when
 * no source exists the viewer says so.
 */

import { h, replaceChildren } from './dom.js';
import { icon } from './icons.js';
import { localStore } from './storage.js';
import { studioStore } from './studio-store.js';
import { isAllowedSourceUrl } from './graph-ir-validate.js';
import { languageForPath } from './analysis/source-parser.js';

const KEYWORDS = new Set([
  'public', 'private', 'protected', 'internal', 'class', 'interface', 'enum', 'record', 'implements', 'extends',
  'static', 'final', 'abstract', 'void', 'return', 'new', 'this', 'super', 'import', 'package', 'synchronized',
  'volatile', 'transient', 'native', 'strictfp', 'throws', 'throw', 'try', 'catch', 'finally', 'if', 'else',
  'while', 'for', 'do', 'switch', 'case', 'default', 'break', 'continue', 'instanceof', 'assert', 'val', 'var',
  'fun', 'override', 'const', 'let', 'mut', 'struct', 'impl', 'fn', 'pub', 'trait', 'type', 'match', 'self',
  'Self', 'use', 'mod', 'crate', 'where', 'async', 'await', 'ref', 'move', 'def', 'None', 'True', 'False',
  'from', 'as', 'in', 'is', 'not', 'and', 'or', 'yield', 'lambda', 'with', 'object', 'companion', 'data',
  'sealed', 'open', 'when', 'namespace', 'using', 'readonly', 'export', 'function', 'typeof', 'null', 'true',
  'false', 'undefined', 'func', 'go', 'defer', 'chan', 'select', 'range', 'virtual', 'template', 'typename',
  'protocol', 'extension', 'guard', 'init', 'deinit', 'struct', 'union', 'operator', 'inline', 'suspend'
]);

const TYPES = new Set([
  'int', 'long', 'boolean', 'double', 'float', 'char', 'byte', 'short', 'String', 'Object', 'List', 'Map', 'Set',
  'Optional', 'Integer', 'Long', 'Boolean', 'Double', 'Float', 'Void', 'i8', 'i16', 'i32', 'i64', 'u8', 'u16',
  'u32', 'u64', 'usize', 'isize', 'f32', 'f64', 'bool', 'str', 'Option', 'Result', 'Some', 'Ok', 'Err', 'Vec',
  'BigInteger', 'BigDecimal', 'Int', 'Unit', 'Any', 'string', 'number', 'void', 'any', 'unknown', 'never'
]);

const CHAR_LITERAL_LANGS = new Set(['java', 'kotlin', 'csharp', 'cpp', 'rust', 'go', 'scala', 'swift']);

function isIdentStart(c) {
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 95 || c === 36 || c > 127;
}

function isIdentPart(c) {
  return isIdentStart(c) || (c >= 48 && c <= 57);
}

/**
 * Splits text into highlight tokens in one forward pass.
 * @returns {Array<{ type: string|null, text: string }>}
 */
export function tokenizeForHighlight(text, lang) {
  const tokens = [];
  const n = text.length;
  const hashComments = lang === 'python';
  const slashComments = lang !== 'python';
  const backtickStrings = lang === 'javascript' || lang === 'typescript' || lang === 'go';
  let plainStart = 0;
  let i = 0;
  const flushPlain = (end) => {
    if (end > plainStart) tokens.push({ type: null, text: text.slice(plainStart, end) });
  };
  const emit = (type, end) => {
    flushPlain(i);
    tokens.push({ type, text: text.slice(i, end) });
    i = end;
    plainStart = end;
  };

  while (i < n) {
    const c = text.charCodeAt(i);
    if (slashComments && c === 47 && text.charCodeAt(i + 1) === 47) {
      const end = text.indexOf('\n', i);
      emit('comment', end === -1 ? n : end);
      continue;
    }
    if (slashComments && c === 47 && text.charCodeAt(i + 1) === 42) {
      const close = text.indexOf('*/', i + 2);
      emit('comment', close === -1 ? n : close + 2);
      continue;
    }
    if (hashComments && c === 35) {
      const end = text.indexOf('\n', i);
      emit('comment', end === -1 ? n : end);
      continue;
    }
    if (c === 34 || c === 39 || (c === 96 && backtickStrings)) {
      const quote = text[i];
      if (text.startsWith(quote.repeat(3), i) && (lang === 'python' || lang === 'kotlin' || lang === 'java' || lang === 'scala' || lang === 'swift')) {
        const close = text.indexOf(quote.repeat(3), i + 3);
        emit('string', close === -1 ? n : close + 3);
        continue;
      }
      if (c === 39 && CHAR_LITERAL_LANGS.has(lang)) {
        // Char literal only when it closes within a few characters (otherwise e.g. a Rust lifetime).
        let close = -1;
        for (let j = i + 1; j < Math.min(n, i + 12); j++) {
          const d = text.charCodeAt(j);
          if (d === 92) { j++; continue; }
          if (d === 39) { close = j; break; }
          if (d === 10) break;
        }
        if (close === -1 || close - i > 10) {
          i++;
          continue;
        }
        emit('string', close + 1);
        continue;
      }
      let j = i + 1;
      while (j < n) {
        const d = text.charCodeAt(j);
        if (d === 92) { j += 2; continue; }
        if (d === c) { j++; break; }
        if (d === 10 && c !== 96) break;
        j++;
      }
      emit('string', Math.min(j, n));
      continue;
    }
    if (c === 64 && i + 1 < n && isIdentStart(text.charCodeAt(i + 1))) {
      let j = i + 2;
      while (j < n && (isIdentPart(text.charCodeAt(j)) || text.charCodeAt(j) === 46)) j++;
      emit('annotation', j);
      continue;
    }
    if (c >= 48 && c <= 57 && (i === 0 || !isIdentPart(text.charCodeAt(i - 1)))) {
      let j = i + 1;
      while (j < n && (isIdentPart(text.charCodeAt(j)) || text.charCodeAt(j) === 46)) j++;
      emit('number', j);
      continue;
    }
    if (isIdentStart(c)) {
      let j = i + 1;
      while (j < n && isIdentPart(text.charCodeAt(j))) j++;
      const word = text.slice(i, j);
      if (KEYWORDS.has(word)) emit('keyword', j);
      else if (TYPES.has(word)) emit('type', j);
      else i = j;
      continue;
    }
    i++;
  }
  i = n;
  flushPlain(n);
  return tokens;
}

function normalizePath(path) {
  return String(path || '').replace(/\\/g, '/').replace(/^\/+/, '');
}

function isSafeRelativePath(path) {
  return Boolean(path) && !path.includes('..') && !path.includes(':') && !path.startsWith('/') && !path.includes('\\');
}

export class SourceEditorModule {
  constructor(containerId = 'monaco-container') {
    this.containerId = containerId;
    this.currentFile = null;
    this.currentContent = '';
    this.contentIsSource = false;
    this.targetLines = [];
    this.sourceCache = new Map();
    this.fontSize = Math.min(18, Math.max(9.5, localStore.getNumber('openheart_code_fontsize', 11.5)));
    this.wordWrap = localStore.getBoolean('openheart_code_wrap', false);
    this.requestSeq = 0;
    this.abortController = null;
    this.rows = [];
    this.gutterRows = [];
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.bindToolbarControls();
    container.addEventListener('click', (e) => {
      const gutter = e.target instanceof Element ? e.target.closest('.code-gutter-line') : null;
      if (!gutter) return;
      const line = parseInt(gutter.getAttribute('data-line'), 10);
      if (Number.isInteger(line)) this.highlightLines([line]);
    });
    this.showMessage([
      'OpenHeart source viewer',
      '',
      'Select a type in the diagram or a file in the Project Explorer to view its source.'
    ], null);
  }

  /** Clears every cached source (called when the active codebase changes). */
  clearSources() {
    this.sourceCache.clear();
  }

  setSource(fileName, content) {
    if (!fileName || typeof content !== 'string') return;
    this.sourceCache.set(normalizePath(fileName), content);
  }

  setMultipleSources(fileMap) {
    if (!fileMap) return;
    fileMap.forEach((content, fileName) => this.setSource(fileName, content));
  }

  bindToolbarControls() {
    const btnFontDec = document.getElementById('btn-code-font-dec');
    const btnFontInc = document.getElementById('btn-code-font-inc');
    const fontValLabel = document.getElementById('code-font-val');
    const btnWrap = document.getElementById('btn-code-wrap');
    const btnCopy = document.getElementById('btn-code-copy');

    if (fontValLabel) fontValLabel.textContent = `${this.fontSize}px`;

    const applyFont = (next) => {
      this.fontSize = next;
      localStore.set('openheart_code_fontsize', this.fontSize.toString());
      if (fontValLabel) fontValLabel.textContent = `${this.fontSize}px`;
      this.updateFontAndLayout();
    };
    if (btnFontDec) btnFontDec.addEventListener('click', () => applyFont(Math.max(9.5, +(this.fontSize - 0.5).toFixed(1))));
    if (btnFontInc) btnFontInc.addEventListener('click', () => applyFont(Math.min(18.0, +(this.fontSize + 0.5).toFixed(1))));

    if (btnWrap) {
      btnWrap.classList.toggle('active', this.wordWrap);
      btnWrap.setAttribute('aria-pressed', String(this.wordWrap));
      btnWrap.addEventListener('click', () => {
        this.wordWrap = !this.wordWrap;
        localStore.set('openheart_code_wrap', this.wordWrap.toString());
        btnWrap.classList.toggle('active', this.wordWrap);
        btnWrap.setAttribute('aria-pressed', String(this.wordWrap));
        this.updateFontAndLayout();
      });
    }

    if (btnCopy) {
      btnCopy.addEventListener('click', async () => {
        if (!this.currentContent || !this.contentIsSource) return;
        try {
          await navigator.clipboard.writeText(this.currentContent);
          replaceChildren(btnCopy, icon('check'), ' Copied');
          btnCopy.classList.add('copied');
          setTimeout(() => {
            replaceChildren(btnCopy, icon('copy'), ' Copy');
            btnCopy.classList.remove('copied');
          }, 1800);
        } catch (_) {
          replaceChildren(btnCopy, icon('copy'), ' Copy failed');
          setTimeout(() => replaceChildren(btnCopy, icon('copy'), ' Copy'), 1800);
        }
      });
    }
  }

  updateFontAndLayout() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    const viewport = container.querySelector('.code-editor-viewport');
    if (viewport) {
      viewport.style.setProperty('--code-font-size', `${this.fontSize}px`);
      const body = viewport.querySelector('.code-content-body');
      if (body) body.classList.toggle('wrap-lines', this.wordWrap);
    }
  }

  setTheme() {
    // Colours are driven entirely by CSS custom properties; no re-render is needed.
  }

  setSyncStatus(text, state) {
    const pill = document.getElementById('editor-sync-status');
    if (!pill) return;
    pill.textContent = text;
    pill.dataset.state = state;
  }

  /**
   * Loads and shows a file. A newer call supersedes (and aborts) any load still in flight.
   */
  async loadFile(fileName, nodeData = null, targetLines = []) {
    const requestId = ++this.requestSeq;
    if (this.abortController) this.abortController.abort();
    const controller = new AbortController();
    this.abortController = controller;

    this.currentFile = fileName ? normalizePath(fileName) : null;
    this.targetLines = Array.isArray(targetLines) ? targetLines.filter(l => Number.isInteger(l) && l > 0) : [];
    this.setSyncStatus('Loading…', 'loading');

    let result;
    try {
      result = await this.resolveSource(this.currentFile, nodeData, controller.signal);
    } catch (err) {
      if (err && err.name === 'AbortError') return;
      result = { content: null, reason: `Loading failed: ${(err && err.message) || err}` };
    }
    if (requestId !== this.requestSeq) return;

    if (result.content !== null) {
      this.currentContent = result.content;
      this.contentIsSource = true;
      this.render(languageForPath(this.currentFile || '') || null);
      this.scrollToHighlightedLine();
      this.setSyncStatus(result.originLabel, 'ok');
      return;
    }

    const lines = [result.reason];
    const insts = nodeData && Array.isArray(nodeData.instructions) ? nodeData.instructions.filter(x => typeof x === 'string') : [];
    if (insts.length) {
      lines.push('', 'Recorded IR data for this node:');
      insts.forEach(inst => lines.push(`  ${inst}`));
    }
    this.showMessage(lines, nodeData);
    this.setSyncStatus('No source', 'missing');
  }

  async resolveSource(path, nodeData, signal) {
    if (!path) {
      return { content: null, reason: 'This element has no source file recorded in the diagram data.' };
    }
    if (this.sourceCache.has(path)) {
      return { content: this.sourceCache.get(path), originLabel: 'Imported source' };
    }
    const attempts = [];
    const rawUrl = nodeData ? (nodeData.raw_url || nodeData.rawUrl) : null;
    if (rawUrl) {
      if (!isAllowedSourceUrl(rawUrl)) {
        attempts.push('the recorded source URL is not on raw.githubusercontent.com or this site, so it was not fetched');
      } else {
        try {
          const res = await fetch(rawUrl, { signal, credentials: 'omit', referrerPolicy: 'no-referrer' });
          if (res.ok) {
            const text = await res.text();
            this.sourceCache.set(path, text);
            return { content: text, originLabel: 'GitHub raw' };
          }
          attempts.push(`the source URL returned HTTP ${res.status}`);
        } catch (err) {
          if (err && err.name === 'AbortError') throw err;
          attempts.push(`the source URL could not be fetched (${(err && err.message) || err})`);
        }
      }
    }
    if (studioStore.sourceKind === 'engine') {
      if (!isSafeRelativePath(path)) {
        attempts.push('the recorded path is not a repository-relative path');
      } else {
        try {
          const res = await fetch(`/api/source?file=${encodeURIComponent(path)}`, { signal });
          if (res.ok) {
            const json = await res.json();
            if (json && json.found && typeof json.content === 'string') {
              this.sourceCache.set(path, json.content);
              return { content: json.content, originLabel: 'Analysis server' };
            }
            attempts.push('the analysis server has no source for this path');
          } else {
            attempts.push(`the analysis server returned HTTP ${res.status}`);
          }
        } catch (err) {
          if (err && err.name === 'AbortError') throw err;
          attempts.push(`the analysis server could not be reached (${(err && err.message) || err})`);
        }
      }
    }
    if (attempts.length === 0) {
      return { content: null, reason: `The source of ${path} is not available in this session (the diagram data references it, but no file contents or source URL were provided).` };
    }
    return { content: null, reason: `The source of ${path} could not be loaded: ${attempts.join('; ')}.` };
  }

  showMessage(lines, nodeData) {
    this.currentContent = lines.join('\n');
    this.contentIsSource = false;
    this.targetLines = [];
    this.renderTokens(lines.map(text => [{ type: 'comment', text }]));
  }

  highlightLines(lines = []) {
    this.targetLines = Array.isArray(lines) ? lines.filter(l => Number.isInteger(l) && l > 0) : [];
    const set = new Set(this.targetLines);
    this.rows.forEach((row, idx) => row.classList.toggle('highlighted', set.has(idx + 1)));
    this.gutterRows.forEach((row, idx) => row.classList.toggle('highlighted', set.has(idx + 1)));
    this.scrollToHighlightedLine();
  }

  scrollToHighlightedLine() {
    if (!this.targetLines || this.targetLines.length === 0) return;
    const targetRow = this.rows[this.targetLines[0] - 1];
    if (!targetRow) return;
    requestAnimationFrame(() => {
      targetRow.scrollIntoView({ behavior: 'smooth', block: 'center' });
      targetRow.classList.add('pulse-highlight');
      setTimeout(() => targetRow.classList.remove('pulse-highlight'), 1200);
    });
  }

  render(lang) {
    const tokens = tokenizeForHighlight(this.currentContent || '', lang);
    const lines = [[]];
    tokens.forEach(tok => {
      const parts = tok.text.split('\n');
      parts.forEach((part, idx) => {
        if (idx > 0) lines.push([]);
        if (part) lines[lines.length - 1].push({ type: tok.type, text: part });
      });
    });
    this.renderTokens(lines);
  }

  renderTokens(lines) {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    const highlighted = new Set(this.targetLines);
    const gutter = h('div', { className: 'code-gutter', 'aria-hidden': 'true' });
    const code = h('code');
    const gutterFrag = document.createDocumentFragment();
    const codeFrag = document.createDocumentFragment();
    this.rows = [];
    this.gutterRows = [];
    lines.forEach((parts, idx) => {
      const lineNum = idx + 1;
      const isHl = highlighted.has(lineNum);
      const g = document.createElement('div');
      g.className = isHl ? 'code-gutter-line highlighted' : 'code-gutter-line';
      g.setAttribute('data-line', String(lineNum));
      g.textContent = String(lineNum);
      gutterFrag.appendChild(g);
      this.gutterRows.push(g);

      const row = document.createElement('div');
      row.className = isHl ? 'code-line-row highlighted' : 'code-line-row';
      row.setAttribute('data-line', String(lineNum));
      if (parts.length === 0) {
        row.textContent = ' ';
      } else {
        parts.forEach(p => {
          if (p.type) {
            const span = document.createElement('span');
            span.className = `syn-${p.type}`;
            span.textContent = p.text;
            row.appendChild(span);
          } else {
            row.appendChild(document.createTextNode(p.text));
          }
        });
      }
      codeFrag.appendChild(row);
      this.rows.push(row);
    });
    gutter.appendChild(gutterFrag);
    code.appendChild(codeFrag);
    const body = h('pre', { className: `code-content-body ${this.wordWrap ? 'wrap-lines' : ''}`, tabindex: '0', 'aria-label': this.currentFile ? `Source of ${this.currentFile}` : 'Source viewer' }, code);
    const viewport = h('div', { className: 'code-editor-viewport', style: { '--code-font-size': `${this.fontSize}px` } }, gutter, body);
    replaceChildren(container, viewport);
  }
}
