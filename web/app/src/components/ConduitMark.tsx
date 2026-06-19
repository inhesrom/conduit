import { createEffect, onCleanup, onMount } from "solid-js";
import { draw } from "../brand/logo";
import { theme } from "../state/theme";

interface Props {
  /** Rendered (CSS) size in px, square. Default 24. */
  size?: number;
  /** Animate packet flow (default true). Forced off under reduced-motion. */
  animated?: boolean;
  class?: string;
}

// Phase units per second — a slow, low-key drift, not a busy marquee.
const FLOW = 0.12;

/** The Conduit mark: the brand pixel engine rendered live to a <canvas>, in
 *  the current theme's palette, with packets optionally flowing. Decorative —
 *  the adjacent "conduit" wordmark carries the name for screen readers. */
export function ConduitMark(props: Props) {
  let canvas: HTMLCanvasElement | undefined;

  onMount(() => {
    const el = canvas;
    const ctx = el?.getContext("2d") ?? null;
    if (!el || !ctx) return;

    const reduce =
      typeof matchMedia !== "undefined" &&
      matchMedia("(prefers-reduced-motion: reduce)").matches;

    let phase = 0;
    let raf = 0;
    let prev = 0;

    const paint = () => {
      const size = props.size ?? 24;
      // Render at device resolution so the pixels stay crisp on hi-dpi.
      const dpr = Math.min(3, Math.max(1, window.devicePixelRatio || 1));
      const px = Math.max(1, Math.round(size * dpr));
      if (el.width !== px) {
        el.width = px;
        el.height = px;
      }
      ctx.imageSmoothingEnabled = false;
      draw(ctx, px, { theme: theme(), phase });
    };

    const tick = (ts: number) => {
      const dt = prev ? (ts - prev) / 1000 : 0;
      prev = ts;
      phase = (phase + dt * FLOW) % 1;
      paint();
      raf = requestAnimationFrame(tick);
    };

    // Repaint when the theme or size changes (and the sole paint when static).
    createEffect(() => {
      void theme();
      void props.size;
      paint();
    });

    if ((props.animated ?? true) && !reduce) {
      raf = requestAnimationFrame(tick);
      onCleanup(() => cancelAnimationFrame(raf));
    }
  });

  return (
    <canvas
      ref={canvas}
      class={props.class}
      aria-hidden="true"
      style={{
        width: `${props.size ?? 24}px`,
        height: `${props.size ?? 24}px`,
        display: "block",
        "image-rendering": "pixelated",
      }}
    />
  );
}
