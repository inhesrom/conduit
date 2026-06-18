import { createSignal } from "solid-js";

/** Light/dark is a 1-bit swap of ink and paper. */
export type Theme = "dark" | "light";

function initial(): Theme {
  try {
    const saved = localStorage.getItem("conduit.theme");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // ignore
  }
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

const [theme, setTheme] = createSignal<Theme>(initial());
export { theme };

function apply(t: Theme): void {
  document.documentElement.setAttribute("data-theme", t);
}

// Set the attribute immediately on load to avoid a flash of the wrong theme.
apply(theme());

export function toggleTheme(): void {
  const next: Theme = theme() === "dark" ? "light" : "dark";
  setTheme(next);
  apply(next);
  try {
    localStorage.setItem("conduit.theme", next);
  } catch {
    // ignore
  }
}
