import { createSignal } from "solid-js";

const [paletteOpen, setPaletteOpen] = createSignal(false);
export { paletteOpen };

export const openPalette = () => setPaletteOpen(true);
export const closePalette = () => setPaletteOpen(false);
export const togglePalette = () => setPaletteOpen((v) => !v);
