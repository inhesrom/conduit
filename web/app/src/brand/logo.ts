/* Conduit — 8-bit pixel logo engine (double-pipe, themeable, conduit cues).
 *
 * Ported from the design kit's `logo.js` (web/brand/logo.js), the single source
 * of truth that generated every PNG/SVG/ICO asset. Kept faithful so the live,
 * animated in-app mark stays pixel-identical to the static brand assets.
 *
 * Parallel straight conduits with shaded + DITHERED tube walls (top-lit
 * highlight -> dark shadow edge) and bright packets flowing through. The
 * `phase` option (0..1) advances the packets; animate it for flow. Native art
 * grid is 32x32 logical pixels; clean 2:1/4:1 downscale to every icon size.
 */

export type ThemeName = "mono" | "amber" | "signal" | "paper";

/** A 2-tone checkerboard dither; `d[(x+y)&1]` picks the tone for a pixel. */
export interface Dither {
  d: [string, string];
}

/** Packet colors for one lane: bright core + dimmer glow. */
export interface Lane {
  core: string;
  glow: string;
}

export interface Theme {
  label: string;
  sub: string;
  frameBg: string;
  iconBg: string;
  /** Channel interior fill; `null` leaves it transparent (paper). */
  intr: string | null;
  hi: string;
  lite: Dither;
  drk: Dither;
  edge: string;
  collar: string;
  node: string;
  lanes: Lane[];
}

export interface BuildOpts {
  /** A theme name (resolved against THEMES) or a Theme object. Default "mono". */
  theme?: ThemeName | Theme;
  orient?: "h" | "v";
  lanes?: number;
  band?: number;
  /** Fractional positions (0..1) for coupling collars. */
  coupling?: number[] | null;
  nodes?: boolean;
  /** Packet flow position, 0..1. Advance over time to animate. */
  phase?: number;
  /** Draw the flowing packets (default true). */
  packets?: boolean;
}

export interface DrawOpts extends BuildOpts {
  /** Solid background fill; omit to clear (transparent). */
  bg?: string;
}

type Orient = "h" | "v";
type Cell = string | null;
type RampEntry = string | Dither;

export const N = 32; // native grid; halves cleanly to 16, quarters to 8, etc.
const G = 2; // gap between parallel lanes
const COREW = 3; // packet core width along the flow

// Themes carry TONES so tube thickness is adjustable:
//   hi=top highlight, lite/drk=dithered body tones, edge=shadow edge,
//   intr=channel interior, lanes[]=packet {core,glow}, collar/node accents.
export const THEMES: Record<ThemeName, Theme> = {
  mono: {
    label: "Mono", sub: "white + black, 1-bit grain",
    frameBg: "#000000", iconBg: "#000000", intr: "#0c0e11",
    hi: "#ffffff", lite: { d: ["#d4d7da", "#9aa0a6"] }, drk: { d: ["#5b6166", "#383d42"] }, edge: "#24282c",
    collar: "#ffffff", node: "#e8eaed",
    lanes: [{ core: "#ffffff", glow: "#aab0b5" }, { core: "#b9bec2", glow: "#6b7176" }],
  },
  amber: {
    label: "Amber", sub: "CRT phosphor warmth",
    frameBg: "#0b0704", iconBg: "#120c05", intr: "#140d05",
    hi: "#ffd591", lite: { d: ["#d99a4e", "#9c6a2c"] }, drk: { d: ["#8a5e27", "#4d3415"] }, edge: "#36240f",
    collar: "#ffe0a6", node: "#ffce7a",
    lanes: [{ core: "#ffce7a", glow: "#c98a3a" }, { core: "#ff9f4d", glow: "#b5641f" }],
  },
  signal: {
    label: "Signal", sub: "cyan + magenta (brand)",
    frameBg: "#0a0d12", iconBg: "#0d1117", intr: "#08181c",
    hi: "#8af0f6", lite: { d: ["#3fbcc3", "#1f868c"] }, drk: { d: ["#1c787e", "#0f3d41"] }, edge: "#0b2a2e",
    collar: "#aef6fb", node: "#5ef0f7",
    lanes: [{ core: "#5ef0f7", glow: "#1fb6c2" }, { core: "#ff6fde", glow: "#c23aa0" }],
  },
  paper: {
    label: "Paper", sub: "inverted — black on white",
    frameBg: "#ffffff", iconBg: "#ffffff", intr: null,
    hi: "#454c54", lite: { d: ["#2c3138", "#20242a"] }, drk: { d: ["#14171b", "#0e1013"] }, edge: "#08090b",
    collar: "#0b0d10", node: "#0b0d10",
    lanes: [{ core: "#0b0d10", glow: "#6b7682" }, { core: "#2c3138", glow: "#97a1ad" }],
  },
};

interface Cfg {
  theme: Theme;
  orient: Orient;
  lanes: number;
  band: number;
  coupling: number[] | null;
  nodes: boolean;
}

// Cross-section ramp (top->bottom) for thickness T: 3 lit wall rows,
// T-6 interior channel rows, 3 shadow wall rows. Min T = 8.
function buildRamp(th: Theme, T: number): RampEntry[] {
  const r: RampEntry[] = [th.hi, th.lite, th.lite];
  for (let i = 0; i < T - 6; i++) r.push("IN");
  r.push(th.drk); r.push(th.drk); r.push(th.edge);
  return r;
}

function cfg(opts: BuildOpts): Cfg {
  const t = opts.theme ?? "mono";
  const theme = typeof t === "string" ? THEMES[t] : t;
  const lanes = Math.max(1, opts.lanes ?? 2);
  const band = Math.max(8, opts.band ?? 11);
  const maxBand = Math.floor((N - (lanes - 1) * G) / lanes);
  return {
    theme,
    orient: opts.orient === "v" ? "v" : "h",
    lanes,
    band: Math.min(band, maxBand),
    coupling: opts.coupling ?? null,
    nodes: !!opts.nodes,
  };
}

function bands(lanes: number, T: number): number[] {
  const total = lanes * T + (lanes - 1) * G;
  const start = Math.floor((N - total) / 2);
  const out: number[] = [];
  for (let i = 0; i < lanes; i++) out.push(start + i * (T + G));
  return out;
}

function toXY(orient: Orient, u: number, v: number): [number, number] {
  return orient === "h" ? [u, v] : [v, u];
}

function resolve(entry: RampEntry, x: number, y: number): string {
  if (typeof entry === "string") return entry;
  return entry.d[(((x + y) % 2) + 2) % 2 === 0 ? 0 : 1];
}

function inBounds(x: number, y: number): boolean {
  return x >= 0 && x < N && y >= 0 && y < N;
}

function buildGrid(opts: BuildOpts): { grid: Cell[][]; intr: boolean[][] } {
  const c = cfg(opts), th = c.theme, T = c.band, tops = bands(c.lanes, T);
  const phase = opts.phase ?? 0, withPackets = opts.packets !== false;
  const ramp = buildRamp(th, T);
  const grid: Cell[][] = [], intr: boolean[][] = [];
  for (let y = 0; y < N; y++) { grid.push(new Array<Cell>(N).fill(null)); intr.push(new Array<boolean>(N).fill(false)); }

  const laneIntr: Array<[number | null, number | null]> = [];
  for (let li = 0; li < c.lanes; li++) {
    const top = tops[li]!; let iA: number | null = null, iB: number | null = null;
    for (let r = 0; r < T; r++) {
      const entry = ramp[r]!, cross = top + r, isIn = entry === "IN";
      for (let u = 0; u < N; u++) {
        const [x, y] = toXY(c.orient, u, cross);
        if (!inBounds(x, y)) continue;
        if (isIn) { grid[y]![x] = th.intr; intr[y]![x] = true; }
        else grid[y]![x] = resolve(entry, x, y);
      }
      if (isIn) { if (iA === null) iA = cross; iB = cross; }
    }
    laneIntr.push([iA, iB]);
  }

  if (withPackets) {
    for (let l = 0; l < c.lanes; l++) {
      const range = laneIntr[l]!, c0 = range[0], c1 = range[1];
      const perLane = c.lanes === 1 ? 3 : 2, laneOff = l * 0.31;
      for (let p = 0; p < perLane; p++) {
        const f = ((((phase + laneOff) + p / perLane) % 1) + 1) % 1;
        const u0 = Math.round(f * (N - 1));
        const col = c.lanes === 1 ? th.lanes[p % th.lanes.length]! : th.lanes[l % th.lanes.length]!;
        paintRange(grid, intr, c.orient, u0 - 1, c0, c1, col.glow);
        paintRange(grid, intr, c.orient, u0 + COREW, c0, c1, col.glow);
        for (let w = 0; w < COREW; w++) paintRange(grid, intr, c.orient, u0 + w, c0, c1, col.core);
      }
    }
  }

  if (c.coupling) {
    for (let ci = 0; ci < c.coupling.length; ci++) {
      const u = Math.round(c.coupling[ci]! * (N - 1));
      for (let ln = 0; ln < c.lanes; ln++) {
        const t0 = tops[ln]!;
        for (let rr = 0; rr < T; rr++) {
          const cr = t0 + rr;
          stamp(grid, c.orient, u - 1, cr, th.collar);
          stamp(grid, c.orient, u, cr, th.collar);
          stamp(grid, c.orient, u + 1, cr, resolve(th.drk, u + 1, cr));
        }
      }
    }
  }

  if (c.nodes && c.lanes >= 1) {
    const W = 5, clTop = tops[0]!, clBot = tops[c.lanes - 1]! + T - 1;
    drawNode(grid, c.orient, 0, W - 1, clTop, clBot, th);
    drawNode(grid, c.orient, N - W, N - 1, clTop, clBot, th);
  }

  return { grid, intr };
}

function drawNode(grid: Cell[][], orient: Orient, u0: number, u1: number, c0: number, c1: number, th: Theme): void {
  for (let u = u0; u <= u1; u++) {
    for (let cc = c0; cc <= c1; cc++) {
      const tone = (cc === c0 || u === u0) ? th.hi : (cc === c1 || u === u1) ? th.edge : resolve(th.lite, u, cc);
      stamp(grid, orient, u, cc, tone);
    }
  }
  const mu = Math.floor((u0 + u1) / 2), mc = Math.floor((c0 + c1) / 2);
  for (let a = 0; a < 2; a++) for (let b = 0; b < 2; b++) stamp(grid, orient, mu + a - 1, mc + b, th.node);
}

function stamp(grid: Cell[][], orient: Orient, u: number, v: number, color: string): void {
  const [x, y] = toXY(orient, u, v);
  if (inBounds(x, y)) grid[y]![x] = color;
}

function paintRange(grid: Cell[][], intr: boolean[][], orient: Orient, u: number, c0: number | null, c1: number | null, color: string): void {
  if (c0 === null || c1 === null) return;
  for (let cc = c0; cc <= c1; cc++) {
    const [x, y] = toXY(orient, u, cc);
    if (!inBounds(x, y) || !intr[y]![x]) continue;
    grid[y]![x] = color;
  }
}

/** Render the mark into a square 2D canvas context of side `pxSize`. */
export function draw(ctx: CanvasRenderingContext2D, pxSize: number, opts: DrawOpts = {}): void {
  const { grid } = buildGrid(opts);
  if (opts.bg) { ctx.fillStyle = opts.bg; ctx.fillRect(0, 0, pxSize, pxSize); }
  else ctx.clearRect(0, 0, pxSize, pxSize);
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const k = grid[y]![x];
      if (!k) continue;
      ctx.fillStyle = k;
      const x0 = Math.round(x * pxSize / N), x1 = Math.round((x + 1) * pxSize / N);
      const y0 = Math.round(y * pxSize / N), y1 = Math.round((y + 1) * pxSize / N);
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
    }
  }
}

export { buildGrid };
