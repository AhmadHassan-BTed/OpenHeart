/**
 * OpenHeart Dark Theme JavaScript Definition
 * Pure tokens and styling parameters for Cytoscape and SVG Vector Cards.
 */
export const DarkTheme = {
  name: 'dark',
  isDark: true,

  canvas: {
    background: '#0B0F19',
    gridDot: 'rgba(255, 255, 255, 0.12)',
    gridLine: 'rgba(255, 255, 255, 0.03)',
    gridSize: '24px'
  },

  packages: {
    tier0: {
      bg: '#13111C',
      opacity: 0.90,
      borderWidth: 2.5,
      borderColor: '#9333EA',
      borderStyle: 'solid',
      borderRadius: '14px',
      textBg: '#1E1B4B',
      textBorder: '#9333EA',
      textColor: '#E9D5FF'
    },
    tier1: {
      bg: '#1A1829',
      opacity: 0.92,
      borderWidth: 2.2,
      borderColor: '#A855F7',
      borderStyle: 'dashed',
      borderRadius: '12px',
      textBg: '#2E1065',
      textBorder: '#A855F7',
      textColor: '#F3E8FF'
    },
    tier2: {
      bg: '#221F38',
      opacity: 0.94,
      borderWidth: 2.0,
      borderColor: '#C084FC',
      borderStyle: 'dashed',
      borderRadius: '10px',
      textBg: '#3B0764',
      textBorder: '#C084FC',
      textColor: '#FAF5FF'
    },
    tier3: {
      bg: '#2B264A',
      opacity: 0.96,
      borderWidth: 2.2,
      borderColor: '#D8B4FE',
      borderStyle: 'solid',
      borderRadius: '8px',
      textBg: '#4C1D95',
      textBorder: '#D8B4FE',
      textColor: '#FFFFFF'
    },
    tier4: {
      bg: '#342D5C',
      opacity: 0.98,
      borderWidth: 2.5,
      borderColor: '#E9D5FF',
      borderStyle: 'solid',
      borderRadius: '8px',
      textBg: '#581C87',
      textBorder: '#E9D5FF',
      textColor: '#FFFFFF'
    },
    fallback: {
      bg: '#111827',
      opacity: 0.90,
      borderWidth: 2.0,
      borderColor: '#64748B',
      borderStyle: 'dashed',
      borderRadius: '10px',
      textBg: '#1F2937',
      textBorder: '#64748B',
      textColor: '#F3F4F6'
    }
  },

  edges: {
    defaultLine: '#94A3B8',
    defaultArrow: '#94A3B8',
    defaultLabelBg: '#1E293B',
    defaultLabelText: '#F8FAFC',
    defaultLabelBorder: '#475569',
    generalization: '#A78BFA',
    realization: '#38BDF8',
    composition: '#F87171',
    aggregation: '#FBBF24',
    association: '#60A5FA',
    dependency: '#94A3B8',
    containment: '#64748B'
  },

  cards: {
    cardShadowOpacity: 0.35,
    defaultBg: '#0F172A',
    defaultHeaderBg: '#1E293B',
    defaultBorder: '#64748B',
    defaultStereotypeBg: '#334155',
    defaultStereotypeColor: '#94A3B8',
    defaultTitleColor: '#F8FAFC',
    defaultBodyText: '#CBD5E1',
    defaultSeparator: '#334155',

    interface: {
      headerBg: '#1E3A8A',
      border: '#60A5FA',
      stereotypeBg: '#1E40AF',
      stereotypeColor: '#93C5FD'
    },
    abstract: {
      headerBg: '#581C87',
      border: '#C084FC',
      stereotypeBg: '#6B21A8',
      stereotypeColor: '#E9D5FF'
    },
    enum: {
      headerBg: '#78350F',
      border: '#FBBF24',
      stereotypeBg: '#92400E',
      stereotypeColor: '#FDE68A'
    },
    state: {
      bg: '#0F172A',
      headerBg: '#075985',
      border: '#38BDF8',
      titleColor: '#F0F9FF',
      bodyText: '#CBD5E1',
      initialBg: '#38BDF8',
      initialStroke: '#0284C7',
      finalOuterBg: '#0F172A',
      finalOuterStroke: '#38BDF8',
      finalInnerBg: '#38BDF8'
    },
    action: {
      bg: '#1E1B4B',
      border: '#818CF8',
      badgeBg: '#312E81',
      badgeColor: '#C7D2FE',
      textColor: '#F8FAFC'
    },
    component: {
      bg: '#064E3B',
      border: '#34D399',
      badgeColor: '#6EE7B7',
      textColor: '#F8FAFC',
      ifaceBg: '#1E293B',
      ifaceStroke: '#60A5FA',
      ifaceText: '#F8FAFC'
    },
    device: {
      bg: '#1E293B',
      border: '#94A3B8',
      tabFill: '#334155',
      badgeColor: '#CBD5E1',
      textColor: '#F8FAFC'
    },
    artifact: {
      bg: '#1E293B',
      border: '#FBBF24',
      badgeColor: '#FDE68A',
      textColor: '#F8FAFC'
    },
    sequence: {
      bg: '#1E1B4B',
      border: '#818CF8',
      badgeColor: '#C7D2FE',
      textColor: '#F8FAFC'
    },
    usecase: {
      bg: '#1E293B',
      border: '#60A5FA',
      textColor: '#F8FAFC',
      actorStroke: '#F8FAFC',
      actorText: '#F8FAFC'
    },
    object: {
      bg: '#0F172A',
      headerBg: '#075985',
      border: '#38BDF8',
      titleColor: '#E0F2FE',
      bodyText: '#CBD5E1'
    },
    cfg: {
      bg: '#1F1F2E',
      headerBg: '#450A0A',
      border: '#F87171',
      titleColor: '#FECACA',
      bodyText: '#E2E8F0'
    },
    bdd: {
      bg: '#1E3A8A',
      border: '#60A5FA',
      badgeColor: '#93C5FD',
      textColor: '#F8FAFC',
      trueBg: '#10B981',
      falseBg: '#EF4444'
    }
  }
};
