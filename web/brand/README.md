# Conduit — logo & brand assets

Drop-in assets for the Conduit app. Mark: two parallel conduits with flowing
packets, 8-bit dithered tube walls. Three palettes — **Mono**, **Amber**, **Paper**.

## Folders
- `assets/mono/`, `assets/amber/`, `assets/paper/` — per-palette art:
  - `conduit-{theme}.svg` — vector, transparent, scales to any size
  - `conduit-{theme}-icon.svg` — vector, rounded, baked background
  - `conduit-{theme}-{512…16}.png` — transparent raster
  - `conduit-{theme}-icon-{1024,512,180,120}.png` — baked app icons
  - `conduit-{theme}-icon-rounded-512.png` — rounded preview
  - `conduit-{theme}-loop.svg` — animated README loop (`<img src=…>`)
  - `conduit-{theme}-compact.{svg,png}` — 1-pipe variant for tight spaces
  - `favicon-{theme}-{32,16}.png`
- `assets/web/` — favicon.ico (+ per-theme), PWA icons, maskable, apple-touch,
  `site.webmanifest`, `web-head.html` (copy-paste `<link>` tags)

## Color schemes (reusable)
- `tokens/colors.css` — every scheme as `--cd-*` variables under `[data-theme]`.
  Retheme by setting `data-theme="amber"` etc.
- `palettes.json` — machine-readable catalog (same role contract).

## Regenerating
`logo.js` is the single source of truth (the pixel engine). All PNG/SVG/ICO
assets were generated from it.

## README usage example
```html
<p align="center"><img src="assets/mono/conduit-mono-loop.svg" width="160" alt="Conduit"></p>
```

Source context: https://github.com/inhesrom/conduit
