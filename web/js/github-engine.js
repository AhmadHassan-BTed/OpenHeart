/**
 * OpenHeart In-Browser Ingestion Engine (approximate, declaration-level)
 *
 * Used when the Rust engine is not reachable (for example on GitHub Pages). It ingests a
 * GitHub repository, a local folder or a ZIP archive, selects a bounded, relevance-ranked
 * sample of source files, extracts declarations with the linear-time parser in a Web Worker,
 * and builds a class-model Graph IR. Every result reports exactly what was sampled, skipped
 * and why; nothing in the result claims verification or full-repository coverage.
 */

import { ParserHost } from './analysis/parser-host.js';
import { buildClassModel } from './analysis/model-builder.js';
import { hashGraphIr } from './analysis/ir-hash.js';
import { languageForPath } from './analysis/source-parser.js';

export const INGESTION_LIMITS = Object.freeze({
  maxFileBytes: 512 * 1024,
  maxRemoteFiles: 150,
  maxRemoteBytes: 12 * 1024 * 1024,
  maxLocalFiles: 1500,
  maxLocalBytes: 48 * 1024 * 1024,
  maxZipArchiveBytes: 200 * 1024 * 1024,
  maxZipEntries: 60000,
  maxZipDeclaredBytes: 1024 * 1024 * 1024,
  fetchConcurrency: 6,
  parseTimeoutMs: 4000,
  maxTreeWalkRequests: 40
});

const IGNORED_DIRS = new Set([
  'node_modules', '.git', 'bin', 'obj', 'target', 'dist', 'build', 'out', '.vs', '.idea', '.gradle',
  '__pycache__', 'vendor', 'third_party', 'third-party', 'bower_components', '.venv', 'venv',
  'site-packages', 'pods', 'carthage', '.next', '.nuxt', 'coverage', '.mvn', '.svn', '.hg', '.tox',
  '.cache', '.terraform', 'generated', 'gen'
]);

const TEST_DIRS = new Set(['test', 'tests', '__tests__', 'spec', 'specs', 'testing', 'androidtest', 'testfixtures', 'e2e', 'integration-test', 'integration-tests', 'it']);

const RELEVANT_DIR_HINT = /^(src|main|lib|app|core|pkg|internal|cmd|source|sources|java|kotlin|scala|com|org|net|io)$/i;

export class GitHubError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'GitHubError';
    this.code = code;
    this.details = details;
  }
}

function abortError() {
  const err = new Error('Analysis cancelled.');
  err.name = 'AbortError';
  return err;
}

function throwIfAborted(signal) {
  if (signal && signal.aborted) throw abortError();
}

function nowMs() {
  return typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now();
}

function normalizePath(path) {
  return String(path || '').replace(/\\/g, '/').replace(/^\/+/, '');
}

function encodePathSegments(path) {
  return normalizePath(path).split('/').map(encodeURIComponent).join('/');
}

// ── Path classification ────────────────────────────────────────────────────────

function isTestPath(path) {
  const lower = path.toLowerCase();
  const segments = lower.split('/');
  if (segments.slice(0, -1).some(seg => TEST_DIRS.has(seg))) return true;
  const base = segments[segments.length - 1];
  if (/^test_.+\.py$/.test(base) || /_test\.(go|py)$/.test(base)) return true;
  if (/\.(spec|test)\.[cm]?[jt]sx?$/.test(base)) return true;
  const original = path.split('/').pop();
  return /(Tests?|Spec|IT)\.(java|kt|kts|cs|scala|swift)$/.test(original);
}

function isMinifiedName(path) {
  return /(\.min\.[cm]?js|\.bundle\.js|-min\.js|\.packed\.js)$/i.test(path);
}

function compileAttributePattern(pattern) {
  let p = pattern.trim();
  if (!p || p.startsWith('#')) return null;
  const anchored = p.startsWith('/');
  if (anchored) p = p.slice(1);
  const dirOnly = p.endsWith('/');
  if (dirOnly) p = p.slice(0, -1);
  let re = '';
  for (let i = 0; i < p.length; i++) {
    const c = p[i];
    if (c === '*') {
      if (p[i + 1] === '*') {
        re += '.*';
        i++;
        if (p[i + 1] === '/') i++;
      } else {
        re += '[^/]*';
      }
    } else if (c === '?') {
      re += '[^/]';
    } else if ('.+()[]{}^$|\\'.includes(c)) {
      re += `\\${c}`;
    } else {
      re += c;
    }
  }
  const hasSlash = p.includes('/');
  const prefix = anchored || hasSlash ? '^' : '(^|/)';
  const suffix = dirOnly ? '/' : '($|/)';
  try {
    return new RegExp(prefix + re + suffix);
  } catch (_) {
    return null;
  }
}

/**
 * Parses .gitattributes linguist-vendored / linguist-generated rules.
 * Later rules override earlier ones, as in git.
 */
export function parseLinguistAttributes(text) {
  const rules = [];
  String(text || '').split('\n').forEach(line => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) return;
    const parts = trimmed.split(/\s+/);
    const pattern = compileAttributePattern(parts[0]);
    if (!pattern) return;
    let vendored = null;
    let generated = null;
    parts.slice(1).forEach(attr => {
      const lower = attr.toLowerCase();
      if (lower === 'linguist-vendored' || lower === 'linguist-vendored=true') vendored = true;
      else if (lower === '-linguist-vendored' || lower === 'linguist-vendored=false') vendored = false;
      else if (lower === 'linguist-generated' || lower === 'linguist-generated=true') generated = true;
      else if (lower === '-linguist-generated' || lower === 'linguist-generated=false') generated = false;
    });
    if (vendored !== null || generated !== null) rules.push({ pattern, vendored, generated });
  });
  return rules;
}

function attributeFlags(path, rules) {
  let vendored = false;
  let generated = false;
  for (const rule of rules) {
    if (!rule.pattern.test(path)) continue;
    if (rule.vendored !== null) vendored = rule.vendored;
    if (rule.generated !== null) generated = rule.generated;
  }
  return { vendored, generated };
}

/**
 * Classifies every candidate file. Returns { eligible: [...], skipped: { reason: count } }.
 * Candidate: { path, size (bytes | null) }
 */
function classifyCandidates(candidates, attributeRules, maxFileBytes) {
  const eligible = [];
  const skipped = { unsupported_language: 0, ignored_directory: 0, vendored: 0, generated: 0, test: 0, minified: 0, too_large: 0 };
  for (const c of candidates) {
    const path = normalizePath(c.path);
    const language = languageForPath(path);
    if (!language) { skipped.unsupported_language++; continue; }
    const segments = path.toLowerCase().split('/');
    if (segments.slice(0, -1).some(seg => IGNORED_DIRS.has(seg))) { skipped.ignored_directory++; continue; }
    const flags = attributeFlags(path, attributeRules);
    if (flags.vendored) { skipped.vendored++; continue; }
    if (flags.generated) { skipped.generated++; continue; }
    if (isMinifiedName(path)) { skipped.minified++; continue; }
    if (isTestPath(path)) { skipped.test++; continue; }
    if (typeof c.size === 'number' && c.size > maxFileBytes) { skipped.too_large++; continue; }
    eligible.push(Object.assign({}, c, { path, language }));
  }
  return { eligible, skipped };
}

function relevanceScore(path, language, primaryLanguage) {
  let score = 0;
  if (/(^|\/)src\/main\//.test(path)) score += 4;
  else if (/(^|\/)(src|lib|app|pkg|internal|cmd|source|sources|core)\//.test(path)) score += 2;
  if (language === primaryLanguage) score += 2;
  return score;
}

/**
 * Picks a bounded, deterministic sample: highest relevance tier first, round-robin across
 * top-level modules inside a tier so a large multi-module repository is covered broadly.
 */
function selectSample(eligible, maxFiles, maxBytes) {
  const langCounts = new Map();
  eligible.forEach(f => langCounts.set(f.language, (langCounts.get(f.language) || 0) + 1));
  let primaryLanguage = null;
  let best = -1;
  langCounts.forEach((count, lang) => {
    if (count > best) { best = count; primaryLanguage = lang; }
  });

  const firstSegments = new Set(eligible.map(f => f.path.split('/')[0]));
  const moduleDepth = firstSegments.size === 1 && eligible.some(f => f.path.includes('/')) ? 1 : 0;
  const moduleKey = (path) => {
    const segs = path.split('/');
    return segs.length > moduleDepth + 1 ? segs.slice(0, moduleDepth + 1).join('/') : '';
  };

  const tiers = new Map();
  eligible.forEach(f => {
    const score = relevanceScore(f.path, f.language, primaryLanguage);
    if (!tiers.has(score)) tiers.set(score, new Map());
    const groups = tiers.get(score);
    const key = moduleKey(f.path);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(f);
  });

  const selected = [];
  let bytes = 0;
  let budgetHit = false;
  const scores = Array.from(tiers.keys()).sort((a, b) => b - a);
  outer:
  for (const score of scores) {
    const groups = tiers.get(score);
    const keys = Array.from(groups.keys()).sort();
    keys.forEach(k => groups.get(k).sort((a, b) => (a.path < b.path ? -1 : 1)));
    const cursors = new Map(keys.map(k => [k, 0]));
    let progressed = true;
    while (progressed) {
      progressed = false;
      for (const k of keys) {
        const list = groups.get(k);
        const idx = cursors.get(k);
        if (idx >= list.length) continue;
        progressed = true;
        const f = list[idx];
        cursors.set(k, idx + 1);
        const size = typeof f.size === 'number' ? f.size : 0;
        if (selected.length >= maxFiles || bytes + size > maxBytes) {
          budgetHit = true;
          break outer;
        }
        selected.push(f);
        bytes += size;
      }
    }
  }
  return { selected, selectedBytes: bytes, budgetHit, primaryLanguage, moduleCount: new Set(eligible.map(f => moduleKey(f.path))).size };
}

function isProbablyMinified(code) {
  if (code.length < 4000) return false;
  let lines = 1;
  let longest = 0;
  let current = 0;
  for (let i = 0; i < code.length; i++) {
    if (code.charCodeAt(i) === 10) {
      lines++;
      if (current > longest) longest = current;
      current = 0;
    } else {
      current++;
    }
  }
  if (current > longest) longest = current;
  return longest > 5000 || code.length / lines > 400;
}

async function runPool(items, limit, signal, worker) {
  let next = 0;
  let stopped = false;
  const runners = [];
  const count = Math.min(limit, items.length);
  for (let r = 0; r < count; r++) {
    runners.push((async () => {
      while (!stopped && next < items.length) {
        throwIfAborted(signal);
        const idx = next++;
        try {
          await worker(items[idx], idx);
        } catch (err) {
          // One fatal error (cancellation, exhausted rate limit) stops every runner.
          stopped = true;
          throw err;
        }
      }
    })());
  }
  await Promise.all(runners);
}

// ── GitHub REST client ─────────────────────────────────────────────────────────

class GitHubApi {
  constructor({ token = null, signal = null } = {}) {
    this.token = token || null;
    this.signal = signal;
    this.requests = 0;
    this.rateLimit = { limit: null, remaining: null, reset: null };
  }

  captureRateLimit(headers) {
    const limit = parseInt(headers.get('x-ratelimit-limit'), 10);
    const remaining = parseInt(headers.get('x-ratelimit-remaining'), 10);
    const reset = parseInt(headers.get('x-ratelimit-reset'), 10);
    if (Number.isFinite(limit)) this.rateLimit.limit = limit;
    if (Number.isFinite(remaining)) this.rateLimit.remaining = remaining;
    if (Number.isFinite(reset)) this.rateLimit.reset = reset;
  }

  async request(path, { accept = 'application/vnd.github+json', as = 'json' } = {}) {
    throwIfAborted(this.signal);
    const headers = { Accept: accept };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    let res;
    try {
      res = await fetch(`https://api.github.com${path}`, { headers, signal: this.signal, credentials: 'omit', referrerPolicy: 'no-referrer' });
    } catch (err) {
      if (err && err.name === 'AbortError') throw err;
      throw new GitHubError('network', `Could not reach api.github.com: ${(err && err.message) || err}.`);
    }
    this.requests++;
    this.captureRateLimit(res.headers);
    if (res.ok) return as === 'text' ? res.text() : res.json();

    let message = `HTTP ${res.status}`;
    try {
      const body = await res.json();
      if (body && typeof body.message === 'string') message = body.message;
    } catch (_) { /* non-JSON error body */ }

    if ((res.status === 403 || res.status === 429) && (this.rateLimit.remaining === 0 || /rate limit/i.test(message))) {
      const resetDate = this.rateLimit.reset ? new Date(this.rateLimit.reset * 1000) : null;
      const minutes = resetDate ? Math.max(0, Math.ceil((resetDate.getTime() - Date.now()) / 60000)) : null;
      const resetText = resetDate ? ` It resets at ${resetDate.toLocaleTimeString()} (in about ${minutes} min).` : '';
      const advice = this.token ? '' : ' Unauthenticated requests are limited to 60 per hour per IP address; add a GitHub token to raise the limit.';
      throw new GitHubError('rate-limit', `GitHub API rate limit exhausted (${this.rateLimit.limit ?? 'unknown'} requests/hour).${resetText}${advice}`, { reset: this.rateLimit.reset });
    }
    if (res.status === 401) {
      throw new GitHubError('auth', 'GitHub rejected the access token (HTTP 401). Check the token or remove it.');
    }
    if (res.status === 404) {
      throw new GitHubError('not-found', this.token
        ? `GitHub returned 404 for ${path}: the repository, ref or path does not exist, or the token lacks access.`
        : `GitHub returned 404 for ${path}: the repository, ref or path does not exist, or the repository is private (private repositories need a token with Contents: read).`);
    }
    if (res.status === 409) {
      throw new GitHubError('empty', 'The repository is empty (GitHub returned HTTP 409).');
    }
    throw new GitHubError('http', `GitHub API request ${path} failed: ${message} (HTTP ${res.status}).`);
  }
}

function repoPath(owner, repo) {
  return `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`;
}

async function resolveRefAndPath(api, owner, repo, segments) {
  const joined = segments.join('/');
  for (const kind of ['heads', 'tags']) {
    const refs = await api.request(`${repoPath(owner, repo)}/git/matching-refs/${kind}/${encodeURIComponent(segments[0])}`);
    if (!Array.isArray(refs)) continue;
    const names = refs
      .map(r => (typeof r.ref === 'string' ? r.ref.replace(`refs/${kind}/`, '') : null))
      .filter(name => name && (joined === name || joined.startsWith(`${name}/`)))
      .sort((a, b) => b.length - a.length);
    if (names.length) {
      return { ref: names[0], refKind: kind === 'heads' ? 'branch' : 'tag', subPath: joined.slice(names[0].length + 1) };
    }
  }
  if (/^[0-9a-f]{7,40}$/i.test(segments[0])) {
    return { ref: segments[0], refKind: 'commit', subPath: segments.slice(1).join('/') };
  }
  throw new GitHubError('ref', `Could not resolve "${joined}" to a branch, tag or commit of ${owner}/${repo}.`);
}

async function fetchRepositoryTree(api, owner, repo, commitSha, subPath, onProgress) {
  const root = await api.request(`${repoPath(owner, repo)}/git/trees/${encodeURIComponent(commitSha)}?recursive=1`);
  if (!root || !Array.isArray(root.tree)) {
    throw new GitHubError('tree', `GitHub returned no tree for ${owner}/${repo}@${commitSha.slice(0, 12)}.`);
  }
  const prefix = subPath ? `${subPath}/` : '';
  if (!root.truncated) {
    const blobs = root.tree
      .filter(e => e.type === 'blob' && (!subPath || e.path === subPath || e.path.startsWith(prefix)))
      .map(e => ({ path: e.path, size: typeof e.size === 'number' ? e.size : null, sha: e.sha }));
    if (subPath && blobs.length === 0) {
      throw new GitHubError('path', `Path "${subPath}" does not exist in ${owner}/${repo} at ${commitSha.slice(0, 12)}.`);
    }
    return { blobs, truncated: false, walkIncomplete: false };
  }

  // Recursive listing was truncated by GitHub: walk subtrees non-recursively within a request budget.
  if (onProgress) onProgress('GitHub truncated the recursive tree listing; walking directories individually...');
  let startSha = root.sha;
  let startPrefix = '';
  if (subPath) {
    for (const segment of subPath.split('/')) {
      const level = await api.request(`${repoPath(owner, repo)}/git/trees/${encodeURIComponent(startSha)}`);
      const hit = Array.isArray(level.tree) ? level.tree.find(e => e.path === segment) : null;
      if (!hit) throw new GitHubError('path', `Path "${subPath}" does not exist in ${owner}/${repo} at ${commitSha.slice(0, 12)}.`);
      if (hit.type === 'blob') {
        return { blobs: [{ path: subPath, size: hit.size ?? null, sha: hit.sha }], truncated: true, walkIncomplete: false };
      }
      startSha = hit.sha;
      startPrefix = startPrefix ? `${startPrefix}/${segment}` : segment;
    }
  }
  const blobs = [];
  const preferred = [{ sha: startSha, prefix: startPrefix }];
  const normal = [];
  let budget = INGESTION_LIMITS.maxTreeWalkRequests;
  while ((preferred.length || normal.length) && budget > 0) {
    const item = preferred.length ? preferred.shift() : normal.shift();
    budget--;
    const level = await api.request(`${repoPath(owner, repo)}/git/trees/${encodeURIComponent(item.sha)}`);
    (level.tree || []).forEach(e => {
      const path = item.prefix ? `${item.prefix}/${e.path}` : e.path;
      if (e.type === 'tree') {
        if (IGNORED_DIRS.has(e.path.toLowerCase())) return;
        (RELEVANT_DIR_HINT.test(e.path) ? preferred : normal).push({ sha: e.sha, prefix: path });
      } else if (e.type === 'blob') {
        blobs.push({ path, size: typeof e.size === 'number' ? e.size : null, sha: e.sha });
      }
    });
  }
  return { blobs, truncated: true, walkIncomplete: preferred.length + normal.length > 0 };
}

// ── Shared analysis core ───────────────────────────────────────────────────────

async function analyzeSelection({ selected, readText, onProgress, signal, title, rawUrlFor, label }) {
  const host = new ParserHost({ timeoutMs: INGESTION_LIMITS.parseTimeoutMs });
  const parses = [];
  const fileCache = new Map();
  const failures = { read_failed: 0, minified_content: 0, parse_failed: 0, parse_timeout: 0 };
  const failureSamples = [];
  let done = 0;
  try {
    await runPool(selected, INGESTION_LIMITS.fetchConcurrency, signal, async (file) => {
      let code;
      try {
        code = await readText(file);
      } catch (err) {
        if (err && (err.name === 'AbortError' || err.code === 'rate-limit' || err.code === 'auth')) throw err;
        failures.read_failed++;
        if (failureSamples.length < 8) failureSamples.push(`${file.path}: ${(err && err.message) || err}`);
        done++;
        return;
      }
      throwIfAborted(signal);
      if (typeof code !== 'string') {
        failures.read_failed++;
        done++;
        return;
      }
      if (code.length > INGESTION_LIMITS.maxFileBytes) {
        failures.read_failed++;
        if (failureSamples.length < 8) failureSamples.push(`${file.path}: content exceeds ${INGESTION_LIMITS.maxFileBytes} bytes`);
        done++;
        return;
      }
      if (isProbablyMinified(code)) {
        failures.minified_content++;
        done++;
        return;
      }
      fileCache.set(file.path, code);
      const outcome = await host.parse(file.path, code, signal);
      if (outcome.aborted) throw abortError();
      if (outcome.ok) {
        parses.push(outcome.result);
      } else if (outcome.timedOut) {
        failures.parse_timeout++;
        if (failureSamples.length < 8) failureSamples.push(`${file.path}: ${outcome.error}`);
      } else {
        failures.parse_failed++;
        if (failureSamples.length < 8) failureSamples.push(`${file.path}: ${outcome.error}`);
      }
      done++;
      if (onProgress) {
        const pct = 50 + Math.floor((done / Math.max(1, selected.length)) * 40);
        onProgress(pct, `${label}: parsed ${done}/${selected.length} files (${file.path.split('/').pop()})`);
      }
    });
  } finally {
    host.terminate();
  }

  if (onProgress) onProgress(92, 'Resolving types, imports and relations...');
  const { graph_ir: graphIr, stats } = buildClassModel(parses, { title, rawUrlFor });
  const hash = await hashGraphIr(graphIr);
  graphIr.metadata.content_hash = hash.value;
  if (!hash.value) graphIr.metadata.content_hash_unavailable = hash.reason;
  return { graphIr, modelStats: stats, fileCache, failures, failureSamples, parsedFiles: parses.length, parserMode: host.mode };
}

function describeSkips(skipped) {
  return Object.entries(skipped).filter(([, n]) => n > 0).map(([k, n]) => `${n} ${k.replace(/_/g, ' ')}`).join(', ');
}

// ── Project (build and deployment) files for the engine module ─────────────────

export const PROJECT_FILE_LIMITS = Object.freeze({
  maxFiles: 60,
  maxFileBytes: 256 * 1024,
  maxTotalBytes: 3 * 1024 * 1024
});

const K8S_DIR_HINT = /^(k8s|kube|kubernetes|manifests?|deploy|deployments?|deployment|helm|charts?|templates|kustomize|overlays|base|infra|ops|openshift)$/;
const K8S_NAME_HINT = /(deployment|service|ingress|statefulset|daemonset|configmap|cronjob|job|pod|kustomization|namespace|route|hpa|networkpolicy|secret)/;

/**
 * Classifies a path as a build or deployment file the engine's deployment and component
 * views read (Dockerfile, compose, Kubernetes manifests, Maven, Gradle, npm, .NET, CMake).
 * Returns the file kind or null.
 */
export function projectFileKind(path) {
  const p = normalizePath(path);
  const segments = p.toLowerCase().split('/');
  const base = segments[segments.length - 1];
  if (segments.slice(0, -1).some(seg => IGNORED_DIRS.has(seg))) return null;
  if (base === 'dockerfile' || base === 'containerfile' || base.startsWith('dockerfile.') || base.endsWith('.dockerfile')) return 'docker';
  if (/^(docker-)?compose(\.[a-z0-9_-]+)?\.ya?ml$/.test(base)) return 'compose';
  if (base === 'pom.xml') return 'maven';
  if (/^(build|settings)\.gradle(\.kts)?$/.test(base)) return 'gradle';
  if (base === 'package.json') return 'npm';
  if (/\.(csproj|fsproj|vbproj|sln)$/.test(base)) return 'dotnet';
  if (base === 'cmakelists.txt') return 'cmake';
  if (/\.ya?ml$/.test(base)) {
    const inK8sDir = segments.slice(0, -1).some(seg => K8S_DIR_HINT.test(seg));
    if (inK8sDir || K8S_NAME_HINT.test(base)) return 'kubernetes';
  }
  return null;
}

function looksLikeKubernetesManifest(text) {
  return /^\s*apiVersion\s*:/m.test(text) && /^\s*kind\s*:/m.test(text);
}

/**
 * Reads the build and deployment files among `candidates` ({ path, size }) within
 * PROJECT_FILE_LIMITS, shallow paths first. Read failures are counted, never fatal
 * (except cancellation and GitHub rate-limit or authentication errors).
 */
async function collectProjectFiles(candidates, readText, signal) {
  const picked = [];
  candidates.forEach(c => {
    const path = normalizePath(c.path);
    const kind = projectFileKind(path);
    if (!kind) return;
    if (typeof c.size === 'number' && c.size > PROJECT_FILE_LIMITS.maxFileBytes) return;
    picked.push({ path, kind, size: typeof c.size === 'number' ? c.size : null, source: c });
  });
  picked.sort((a, b) => (a.path.split('/').length - b.path.split('/').length) || (a.path < b.path ? -1 : 1));
  const stats = { found: picked.length, read: 0, failed: 0, skipped: 0, by_kind: {} };
  const files = [];
  let total = 0;
  for (const item of picked) {
    throwIfAborted(signal);
    if (files.length >= PROJECT_FILE_LIMITS.maxFiles || (item.size !== null && total + item.size > PROJECT_FILE_LIMITS.maxTotalBytes)) {
      stats.skipped++;
      continue;
    }
    let text;
    try {
      text = await readText(item.source);
    } catch (err) {
      if (err && (err.name === 'AbortError' || err.code === 'rate-limit' || err.code === 'auth')) throw err;
      stats.failed++;
      continue;
    }
    if (typeof text !== 'string' || text.length > PROJECT_FILE_LIMITS.maxFileBytes || total + text.length > PROJECT_FILE_LIMITS.maxTotalBytes) {
      stats.skipped++;
      continue;
    }
    if (item.kind === 'kubernetes' && !looksLikeKubernetesManifest(text)) {
      stats.skipped++;
      continue;
    }
    total += text.length;
    files.push({ path: item.path, text });
    stats.read++;
    stats.by_kind[item.kind] = (stats.by_kind[item.kind] || 0) + 1;
  }
  return { files, stats };
}

// ── Public API ─────────────────────────────────────────────────────────────────

export class GitHubEngine {
  /**
   * Parses a GitHub URL. Accepts https://github.com/owner/repo[/tree|blob/<ref and path>],
   * owner/repo shorthand and git@github.com:owner/repo.git. Returns null when not a GitHub repo.
   * The ref and sub-path cannot be separated without the API (refs may contain '/'), so the
   * raw segments after tree/blob are returned for resolution.
   */
  static parseRepoUrl(url) {
    if (!url || typeof url !== 'string') return null;
    let clean = url.trim();
    clean = clean.replace(/^git@github\.com:/i, '');
    clean = clean.replace(/^https?:\/\/(www\.)?github\.com\//i, '');
    clean = clean.replace(/[?#].*$/, '').replace(/\.git$/i, '').replace(/^\/+|\/+$/g, '');
    if (/^[a-z][a-z0-9+.-]*:/i.test(clean)) return null;
    const parts = clean.split('/').filter(Boolean).map(p => {
      try { return decodeURIComponent(p); } catch (_) { return p; }
    });
    if (parts.length < 2) return null;
    const [owner, repo] = parts;
    if (!/^[A-Za-z0-9-]{1,39}$/.test(owner) || !/^[A-Za-z0-9._-]{1,100}$/.test(repo)) return null;
    let refSegments = [];
    if ((parts[2] === 'tree' || parts[2] === 'blob') && parts.length > 3) {
      refSegments = parts.slice(3);
    }
    return { owner, repo: repo.replace(/\.git$/i, ''), refSegments };
  }

  /**
   * @param {string} repoUrl
   * @param {{ onProgress?: (pct:number, message:string)=>void, signal?: AbortSignal, token?: string|null }} options
   */
  static async analyzeGitHubRepo(repoUrl, options = {}) {
    const { onProgress = null, signal = null, token = null, collectProjectFiles: wantProjectFiles = false } = options;
    const started = nowMs();
    const info = this.parseRepoUrl(repoUrl);
    if (!info) {
      throw new GitHubError('url', `"${repoUrl}" is not a GitHub repository URL. Expected https://github.com/owner/repo.`);
    }
    const { owner, repo } = info;
    const api = new GitHubApi({ token, signal });

    if (onProgress) onProgress(8, `Resolving ${owner}/${repo}...`);
    let ref = 'HEAD';
    let refKind = 'default-branch';
    let subPath = '';
    if (info.refSegments.length) {
      ({ ref, refKind, subPath } = await resolveRefAndPath(api, owner, repo, info.refSegments));
    }
    const commitSha = (await api.request(`${repoPath(owner, repo)}/commits/${encodeURIComponent(ref)}`, {
      accept: 'application/vnd.github.sha',
      as: 'text'
    })).trim();
    if (!/^[0-9a-f]{40}$/i.test(commitSha)) {
      throw new GitHubError('ref', `GitHub did not return a commit SHA for ${owner}/${repo}@${ref}.`);
    }

    if (onProgress) onProgress(18, `Listing files at ${commitSha.slice(0, 12)}${subPath ? ` under ${subPath}` : ''}...`);
    const tree = await fetchRepositoryTree(api, owner, repo, commitSha, subPath, (msg) => onProgress && onProgress(22, msg));

    const rawUrlFor = (path) => `https://raw.githubusercontent.com/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/${commitSha}/${encodePathSegments(path)}`;

    let attributeRules = [];
    const attrBlob = tree.blobs.find(b => b.path === '.gitattributes');
    if (attrBlob) {
      try {
        const text = token
          ? await api.request(`${repoPath(owner, repo)}/git/blobs/${attrBlob.sha}`, { accept: 'application/vnd.github.raw+json', as: 'text' })
          : await (await fetch(rawUrlFor('.gitattributes'), { signal, credentials: 'omit' })).text();
        attributeRules = parseLinguistAttributes(text);
      } catch (err) {
        if (err && err.name === 'AbortError') throw err;
        attributeRules = [];
      }
    }

    const { eligible, skipped } = classifyCandidates(tree.blobs, attributeRules, INGESTION_LIMITS.maxFileBytes);
    if (eligible.length === 0) {
      throw new GitHubError('no-sources', `No supported source files were found in ${owner}/${repo}${subPath ? `/${subPath}` : ''} (${tree.blobs.length} files listed; skipped: ${describeSkips(skipped) || 'none'}).`);
    }
    const sample = selectSample(eligible, INGESTION_LIMITS.maxRemoteFiles, INGESTION_LIMITS.maxRemoteBytes);
    if (onProgress) onProgress(30, `Fetching ${sample.selected.length} of ${eligible.length} eligible source files...`);

    const readText = async (file) => {
      if (token) {
        return api.request(`${repoPath(owner, repo)}/git/blobs/${file.sha}`, { accept: 'application/vnd.github.raw+json', as: 'text' });
      }
      const res = await fetch(rawUrlFor(file.path), { signal, credentials: 'omit', referrerPolicy: 'no-referrer' });
      if (!res.ok) throw new Error(`raw.githubusercontent.com returned HTTP ${res.status}`);
      return res.text();
    };

    const analysis = await analyzeSelection({
      selected: sample.selected,
      readText,
      onProgress,
      signal,
      title: `${owner}/${repo}${subPath ? `/${subPath}` : ''}`,
      rawUrlFor: token ? null : rawUrlFor,
      label: `${owner}/${repo}`
    });

    let project = { files: [], stats: null };
    if (wantProjectFiles) {
      if (onProgress) onProgress(96, `Reading build and deployment files of ${owner}/${repo}...`);
      project = await collectProjectFiles(tree.blobs, readText, signal);
    }

    const elapsed = Math.round(nowMs() - started);
    if (onProgress) onProgress(100, `In-browser class model ready for ${owner}/${repo}.`);
    return {
      status: 'success',
      source: {
        kind: 'github',
        name: `${owner}/${repo}`,
        url: `https://github.com/${owner}/${repo}`,
        owner,
        repo,
        ref,
        ref_kind: refKind,
        commit: commitSha,
        sub_path: subPath || null,
        private_access: Boolean(token)
      },
      session_id: `gh:${owner}/${repo}@${commitSha.slice(0, 12)}`,
      stats: Object.assign({
        engine: 'in-browser declaration extractor (approximate)',
        files_listed: tree.blobs.length,
        files_eligible: eligible.length,
        files_selected: sample.selected.length,
        files_parsed: analysis.parsedFiles,
        files_skipped: skipped,
        files_failed: analysis.failures,
        failure_samples: analysis.failureSamples,
        sample_budget: { max_files: INGESTION_LIMITS.maxRemoteFiles, max_bytes: INGESTION_LIMITS.maxRemoteBytes, hit: sample.budgetHit },
        module_groups: sample.moduleCount,
        primary_language: sample.primaryLanguage,
        tree_truncated: tree.truncated,
        tree_walk_incomplete: tree.walkIncomplete,
        api_requests: api.requests,
        rate_limit: Object.assign({}, api.rateLimit),
        parser_mode: analysis.parserMode,
        project_files: project.stats,
        execution_time_ms: elapsed
      }, analysis.modelStats),
      graph_ir: analysis.graphIr,
      fileCache: analysis.fileCache,
      projectFiles: project.files
    };
  }

  /**
   * Analyzes local files (folder picker, drag and drop, or ZIP entries).
   * @param {Array<{ path: string, size: number|null, getText: () => Promise<string> }>} fileList
   * @param {string} codebaseName
   * @param {{ onProgress?: Function, signal?: AbortSignal, sourceKind?: 'folder'|'zip' }} options
   */
  static async analyzeSourceFiles(fileList, codebaseName, options = {}) {
    const { onProgress = null, signal = null, sourceKind = 'folder', collectProjectFiles: wantProjectFiles = false } = options;
    const started = nowMs();
    if (!Array.isArray(fileList) || fileList.length === 0) {
      throw new Error('No files were provided for analysis.');
    }
    if (onProgress) onProgress(12, `Scanning ${fileList.length} files in ${codebaseName}...`);
    const byPath = new Map();
    fileList.forEach(f => byPath.set(normalizePath(f.path), f));

    let attributeRules = [];
    const attrPath = Array.from(byPath.keys()).find(p => p === '.gitattributes' || /^[^/]+\/\.gitattributes$/.test(p));
    if (attrPath) {
      try {
        attributeRules = parseLinguistAttributes(await byPath.get(attrPath).getText());
      } catch (_) {
        attributeRules = [];
      }
    }

    const candidates = Array.from(byPath.entries()).map(([path, f]) => ({ path, size: typeof f.size === 'number' ? f.size : null }));
    const { eligible, skipped } = classifyCandidates(candidates, attributeRules, INGESTION_LIMITS.maxFileBytes);
    if (eligible.length === 0) {
      throw new Error(`No supported source files were found in "${codebaseName}" (${candidates.length} files; skipped: ${describeSkips(skipped) || 'none'}). Supported: Java, Kotlin, Scala, C#, TypeScript, JavaScript, Python, Rust, Go, Swift, C/C++.`);
    }
    const sample = selectSample(eligible, INGESTION_LIMITS.maxLocalFiles, INGESTION_LIMITS.maxLocalBytes);
    if (onProgress) onProgress(30, `Reading ${sample.selected.length} of ${eligible.length} eligible source files...`);

    const analysis = await analyzeSelection({
      selected: sample.selected,
      readText: (file) => byPath.get(file.path).getText(),
      onProgress,
      signal,
      title: codebaseName,
      rawUrlFor: null,
      label: codebaseName
    });
    let project = { files: [], stats: null };
    if (wantProjectFiles) {
      if (onProgress) onProgress(96, `Reading build and deployment files of ${codebaseName}...`);
      project = await collectProjectFiles(candidates, (c) => byPath.get(normalizePath(c.path)).getText(), signal);
    }
    const elapsed = Math.round(nowMs() - started);
    if (onProgress) onProgress(100, `In-browser class model ready for ${codebaseName}.`);
    return {
      status: 'success',
      source: { kind: sourceKind, name: codebaseName, url: null },
      session_id: `${sourceKind}:${codebaseName}:${analysis.graphIr.metadata.content_hash ? analysis.graphIr.metadata.content_hash.slice(7, 19) : Date.now().toString(36)}`,
      stats: Object.assign({
        engine: 'in-browser declaration extractor (approximate)',
        files_listed: candidates.length,
        files_eligible: eligible.length,
        files_selected: sample.selected.length,
        files_parsed: analysis.parsedFiles,
        files_skipped: skipped,
        files_failed: analysis.failures,
        failure_samples: analysis.failureSamples,
        sample_budget: { max_files: INGESTION_LIMITS.maxLocalFiles, max_bytes: INGESTION_LIMITS.maxLocalBytes, hit: sample.budgetHit },
        module_groups: sample.moduleCount,
        primary_language: sample.primaryLanguage,
        tree_truncated: false,
        tree_walk_incomplete: false,
        parser_mode: analysis.parserMode,
        project_files: project.stats,
        execution_time_ms: elapsed
      }, analysis.modelStats),
      graph_ir: analysis.graphIr,
      fileCache: analysis.fileCache,
      projectFiles: project.files
    };
  }

  /**
   * Opens a ZIP archive with JSZip and returns lazily-read, size-bounded file items.
   * Entry contents are streamed and the read is aborted as soon as an entry exceeds the
   * per-file cap, so a decompression bomb cannot inflate past the limit in memory.
   */
  static async readZipArchive(file) {
    if (typeof JSZip === 'undefined') {
      throw new Error('The ZIP library (JSZip) is not loaded; check your network connection and reload.');
    }
    if (file.size > INGESTION_LIMITS.maxZipArchiveBytes) {
      throw new Error(`The archive is ${Math.round(file.size / 1048576)} MB; the in-browser limit is ${Math.round(INGESTION_LIMITS.maxZipArchiveBytes / 1048576)} MB.`);
    }
    const zip = await JSZip.loadAsync(file);
    const items = [];
    let declaredTotal = 0;
    let entryCount = 0;
    zip.forEach((relPath, entry) => {
      entryCount++;
      if (entry.dir) return;
      const declared = entry._data && typeof entry._data.uncompressedSize === 'number' ? entry._data.uncompressedSize : null;
      if (declared !== null) declaredTotal += declared;
      items.push({
        path: relPath,
        size: declared,
        getText: () => readZipEntryBounded(entry, INGESTION_LIMITS.maxFileBytes)
      });
    });
    if (entryCount > INGESTION_LIMITS.maxZipEntries) {
      throw new Error(`The archive lists ${entryCount} entries; the in-browser limit is ${INGESTION_LIMITS.maxZipEntries}.`);
    }
    if (declaredTotal > INGESTION_LIMITS.maxZipDeclaredBytes) {
      throw new Error(`The archive declares ${Math.round(declaredTotal / 1048576)} MB of uncompressed data, above the ${Math.round(INGESTION_LIMITS.maxZipDeclaredBytes / 1048576)} MB safety limit.`);
    }
    return items;
  }
}

function readZipEntryBounded(entry, maxBytes) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let total = 0;
    let finished = false;
    let stream;
    try {
      stream = entry.internalStream('uint8array');
    } catch (err) {
      reject(err);
      return;
    }
    stream.on('data', (chunk) => {
      if (finished) return;
      total += chunk.length;
      if (total > maxBytes) {
        finished = true;
        stream.pause();
        reject(new Error(`entry inflates beyond ${maxBytes} bytes`));
        return;
      }
      chunks.push(chunk);
    });
    stream.on('error', (err) => {
      if (finished) return;
      finished = true;
      reject(err);
    });
    stream.on('end', () => {
      if (finished) return;
      finished = true;
      const buf = new Uint8Array(total);
      let offset = 0;
      chunks.forEach(c => { buf.set(c, offset); offset += c.length; });
      resolve(new TextDecoder('utf-8').decode(buf));
    });
    stream.resume();
  });
}
