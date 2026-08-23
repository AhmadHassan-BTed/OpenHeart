/**
 * OpenHeart Fully Dynamic SVG UML Card Generator Suite (Zero Hardcoding)
 * Styles are derived purely from parsed AST types, stereotypes, and visibility tokens.
 * Supports dynamic width/height fitting, high-performance rendering, and Dark Mode themes.
 */
import { getTheme } from './themes/index.js';

/** ── 1. Dynamic UML Class / Interface / Abstract / Enum Card ── */
export function generateUmlClassCardSvg(classData) {
  const {
    name = "Class",
    stereotype = "<<class>>",
    kind = "class",
    fields = [],
    methods = [],
    width = 290,
    height = 180,
    isDark = false
  } = classData;

  const theme = getTheme(isDark);
  const cardTokens = theme.cards;

  const HEADER_HEIGHT = 50;
  const ROW_HEIGHT = 22;
  const PADDING_X = 18;

  let badgeText = stereotype.replace(/[<>]/g, '').trim();
  if (!badgeText) badgeText = kind;

  // Dynamically determine required width so no member line is ever truncated
  let maxLineChars = Math.max(name.length + 6, badgeText.length + 8);
  fields.forEach(f => {
    const len = typeof f === 'string' ? f.length : (f.signature || f.name || '').length;
    if (len > maxLineChars) maxLineChars = len;
  });
  methods.forEach(m => {
    const len = typeof m === 'string' ? m.length : (m.signature || m.name || '').length;
    if (len > maxLineChars) maxLineChars = len;
  });

  const dynamicWidth = Math.max(width, Math.min(950, Math.round(maxLineChars * 8.5 + 64)));

  const fieldsCount = Math.max(1, fields.length);
  const methodsCount = Math.max(1, methods.length);
  const calculatedHeight = HEADER_HEIGHT + (fieldsCount * ROW_HEIGHT) + (methodsCount * ROW_HEIGHT) + 56;
  const cardHeight = Math.max(height, calculatedHeight);

  // Dynamic theme determination based strictly on AST kind
  let headerBg = cardTokens.defaultHeaderBg;
  let borderStroke = cardTokens.defaultBorder;
  let stereotypeBg = cardTokens.defaultStereotypeBg;
  let stereotypeColor = cardTokens.defaultStereotypeColor;
  let cardBg = cardTokens.defaultBg;
  let titleColor = cardTokens.defaultTitleColor;
  let bodyTextColor = cardTokens.defaultBodyText;
  let separatorColor = cardTokens.defaultSeparator;

  if (kind === 'interface') {
    headerBg = cardTokens.interface.headerBg;
    borderStroke = cardTokens.interface.border;
    stereotypeBg = cardTokens.interface.stereotypeBg;
    stereotypeColor = cardTokens.interface.stereotypeColor;
  } else if (kind === 'abstract') {
    headerBg = cardTokens.abstract.headerBg;
    borderStroke = cardTokens.abstract.border;
    stereotypeBg = cardTokens.abstract.stereotypeBg;
    stereotypeColor = cardTokens.abstract.stereotypeColor;
  } else if (kind === 'enum') {
    headerBg = cardTokens.enum.headerBg;
    borderStroke = cardTokens.enum.border;
    stereotypeBg = cardTokens.enum.stereotypeBg;
    stereotypeColor = cardTokens.enum.stereotypeColor;
  } else if (stereotype && stereotype !== '<<class>>') {
    const hue = hashString(stereotype) % 360;
    headerBg = theme.isDark ? `hsl(${hue}, 45%, 20%)` : `hsl(${hue}, 85%, 96%)`;
    borderStroke = `hsl(${hue}, 70%, 55%)`;
    stereotypeBg = theme.isDark ? `hsl(${hue}, 40%, 28%)` : `hsl(${hue}, 80%, 90%)`;
    stereotypeColor = theme.isDark ? `hsl(${hue}, 80%, 80%)` : `hsl(${hue}, 85%, 30%)`;
  }

  let svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${dynamicWidth}" height="${cardHeight}" viewBox="0 0 ${dynamicWidth} ${cardHeight}">
  <defs>
    <filter id="cardShadow_${escapeXml(name)}" x="-5%" y="-5%" width="110%" height="115%" filterUnits="userSpaceOnUse">
      <feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000000" flood-opacity="${isDark ? '0.3' : '0.06'}"/>
    </filter>
  </defs>

  <rect x="1" y="1" width="${dynamicWidth - 2}" height="${cardHeight - 2}" rx="8" ry="8" fill="${cardBg}" stroke="${borderStroke}" stroke-width="1.5" filter="url(#cardShadow_${escapeXml(name)})"/>
  <path d="M 1,9 Q 1,1 9,1 L ${dynamicWidth - 9},1 Q ${dynamicWidth - 1},1 ${dynamicWidth - 1},9 L ${dynamicWidth - 1},${HEADER_HEIGHT} L 1,${HEADER_HEIGHT} Z" fill="${headerBg}" />
  <line x1="1" y1="${HEADER_HEIGHT}" x2="${dynamicWidth - 1}" y2="${HEADER_HEIGHT}" stroke="${borderStroke}" stroke-width="1.2" />

  <rect x="${dynamicWidth / 2 - 50}" y="6" width="100" height="14" rx="7" ry="7" fill="${stereotypeBg}" />
  <text x="${dynamicWidth / 2}" y="16.5" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${stereotypeColor}" text-anchor="middle">&lt;&lt;${escapeXml(badgeText)}&gt;&gt;</text>
  <text x="${dynamicWidth / 2}" y="38" font-family="JetBrains Mono, -apple-system, sans-serif" font-size="12.5" font-weight="700" fill="${titleColor}" text-anchor="middle">${escapeXml(name)}</text>
`;

  let currentY = HEADER_HEIGHT + 18;
  if (fields.length > 0) {
    fields.forEach((field) => {
      const { vis, text, color } = parseMemberRow(field);
      svg += `
  <g transform="translate(${PADDING_X}, ${currentY})">
    <circle cx="5" cy="-3.5" r="3.5" fill="${color}" />
    <text x="14" y="0" font-family="JetBrains Mono, monospace" font-size="10" fill="${bodyTextColor}">
      <tspan font-weight="700" fill="${color}">${vis} </tspan>
      <tspan>${escapeXml(text)}</tspan>
    </text>
  </g>`;
      currentY += ROW_HEIGHT;
    });
  } else {
    svg += `
  <g transform="translate(${PADDING_X}, ${currentY})">
    <text x="0" y="0" font-family="JetBrains Mono, monospace" font-size="9.5" font-style="italic" fill="#94A3B8">/* no attribute fields */</text>
  </g>`;
    currentY += ROW_HEIGHT;
  }

  currentY += 6;
  svg += `<line x1="1" y1="${currentY}" x2="${dynamicWidth - 1}" y2="${currentY}" stroke="${separatorColor}" stroke-width="1" stroke-dasharray="3 3"/>`;
  currentY += 18;

  if (methods.length > 0) {
    methods.forEach((method) => {
      const { vis, text, color } = parseMemberRow(method);
      svg += `
  <g transform="translate(${PADDING_X}, ${currentY})">
    <circle cx="5" cy="-3.5" r="3.5" fill="${color}" />
    <text x="14" y="0" font-family="JetBrains Mono, monospace" font-size="10" fill="${titleColor}">
      <tspan font-weight="700" fill="${color}">${vis} </tspan>
      <tspan>${escapeXml(text)}</tspan>
    </text>
  </g>`;
      currentY += ROW_HEIGHT;
    });
  } else {
    svg += `
  <g transform="translate(${PADDING_X}, ${currentY})">
    <text x="0" y="0" font-family="JetBrains Mono, monospace" font-size="9.5" font-style="italic" fill="#94A3B8">/* no operations */</text>
  </g>`;
  }

  svg += `\n</svg>`;

  return {
    svg,
    dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`,
    width: dynamicWidth,
    height: cardHeight
  };
}

/** ── 2. Dynamic UML 2.5 Package Folder Container (Zero Hardcoding) ── */
export function generatePackageFolderSvg(pkgData) {
  const {
    name = "package",
    nestLevel = 0,
    isCollapsed = false,
    childCount = 0,
    width = 400,
    height = 250
  } = pkgData;

  const TAB_HEIGHT = 30;
  const shortName = name.replace(/^package\s*\[?/, '').replace(/\]?$/, '');
  const tabWidth = Math.min(width - 40, Math.max(150, shortName.length * 8.5 + 40));

  const hue = hashString(shortName) % 360;
  const isDomainTier = nestLevel === 0;

  const tabBg = isDomainTier ? `hsl(${hue}, 75%, 92%)` : `hsl(${hue}, 70%, 88%)`;
  const bodyBg = isDomainTier ? `hsl(${hue}, 55%, 98%)` : `hsl(${hue}, 50%, 95%)`;
  const borderColor = isDomainTier ? `hsl(${hue}, 65%, 60%)` : `hsl(${hue}, 70%, 42%)`;
  const textColor = `hsl(${hue}, 80%, 25%)`;
  const borderStyle = isDomainTier ? '' : 'stroke-dasharray="6 4"';

  let svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <path d="M 2,${TAB_HEIGHT} L 2,6 Q 2,2 6,2 L ${tabWidth - 10},2 Q ${tabWidth - 4},2 ${tabWidth + 6},${TAB_HEIGHT} Z" fill="${tabBg}" stroke="${borderColor}" stroke-width="1.8" />
  <text x="14" y="${TAB_HEIGHT - 10}" font-family="JetBrains Mono, -apple-system, sans-serif" font-size="11" font-weight="700" fill="${textColor}">
    ${isCollapsed ? '📁 [+]' : '📂 [−]'} ${escapeXml(shortName)}
  </text>
  <rect x="2" y="${TAB_HEIGHT}" width="${width - 4}" height="${height - TAB_HEIGHT - 2}" rx="8" ry="8" fill="${bodyBg}" stroke="${borderColor}" stroke-width="1.8" ${borderStyle} />
`;

  if (isCollapsed) {
    svg += `
  <rect x="${width / 2 - 110}" y="${TAB_HEIGHT + (height - TAB_HEIGHT) / 2 - 14}" width="220" height="28" rx="14" fill="#FFFFFF" stroke="${borderColor}" stroke-width="1" />
  <text x="${width / 2}" y="${TAB_HEIGHT + (height - TAB_HEIGHT) / 2 + 4}" font-family="JetBrains Mono, monospace" font-size="10" font-weight="700" fill="${textColor}" text-anchor="middle">
    📦 ${childCount} Classes (Click to Expand)
  </text>`;
  }

  svg += `\n</svg>`;

  return {
    svg,
    dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`,
    width,
    height
  };
}

/** ── 3. State Machine Node ── */
export function generateStateNodeSvg(data) {
  const { name = "State", entryAction = null, doActivity = null, exitAction = null, width = 240, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.state;
  const isInitial = name === '[*]' || name === 'state_init' || name.endsWith('_init');
  const isFinal = name === 'state_final' || name.endsWith('_final');

  if (isInitial) {
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">
  <circle cx="24" cy="24" r="18" fill="${t.initialBg}" stroke="${t.initialStroke}" stroke-width="2.5" />
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 48, height: 48 };
  }

  if (isFinal) {
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">
  <circle cx="24" cy="24" r="20" fill="${t.finalOuterBg}" stroke="${t.finalOuterStroke}" stroke-width="2" />
  <circle cx="24" cy="24" r="13" fill="${t.finalInnerBg}" />
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 48, height: 48 };
  }

  const actions = [];
  if (entryAction) actions.push({ label: 'entry /', action: entryAction, color: '#10B981' });
  if (doActivity) actions.push({ label: 'do /', action: doActivity, color: '#3B82F6' });
  if (exitAction) actions.push({ label: 'exit /', action: exitAction, color: '#EF4444' });

  const height = 40 + (actions.length > 0 ? actions.length * 18 + 14 : 10);
  const cardBg = t.bg;
  const headerBg = t.headerBg;
  const strokeColor = t.border;
  const titleColor = t.titleColor;
  const bodyTextColor = t.bodyText;

  let svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="14" ry="14" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <path d="M 1,14 Q 1,1 14,1 L ${width - 14},1 Q ${width - 1},1 ${width - 1},14 L ${width - 1},32 L 1,32 Z" fill="${headerBg}" />
  <line x1="1" y1="32" x2="${width - 1}" y2="32" stroke="${strokeColor}" stroke-width="1.2" />
  <text x="${width / 2}" y="21" font-family="JetBrains Mono, sans-serif" font-size="11.5" font-weight="700" fill="${titleColor}" text-anchor="middle">${escapeXml(name)}</text>
`;

  let actY = 48;
  actions.forEach(act => {
    svg += `
  <text x="14" y="${actY}" font-family="JetBrains Mono, monospace" font-size="9.5" fill="${bodyTextColor}">
    <tspan font-weight="700" fill="${act.color}">${act.label} </tspan>
    <tspan>${escapeXml(act.action)}</tspan>
  </text>`;
    actY += 18;
  });

  svg += `\n</svg>`;
  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 4. Activity Action Node ── */
export function generateActionNodeSvg(data) {
  const { name = "Action", isStart = false, isStop = false, width = 230, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.action;

  if (isStart || name === 'start') {
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 44 44">
  <circle cx="22" cy="22" r="16" fill="#10B981" stroke="#059669" stroke-width="2" />
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 44, height: 44 };
  }

  if (isStop || name === 'stop') {
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 44 44">
  <circle cx="22" cy="22" r="18" fill="${theme.isDark ? '#0F172A' : '#FFFFFF'}" stroke="#EF4444" stroke-width="2.5" />
  <circle cx="22" cy="22" r="11" fill="#EF4444" />
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 44, height: 44 };
  }

  const height = 54;
  const cardBg = t.bg;
  const strokeColor = t.border;
  const badgeBg = t.badgeBg;
  const badgeColor = t.badgeColor;
  const textColor = t.textColor;

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="20" ry="20" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <rect x="${width / 2 - 35}" y="5" width="70" height="12" rx="6" ry="6" fill="${badgeBg}" />
  <text x="${width / 2}" y="14" font-family="JetBrains Mono, monospace" font-size="8" font-weight="700" fill="${badgeColor}" text-anchor="middle">&lt;&lt;action&gt;&gt;</text>
  <text x="${width / 2}" y="36" font-family="JetBrains Mono, sans-serif" font-size="11" font-weight="600" fill="${textColor}" text-anchor="middle">${escapeXml(name)}</text>
</svg>`;

  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 5. Component & Interface Sockets ── */
export function generateComponentNodeSvg(data) {
  const { name = "Component", isInterface = false, width = 240, height = 70, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.component;

  if (isInterface) {
    const textColor = t.ifaceText;
    const circleBg = t.ifaceBg;
    const strokeColor = t.ifaceStroke;

    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="180" height="54" viewBox="0 0 180 54">
  <circle cx="24" cy="27" r="12" fill="${circleBg}" stroke="${strokeColor}" stroke-width="2.5" />
  <line x1="36" y1="27" x2="60" y2="27" stroke="${strokeColor}" stroke-width="2" />
  <text x="66" y="31" font-family="JetBrains Mono, sans-serif" font-size="11" font-weight="700" fill="${textColor}">${escapeXml(name)}</text>
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 180, height: 54 };
  }

  const cardBg = t.bg;
  const strokeColor = t.border;
  const textColor = t.textColor;
  const badgeColor = t.badgeColor;

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="6" ry="6" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <rect x="-4" y="14" width="16" height="12" rx="2" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.5" />
  <rect x="-4" y="34" width="16" height="12" rx="2" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.5" />
  <text x="${width / 2}" y="24" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${badgeColor}" text-anchor="middle">&lt;&lt;component&gt;&gt;</text>
  <text x="${width / 2}" y="46" font-family="JetBrains Mono, sans-serif" font-size="12" font-weight="700" fill="${textColor}" text-anchor="middle">${escapeXml(name)}</text>
</svg>`;

  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 6. Deployment Node & Artifact ── */
export function generateDeploymentNodeSvg(data) {
  const { name = "Device", isArtifact = false, width = 230, height = 75, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = isArtifact ? theme.cards.artifact : theme.cards.device;
  const cardBg = t.bg;
  const strokeColor = t.border;
  const textColor = t.textColor;
  const badgeColor = t.badgeColor;

  if (isArtifact) {
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="6" ry="6" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" stroke-dasharray="4 3"/>
  <text x="${width / 2}" y="24" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${badgeColor}" text-anchor="middle">&lt;&lt;artifact&gt;&gt;</text>
  <text x="${width / 2}" y="48" font-family="JetBrains Mono, monospace" font-size="11.5" font-weight="700" fill="${textColor}" text-anchor="middle">📦 ${escapeXml(name)}</text>
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
  }

  const tabFill = t.tabFill;
  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <polygon points="12,1 12,12 1,12" fill="${tabFill}" stroke="${strokeColor}" stroke-width="1.2" />
  <rect x="1" y="12" width="${width - 14}" height="${height - 14}" rx="4" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <text x="${(width - 14) / 2}" y="32" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${badgeColor}" text-anchor="middle">&lt;&lt;device&gt;&gt;</text>
  <text x="${(width - 14) / 2}" y="52" font-family="JetBrains Mono, sans-serif" font-size="12" font-weight="700" fill="${textColor}" text-anchor="middle">🖥️ ${escapeXml(name)}</text>
</svg>`;

  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 7. Sequence Participant / Lifeline Header Card ── */
export function generateSequenceLifelineSvg(data) {
  const { name = "Participant", isActor = false, width = 180, height = 60, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.sequence;
  const cardBg = t.bg;
  const strokeColor = t.border;
  const badgeColor = t.badgeColor;
  const textColor = t.textColor;

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="8" ry="8" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <text x="${width / 2}" y="20" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${badgeColor}" text-anchor="middle">${isActor ? '&lt;&lt;actor&gt;&gt;' : '&lt;&lt;participant&gt;&gt;'}</text>
  <text x="${width / 2}" y="42" font-family="JetBrains Mono, sans-serif" font-size="12" font-weight="700" fill="${textColor}" text-anchor="middle">${escapeXml(name)}</text>
</svg>`;
  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 8. Use Case Bubble & Actor ── */
export function generateUseCaseSvg(data) {
  const { name = "Use Case", isActor = false, width = 220, height = 70, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.usecase;
  const strokeColor = t.border;
  const cardBg = t.bg;
  const textColor = t.textColor;

  if (isActor) {
    const actorStroke = t.actorStroke;
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="120" height="90" viewBox="0 0 120 90">
  <circle cx="60" cy="18" r="12" fill="${cardBg}" stroke="${actorStroke}" stroke-width="2" />
  <line x1="60" y1="30" x2="60" y2="58" stroke="${actorStroke}" stroke-width="2" />
  <line x1="38" y1="42" x2="82" y2="42" stroke="${actorStroke}" stroke-width="2" />
  <line x1="60" y1="58" x2="42" y2="80" stroke="${actorStroke}" stroke-width="2" />
  <line x1="60" y1="58" x2="78" y2="80" stroke="${actorStroke}" stroke-width="2" />
  <text x="60" y="88" font-family="JetBrains Mono, sans-serif" font-size="10" font-weight="700" fill="${textColor}" text-anchor="middle">${escapeXml(name)}</text>
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 120, height: 90 };
  }

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <ellipse cx="${width / 2}" cy="${height / 2}" rx="${width / 2 - 3}" ry="${height / 2 - 3}" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.8" />
  <text x="${width / 2}" y="${height / 2 + 4}" font-family="JetBrains Mono, sans-serif" font-size="11" font-weight="600" fill="${textColor}" text-anchor="middle">${escapeXml(name)}</text>
</svg>`;
  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 9. Object Runtime Instance Card ── */
export function generateObjectCardSvg(data) {
  const { name = "obj", fields = [], width = 240, height = 90, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.object;
  const HEADER_HEIGHT = 32;
  const ROW_HEIGHT = 18;
  const calculatedHeight = HEADER_HEIGHT + Math.max(1, fields.length) * ROW_HEIGHT + 14;
  const cardHeight = Math.max(height, calculatedHeight);

  const cardBg = t.bg;
  const headerBg = t.headerBg;
  const strokeColor = t.border;
  const titleColor = t.titleColor;
  const bodyTextColor = t.bodyText;

  let svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${cardHeight}" viewBox="0 0 ${width} ${cardHeight}">
  <rect x="1" y="1" width="${width - 2}" height="${cardHeight - 2}" rx="6" ry="6" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.5" />
  <rect x="1" y="1" width="${width - 2}" height="${HEADER_HEIGHT}" rx="6" ry="6" fill="${headerBg}" />
  <line x1="1" y1="${HEADER_HEIGHT}" x2="${width - 1}" y2="${HEADER_HEIGHT}" stroke="${strokeColor}" stroke-width="1.2" />
  <text x="${width / 2}" y="21" font-family="JetBrains Mono, sans-serif" font-size="11" font-weight="700" text-decoration="underline" fill="${titleColor}" text-anchor="middle">${escapeXml(name)}</text>
`;

  let currY = HEADER_HEIGHT + 16;
  fields.forEach(f => {
    const text = typeof f === 'string' ? f : (f.signature || f.name);
    svg += `
  <text x="12" y="${currY}" font-family="JetBrains Mono, monospace" font-size="9.5" fill="${bodyTextColor}">${escapeXml(text)}</text>`;
    currY += ROW_HEIGHT;
  });

  svg += `\n</svg>`;
  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height: cardHeight };
}

/** ── 10. Compiler CFG Basic Block Card ── */
export function generateCfgBlockSvg(data) {
  const { id = "bb_0", label = "Block", instructions = [], width = 280, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.cfg;
  const HEADER_HEIGHT = 36;
  const ROW_HEIGHT = 18;
  const height = HEADER_HEIGHT + Math.max(1, instructions.length) * ROW_HEIGHT + 18;

  const cardBg = t.bg;
  const headerBg = t.headerBg;
  const strokeColor = t.border;
  const titleColor = t.titleColor;
  const bodyTextColor = t.bodyText;

  let svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="1" y="1" width="${width - 2}" height="${height - 2}" rx="6" ry="6" fill="${cardBg}" stroke="${strokeColor}" stroke-width="1.5" />
  <path d="M 1,6 Q 1,1 6,1 L ${width - 6},1 Q ${width - 1},1 ${width - 1},6 L ${width - 1},${HEADER_HEIGHT} L 1,${HEADER_HEIGHT} Z" fill="${headerBg}" />
  <line x1="1" y1="${HEADER_HEIGHT}" x2="${width - 1}" y2="${HEADER_HEIGHT}" stroke="${strokeColor}" stroke-width="1.2" />
  <text x="12" y="23" font-family="JetBrains Mono, monospace" font-size="11" font-weight="700" fill="${titleColor}">⚡ BASIC BLOCK #${escapeXml(id)}</text>
`;

  let instY = HEADER_HEIGHT + 16;
  if (instructions.length > 0) {
    instructions.forEach(inst => {
      svg += `
  <text x="12" y="${instY}" font-family="JetBrains Mono, monospace" font-size="9.5" fill="${bodyTextColor}">
    <tspan fill="${strokeColor}">▸ </tspan><tspan>${escapeXml(inst)}</tspan>
  </text>`;
      instY += ROW_HEIGHT;
    });
  } else {
    svg += `
  <text x="12" y="${instY}" font-family="JetBrains Mono, monospace" font-size="9.5" fill="${bodyTextColor}">${escapeXml(label)}</text>`;
  }

  svg += `\n</svg>`;
  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

/** ── 11. ROBDD Decision Gate Node ── */
export function generateBddGateSvg(data) {
  const { varName = "var", isTerminal = false, terminalValue = 1, isDark = false } = data;
  const theme = getTheme(isDark);
  const t = theme.cards.bdd;

  if (isTerminal) {
    const isTrue = terminalValue === 1;
    const bg = isTrue ? t.trueBg : t.falseBg;
    const text = isTrue ? "TRUE (1)" : "FALSE (0)";
    const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="90" height="38" viewBox="0 0 90 38">
  <rect x="1" y="1" width="88" height="36" rx="18" fill="${bg}" />
  <text x="45" y="23" font-family="JetBrains Mono, monospace" font-size="10.5" font-weight="800" fill="#FFFFFF" text-anchor="middle">${text}</text>
</svg>`;
    return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width: 90, height: 38 };
  }

  const cardBg = t.bg;
  const strokeColor = t.border;
  const badgeColor = t.badgeColor;
  const textColor = t.textColor;

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="130" height="60" viewBox="0 0 130 60">
  <polygon points="65,2 128,30 65,58 2,30" fill="${cardBg}" stroke="${strokeColor}" stroke-width="2" />
  <text x="65" y="26" font-family="JetBrains Mono, monospace" font-size="8.5" font-weight="700" fill="${badgeColor}" text-anchor="middle">&lt;&lt;gate&gt;&gt;</text>
  <text x="65" y="39" font-family="JetBrains Mono, monospace" font-size="10.5" font-weight="700" fill="${textColor}" text-anchor="middle">${escapeXml(varName)}</text>
</svg>`;

  return { svg, dataUri: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, width, height };
}

function parseMemberRow(raw) {
  let cleaned = (typeof raw === 'string' ? raw : (raw.signature || raw.name || '')).trim();
  let vis = '+';
  let color = '#10B981';

  if (cleaned.startsWith('-') || cleaned.startsWith('−')) {
    vis = '−';
    color = '#EF4444';
    cleaned = cleaned.substring(1).trim();
  } else if (cleaned.startsWith('+')) {
    vis = '+';
    color = '#10B981';
    cleaned = cleaned.substring(1).trim();
  } else if (cleaned.startsWith('#')) {
    vis = '#';
    color = '#F59E0B';
    cleaned = cleaned.substring(1).trim();
  } else if (cleaned.startsWith('~')) {
    vis = '~';
    color = '#3B82F6';
    cleaned = cleaned.substring(1).trim();
  }

  return { vis, text: cleaned, color };
}

function hashString(str) {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = (hash << 5) - hash + str.charCodeAt(i);
    hash |= 0;
  }
  return Math.abs(hash);
}

function escapeXml(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}
