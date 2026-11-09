/**
 * OpenHeart Safe DOM Construction Helpers
 * Every dynamic string reaches the document through textContent or attribute setters.
 * The only markup the studio ever parses is developer-authored constant SVG (icons),
 * routed through the 'openheart-static' Trusted Types policy, which accepts nothing
 * except strings that were registered as constants at module load time.
 */

const STATIC_MARKUP = new Set();
let staticPolicy = null;

/**
 * Registers a developer-authored constant markup string so it may later be parsed.
 * Never call this with data that originates from a repository, archive, state file,
 * network response or storage.
 */
export function registerStaticMarkup(markup) {
  if (typeof markup !== 'string') {
    throw new TypeError('Static markup must be a string constant.');
  }
  STATIC_MARKUP.add(markup);
  return markup;
}

function acceptRegisteredMarkup(input) {
  if (STATIC_MARKUP.has(input)) return input;
  throw new TypeError('openheart-static policy rejected markup that is not a registered constant.');
}

function getStaticPolicy() {
  if (staticPolicy) return staticPolicy;
  const tt = typeof window !== 'undefined' ? window.trustedTypes : undefined;
  if (tt && typeof tt.createPolicy === 'function') {
    staticPolicy = tt.createPolicy('openheart-static', { createHTML: acceptRegisteredMarkup });
  } else {
    staticPolicy = { createHTML: acceptRegisteredMarkup };
  }
  return staticPolicy;
}

/**
 * Parses a registered constant markup string into a detached DocumentFragment.
 */
export function staticFragment(markup) {
  const tpl = document.createElement('template');
  tpl.innerHTML = getStaticPolicy().createHTML(markup);
  return tpl.content.cloneNode(true);
}

function appendChildren(el, children) {
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    el.appendChild(child instanceof Node ? child : document.createTextNode(String(child)));
  }
}

/**
 * Creates an element. Props:
 *  - className, text, dataset {..}, style {cssProp: value}, on<Event>: handler,
 *  - any other key becomes an attribute (true -> empty attribute, false/null/undefined -> omitted).
 * Children may be Nodes, strings, numbers, arrays, or null/undefined/false (skipped).
 */
export function h(tag, props = null, ...children) {
  const el = document.createElement(tag);
  if (props) {
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined || value === null || value === false) continue;
      if (key === 'className') {
        el.className = String(value);
      } else if (key === 'text') {
        el.textContent = String(value);
      } else if (key === 'dataset') {
        for (const [dk, dv] of Object.entries(value)) {
          if (dv !== undefined && dv !== null) el.dataset[dk] = String(dv);
        }
      } else if (key === 'style') {
        for (const [prop, v] of Object.entries(value)) {
          if (v !== undefined && v !== null) el.style.setProperty(prop, String(v));
        }
      } else if (key.length > 2 && key.startsWith('on') && typeof value === 'function') {
        el.addEventListener(key.slice(2).toLowerCase(), value);
      } else if (key.startsWith('on')) {
        throw new TypeError(`Refusing to set inline event handler attribute "${key}".`);
      } else {
        el.setAttribute(key, value === true ? '' : String(value));
      }
    }
  }
  appendChildren(el, children);
  return el;
}

/**
 * Replaces all children of an element with the given children (strings become text nodes).
 */
export function replaceChildren(el, ...children) {
  if (!el) return;
  el.textContent = '';
  appendChildren(el, children);
}

/**
 * Convenience for emphasised inline text inside status messages.
 */
export function strong(text) {
  return h('strong', null, text);
}

const HEX_COLOR = /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
const RGB_COLOR = /^rgba?\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*(?:,\s*(?:0|1|0?\.\d+)\s*)?\)$/;

/**
 * Returns the colour if it is a plain hex or rgb()/rgba() literal, otherwise the fallback.
 */
export function safeColor(value, fallback) {
  if (typeof value === 'string') {
    const v = value.trim();
    if (HEX_COLOR.test(v) || RGB_COLOR.test(v)) return v;
  }
  return fallback;
}

/**
 * Restricts a value to characters that are safe inside a downloaded file name.
 */
export function safeFileStem(value, fallback = 'OpenHeart') {
  const cleaned = String(value || '')
    .replace(/[^A-Za-z0-9._-]+/g, '_')
    .replace(/^[._]+/, '')
    .slice(0, 80);
  return cleaned || fallback;
}
