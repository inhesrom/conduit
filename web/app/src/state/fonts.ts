/** Selectable fonts for the Fonts settings section.
 *
 * Every non-system font here is bundled via an @fontsource import in index.tsx,
 * so the choice works regardless of what's installed on the user's machine. An
 * entry's `css` is the full family stack (with fallbacks) pushed into a CSS var
 * or xterm's fontFamily; `primary` is the exact registered family name, used to
 * warm the woff2 via document.fonts.load() before it's measured (matters for the
 * terminal, where a not-yet-loaded font mis-measures cell width — see
 * terminal.ts). Terminal and diff slots only accept `mono` fonts. */
export interface FontOption {
  id: string;
  label: string;
  css: string;
  /** Exact family name to pass to document.fonts.load(); null for system stacks. */
  primary: string | null;
  kind: "mono" | "sans";
}

const MONO_FALLBACK = `ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace`;
const SANS_FALLBACK = `ui-sans-serif, system-ui, sans-serif`;

export const MONO_FONTS: FontOption[] = [
  {
    id: "jetbrains-mono",
    label: "JetBrains Mono",
    css: `"JetBrains Mono Variable", "JetBrains Mono", ${MONO_FALLBACK}`,
    primary: "JetBrains Mono Variable",
    kind: "mono",
  },
  {
    id: "fira-code",
    label: "Fira Code",
    css: `"Fira Code Variable", "Fira Code", ${MONO_FALLBACK}`,
    primary: "Fira Code Variable",
    kind: "mono",
  },
  {
    id: "cascadia-code",
    label: "Cascadia Code",
    css: `"Cascadia Code Variable", "Cascadia Code", ${MONO_FALLBACK}`,
    primary: "Cascadia Code Variable",
    kind: "mono",
  },
  {
    id: "ibm-plex-mono",
    label: "IBM Plex Mono",
    css: `"IBM Plex Mono", ${MONO_FALLBACK}`,
    primary: "IBM Plex Mono",
    kind: "mono",
  },
  {
    id: "system-mono",
    label: "System default",
    css: MONO_FALLBACK,
    primary: null,
    kind: "mono",
  },
];

export const SANS_FONTS: FontOption[] = [
  {
    id: "pixelify-sans",
    label: "Pixelify Sans",
    css: `"Pixelify Sans Variable", ${SANS_FALLBACK}`,
    primary: "Pixelify Sans Variable",
    kind: "sans",
  },
  {
    id: "inter",
    label: "Inter",
    css: `"Inter Variable", "Inter", ${SANS_FALLBACK}`,
    primary: "Inter Variable",
    kind: "sans",
  },
  {
    id: "system-ui",
    label: "System default",
    css: SANS_FALLBACK,
    primary: null,
    kind: "sans",
  },
];

const BY_ID = new Map<string, FontOption>([...MONO_FONTS, ...SANS_FONTS].map((f) => [f.id, f]));

/** Look up a font option by id, or undefined for an unknown id. */
export function fontById(id: string): FontOption | undefined {
  return BY_ID.get(id);
}

/** The CSS family stack for a font id. Falls back to the mono stack for an
 * unknown id so a stale/garbage setting never blanks out font-family. */
export function fontCss(id: string): string {
  return BY_ID.get(id)?.css ?? MONO_FALLBACK;
}
