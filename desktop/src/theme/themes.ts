// Theme Engine. Built-in themes mirror the JSON theme definition from the
// build plan; applying a theme swaps the <html> class and CSS variables so the
// whole UI re-skins instantly. Community / user themes (v1.0) are stored in
// localStorage in the same shape and applied through a generic "theme-custom"
// class that takes its colours from inline CSS variables — so a user can author
// or import a theme without shipping new CSS.

export interface ArcadiaTheme {
  id: string;
  name: string;
  description: string;
  className: string;
  primary: string; // hex, for swatches + custom inline vars
  secondary: string;
  scanlines: boolean;
  crtGlow: boolean;
  glassOpacity: number;
  /** True for user-authored/imported themes (persisted in localStorage). */
  custom?: boolean;
}

const CUSTOM_KEY = "arcadia.customThemes";
const CUSTOM_CLASS = "theme-custom";

export const THEMES: ArcadiaTheme[] = [
  {
    id: "dreamglass",
    name: "DreamGlass",
    description: "Heavy Y2K. Translucent plastic, frosted glass, neon edges.",
    className: "theme-dreamglass",
    primary: "#00D7FF",
    secondary: "#FF4DD2",
    scanlines: false,
    crtGlow: true,
    glassOpacity: 0.6,
  },
  {
    id: "afterglow",
    name: "Afterglow",
    description: "CRT-heavy. Warm phosphor bloom and scanlines.",
    className: "theme-afterglow",
    primary: "#FF8A4C",
    secondary: "#785AFF",
    scanlines: true,
    crtGlow: true,
    glassOpacity: 0.55,
  },
  {
    id: "arcadia-green",
    name: "Arcadia Green",
    description: "Classic monitor aesthetic. Green phosphor, low chrome.",
    className: "theme-arcadia-green",
    primary: "#40FF94",
    secondary: "#20C878",
    scanlines: true,
    crtGlow: true,
    glassOpacity: 0.5,
  },
];

/** "#00D7FF" -> "0 215 255" for the space-separated rgb() CSS variables. */
function hexToRgbTriple(hex: string): string | null {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
}

export function loadCustomThemes(): ArcadiaTheme[] {
  try {
    const raw = localStorage.getItem(CUSTOM_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as ArcadiaTheme[];
    return Array.isArray(parsed)
      ? parsed.map((t) => ({ ...t, custom: true, className: CUSTOM_CLASS }))
      : [];
  } catch {
    return [];
  }
}

function persistCustomThemes(themes: ArcadiaTheme[]) {
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(themes));
}

/** Built-in themes followed by user/community themes. */
export function allThemes(): ArcadiaTheme[] {
  return [...THEMES, ...loadCustomThemes()];
}

/** Create or update a custom theme (keyed by id). Returns the saved theme. */
export function saveCustomTheme(theme: ArcadiaTheme): ArcadiaTheme {
  const saved: ArcadiaTheme = {
    ...theme,
    custom: true,
    className: CUSTOM_CLASS,
    id: theme.id?.trim() || `custom-${Date.now()}`,
  };
  const others = loadCustomThemes().filter((t) => t.id !== saved.id);
  persistCustomThemes([...others, saved]);
  return saved;
}

export function deleteCustomTheme(id: string) {
  persistCustomThemes(loadCustomThemes().filter((t) => t.id !== id));
}

export function applyTheme(theme: ArcadiaTheme) {
  const html = document.documentElement;
  for (const t of THEMES) html.classList.remove(t.className);
  html.classList.remove(CUSTOM_CLASS);

  if (theme.custom) {
    // Custom themes have no dedicated stylesheet: drive accent colours from
    // inline variables on top of the default dark surface palette.
    html.classList.add(CUSTOM_CLASS);
    const primary = hexToRgbTriple(theme.primary);
    const secondary = hexToRgbTriple(theme.secondary);
    if (primary) html.style.setProperty("--arc-primary", primary);
    if (secondary) html.style.setProperty("--arc-secondary", secondary);
  } else {
    // Built-in themes set their palette via the class; clear any inline accent
    // overrides a previously-applied custom theme may have left behind.
    html.classList.add(theme.className);
    html.style.removeProperty("--arc-primary");
    html.style.removeProperty("--arc-secondary");
  }

  html.style.setProperty("--arc-glass-opacity", String(theme.glassOpacity));
  html.style.setProperty("--arc-scanlines", theme.scanlines ? "1" : "0");
  html.style.setProperty("--arc-crt-glow", theme.crtGlow ? "1" : "0");
}

export function themeById(id: string): ArcadiaTheme {
  return allThemes().find((t) => t.id === id) ?? THEMES[0];
}
