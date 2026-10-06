/**
 * 统一线描图标系统。
 * 规格：24×24 viewBox，stroke=currentColor，strokeWidth 1.5，圆头圆角。
 * 与 aurora-lens 同一套规格，两个应用的图标视觉重量一致。
 */

function Svg({ size = 18, children, ...rest }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {children}
    </svg>
  );
}

/* ---------- 品牌 ---------- */

export const LogoMark = (p) => (
  <Svg viewBox="0 0 24 24" size={20} {...p}>
    <path d="M7 9.5h8.5M13 6.5l3 3-3 3" strokeWidth="1.7" />
    <path d="M17 14.5H8.5M11 11.5l-3 3 3 3" strokeWidth="1.7" />
  </Svg>
);

/* ---------- 界面 ---------- */

export const IconPlus = (p) => (
  <Svg {...p}><path d="M12 6v12M6 12h12" /></Svg>
);

export const IconClose = (p) => (
  <Svg {...p}><path d="M7 7l10 10M17 7 7 17" /></Svg>
);

export const IconFolder = (p) => (
  <Svg {...p}>
    <path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2.5h7A1.5 1.5 0 0 1 19 10v7a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17Z" />
  </Svg>
);

export const IconFolderOpen = (p) => (
  <Svg {...p}>
    <path d="M3 8.5A1.5 1.5 0 0 1 4.5 7h3.8l1.7 2.2h6A1.5 1.5 0 0 1 17.5 11v.5" />
    <path d="M3 8.5V17a1.5 1.5 0 0 0 1.5 1.5h11.2a1.5 1.5 0 0 0 1.44-1.08l1.6-5A1.5 1.5 0 0 0 17.28 10H6.2a1.5 1.5 0 0 0-1.43 1.05L3.6 14.5" />
  </Svg>
);

export const IconFile = (p) => (
  <Svg {...p}>
    <path d="M13.5 3H7a1.5 1.5 0 0 0-1.5 1.5v15A1.5 1.5 0 0 0 7 21h10a1.5 1.5 0 0 0 1.5-1.5V8Z" />
    <path d="M13.5 3v5h5" />
  </Svg>
);

export const IconTrash = (p) => (
  <Svg {...p}>
    <path d="M4 7h16M9.5 7V5.5A1.5 1.5 0 0 1 11 4h2a1.5 1.5 0 0 1 1.5 1.5V7" />
    <path d="M6.5 7l.8 11.6A1.5 1.5 0 0 0 8.8 20h6.4a1.5 1.5 0 0 0 1.5-1.4L17.5 7" />
  </Svg>
);

export const IconArrowRight = (p) => (
  <Svg {...p}><path d="M4.5 12h15M14 6.5l5.5 5.5-5.5 5.5" /></Svg>
);

export const IconCheck = (p) => (
  <Svg {...p} strokeWidth="2"><path d="M5 12.5l4.5 4.5L19 7" /></Svg>
);

export const IconAlert = (p) => (
  <Svg {...p}>
    <path d="M12 4.5 3 19.5h18Z" />
    <path d="M12 10v4M12 17h.01" />
  </Svg>
);

export const IconLock = (p) => (
  <Svg {...p}>
    <rect x="5" y="10.5" width="14" height="9.5" rx="2" />
    <path d="M8.5 10.5V8a3.5 3.5 0 1 1 7 0v2.5" />
  </Svg>
);

export const IconSparkle = (p) => (
  <Svg {...p}>
    <path d="M12 4v3.5M12 16.5V20M4 12h3.5M16.5 12H20" />
    <path d="M6.7 6.7l2.4 2.4M14.9 14.9l2.4 2.4M17.3 6.7l-2.4 2.4M9.1 14.9l-2.4 2.4" />
  </Svg>
);

export const IconLayers = (p) => (
  <Svg {...p}>
    <path d="M12 3.5 3.5 8 12 12.5 20.5 8Z" />
    <path d="M3.5 12.5 12 17l8.5-4.5" />
    <path d="M3.5 16.5 12 21l8.5-4.5" />
  </Svg>
);

export const IconSettings = (p) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2.8v2.4M12 18.8v2.4M21.2 12h-2.4M5.2 12H2.8M18.5 5.5l-1.7 1.7M7.2 16.8l-1.7 1.7M18.5 18.5l-1.7-1.7M7.2 7.2 5.5 5.5" />
  </Svg>
);

export const IconShield = (p) => (
  <Svg {...p}>
    <path d="M12 3 5 5.8v5.4c0 4.3 2.9 8.1 7 9.8 4.1-1.7 7-5.5 7-9.8V5.8Z" />
    <path d="M9.2 12l1.9 1.9 3.7-3.8" />
  </Svg>
);

export const IconStop = (p) => (
  <Svg {...p}><rect x="6.5" y="6.5" width="11" height="11" rx="2" /></Svg>
);

export const IconRetry = (p) => (
  <Svg {...p}>
    <path d="M20 12a8 8 0 1 1-2.6-5.9" />
    <path d="M20 4v4.5h-4.5" />
  </Svg>
);

export const IconRefresh = (p) => (
  <Svg {...p}>
    <path d="M20.5 12a8.5 8.5 0 1 1-2.5-6" />
    <path d="M20.5 3.5V9H15" />
  </Svg>
);

export const IconDownload = (p) => (
  <Svg {...p}>
    <path d="M12 4v11M7.5 10.5 12 15l4.5-4.5" />
    <path d="M5 18.5h14" />
  </Svg>
);

export const IconChevronDown = (p) => (
  <Svg {...p}><path d="M6.5 9.5 12 15l5.5-5.5" /></Svg>
);

export const IconChevronRight = (p) => (
  <Svg {...p}><path d="M9.5 6 15 12l-5.5 6" /></Svg>
);
