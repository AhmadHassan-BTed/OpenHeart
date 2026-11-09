/**
 * OpenHeart Graph IR Content Hash
 * A real SHA-256 digest of the IR's nodes and edges (crypto.subtle), used as provenance.
 * When Web Crypto is unavailable (non-secure origins) the hash is reported as unavailable
 * together with the reason, never substituted with a made-up value.
 */

function toHex(buffer) {
  const bytes = new Uint8Array(buffer);
  let out = '';
  for (let i = 0; i < bytes.length; i++) {
    out += bytes[i].toString(16).padStart(2, '0');
  }
  return out;
}

/**
 * @returns {Promise<{ value: string|null, reason: string|null }>}
 */
export async function hashGraphIr(graphIr) {
  if (!graphIr || !Array.isArray(graphIr.nodes)) {
    return { value: null, reason: 'No Graph IR to hash.' };
  }
  const subtle = typeof crypto !== 'undefined' && crypto ? crypto.subtle : undefined;
  if (!subtle || typeof subtle.digest !== 'function') {
    return { value: null, reason: 'Web Crypto (crypto.subtle) is unavailable on this origin; it requires HTTPS or localhost.' };
  }
  try {
    const canonical = JSON.stringify({ nodes: graphIr.nodes, edges: graphIr.edges || [] });
    const digest = await subtle.digest('SHA-256', new TextEncoder().encode(canonical));
    return { value: `sha256:${toHex(digest)}`, reason: null };
  } catch (err) {
    return { value: null, reason: `Hashing failed: ${(err && err.message) || err}` };
  }
}

export function shortHash(value) {
  if (!value) return null;
  const idx = value.indexOf(':');
  const hex = idx >= 0 ? value.slice(idx + 1) : value;
  return `${idx >= 0 ? value.slice(0, idx) + ':' : ''}${hex.slice(0, 12)}`;
}
