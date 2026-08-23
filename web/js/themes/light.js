/**
 * OpenHeart Light Theme JavaScript Definition
 * Pure tokens and styling parameters for Cytoscape and SVG Vector Cards.
 */
export const LightTheme = {
  name: 'light',
  isDark: false,

  canvas: {
    background: '#FAFCFE',
    gridDot: '#CBD5E1',
    gridLine: 'rgba(0, 0, 0, 0.02)',
    gridSize: '24px'
  },

  packages: {
    tier0: {
      bg: '#FAF5FF',
      opacity: 0.85,
      borderWidth: 2.5,
      borderColor: '#C084FC',
      borderStyle: 'solid',
      borderRadius: '14px',
      textBg: '#FFFFFF',
      textBorder: '#C084FC',
      textColor: '#6B21A8'
    },
    tier1: {
      bg: '#F3E8FF',
      opacity: 0.90,
      borderWidth: 2.2,
      borderColor: '#A855F7',
      borderStyle: 'solid',
      borderRadius: '12px',
      textBg: '#FAF5FF',
      textBorder: '#A855F7',
      textColor: '#581C87'
    },
    tier2: {
      bg: '#E9D5FF',
      opacity: 0.95,
      borderWidth: 2.0,
      borderColor: '#9333EA',
      borderStyle: 'solid',
      borderRadius: '10px',
      textBg: '#F3E8FF',
      textBorder: '#9333EA',
      textColor: '#4C1D95'
    },
    tier3: {
      bg: '#DDD6FE',
      opacity: 1.0,
      borderWidth: 2.2,
      borderColor: '#7E22CE',
      borderStyle: 'solid',
      borderRadius: '8px',
      textBg: '#EDE9FE',
      textBorder: '#7E22CE',
      textColor: '#3B0764'
    },
    tier4: {
      bg: '#C4B5FD',
      opacity: 1.0,
      borderWidth: 2.5,
      borderColor: '#6B21A8',
      borderStyle: 'solid',
      borderRadius: '8px',
      textBg: '#DDD6FE',
      textBorder: '#6B21A8',
      textColor: '#2E1065'
    },
    fallback: {
      bg: '#F8FAFC',
      opacity: 0.90,
      borderWidth: 2.0,
      borderColor: '#475569',
      borderStyle: 'solid',
      borderRadius: '10px',
      textBg: '#FFFFFF',
      textBorder: '#475569',
      textColor: '#1E293B'
    }
  },

  edges: {
    defaultLine: '#64748B',
    defaultArrow: '#64748B',
    defaultLabelBg: '#FFFFFF',
    defaultLabelText: '#1E293B',
    defaultLabelBorder: '#CBD5E1',
    generalization: '#8B5CF6',
    realization: '#0284C7',
    composition: '#DC2626',
    aggregation: '#F59E0B',
    association: '#2563EB',
    dependency: '#64748B',
    containment: '#475569'
  },

  cards: {
    cardShadowOpacity: 0.06,
    defaultBg: '#FFFFFF',
    defaultHeaderBg: '#F8FAFC',
    defaultBorder: '#475569',
    defaultStereotypeBg: '#F1F5F9',
    defaultStereotypeColor: '#475569',
    defaultTitleColor: '#0F172A',
    defaultBodyText: '#334155',
    defaultSeparator: '#E2E8F0',

    interface: {
      headerBg: '#EFF6FF',
      border: '#2563EB',
      stereotypeBg: '#DBEAFE',
      stereotypeColor: '#1D4ED8'
    },
    abstract: {
      headerBg: '#FAF5FF',
      border: '#7C3AED',
      stereotypeBg: '#F3E8FF',
      stereotypeColor: '#7E22CE'
    },
    enum: {
      headerBg: '#FFFBEB',
      border: '#D97706',
      stereotypeBg: '#FEF3C7',
      stereotypeColor: '#B45309'
    },
    state: {
      bg: '#FFFFFF',
      headerBg: '#F0F9FF',
      border: '#0284C7',
      titleColor: '#0369A1',
      bodyText: '#334155',
      initialBg: '#0F172A',
      initialStroke: '#38BDF8',
      finalOuterBg: '#FFFFFF',
      finalOuterStroke: '#0F172A',
      finalInnerBg: '#0F172A'
    },
    action: {
      bg: '#FFFFFF',
      border: '#6366F1',
      badgeBg: '#EEF2FF',
      badgeColor: '#4F46E5',
      textColor: '#1E293B'
    },
    component: {
      bg: '#FFFFFF',
      border: '#059669',
      badgeColor: '#059669',
      textColor: '#0F172A',
      ifaceBg: '#FFFFFF',
      ifaceStroke: '#2563EB',
      ifaceText: '#1E293B'
    },
    device: {
      bg: '#FFFFFF',
      border: '#64748B',
      tabFill: '#F1F5F9',
      badgeColor: '#475569',
      textColor: '#0F172A'
    },
    artifact: {
      bg: '#FFFFFF',
      border: '#D97706',
      badgeColor: '#B45309',
      textColor: '#0F172A'
    },
    sequence: {
      bg: '#F8FAFC',
      border: '#4F46E5',
      badgeColor: '#4F46E5',
      textColor: '#0F172A'
    },
    usecase: {
      bg: '#FFFFFF',
      border: '#2563EB',
      textColor: '#1E293B',
      actorStroke: '#0F172A',
      actorText: '#0F172A'
    },
    object: {
      bg: '#FFFFFF',
      headerBg: '#F0F9FF',
      border: '#0284C7',
      titleColor: '#0369A1',
      bodyText: '#334155'
    },
    cfg: {
      bg: '#FFFFFF',
      headerBg: '#FEF2F2',
      border: '#DC2626',
      titleColor: '#991B1B',
      bodyText: '#334155'
    },
    bdd: {
      bg: '#FFFFFF',
      border: '#3B82F6',
      badgeColor: '#3B82F6',
      textColor: '#0F172A',
      trueBg: '#10B981',
      falseBg: '#EF4444'
    }
  }
};
