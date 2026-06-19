import { createSignal } from "solid-js";

/** The three brand palettes from the Conduit design kit. Amber (CRT phosphor)
 *  is the default; mono is the original 1-bit black/white; paper is light. */
export type Theme = "amber" | "mono" | "paper";

const ORDER: Theme[] = ["amber", "mono", "paper"];

function isTheme(v: unknown): v is Theme {
  return v === "amber" || v === "mono" || v === "paper";
}

function initial(): Theme {
  try {
    const saved = localStorage.getItem("conduit.theme");
    if (isTheme(saved)) return saved;
    // Anything else — no preference, or a stale "dark"/"light" from the old
    // toggle — falls through to the amber default.
  } catch {
    // ignore
  }
  return "amber";
}

const [theme, setTheme] = createSignal<Theme>(initial());
export { theme };

function apply(t: Theme): void {
  document.documentElement.setAttribute("data-theme", t);
}

// Set the attribute immediately on load to avoid a flash of the wrong theme.
apply(theme());

/** Advance to the next palette: amber → mono → paper → amber. */
export function cycleTheme(): void {
  const next = ORDER[(ORDER.indexOf(theme()) + 1) % ORDER.length]!;
  setTheme(next);
  apply(next);
  try {
    localStorage.setItem("conduit.theme", next);
  } catch {
    // ignore
  }
}
