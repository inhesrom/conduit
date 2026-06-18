/** FLIP for the Board's signature reflow.
 *
 * All rows live in one keyed list, so when a workspace changes attention band
 * its element keeps identity and simply moves position. We record each row's
 * top before the change and, after the DOM settles, animate it from the old
 * position to the new one — the row literally glides to its new group, so the
 * eye follows what changed. Respects prefers-reduced-motion.
 */
const reduceMotion =
  typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;

export function makeFlip() {
  const els = new Map<string, HTMLElement>();
  let prev = new Map<string, number>();

  const register = (id: string) => (el: HTMLElement | undefined) => {
    if (el) els.set(id, el);
    else els.delete(id);
  };

  const play = (): void => {
    const next = new Map<string, number>();
    els.forEach((el, id) => next.set(id, el.getBoundingClientRect().top));

    if (!reduceMotion) {
      els.forEach((el, id) => {
        const o = prev.get(id);
        const n = next.get(id)!;
        if (o === undefined) {
          // First appearance: a quiet fade/rise in.
          el.animate(
            [
              { opacity: 0, transform: "translateY(4px)" },
              { opacity: 1, transform: "translateY(0)" },
            ],
            { duration: 220, easing: "cubic-bezier(0.2,0.8,0.2,1)" },
          );
        } else if (Math.abs(o - n) > 0.5) {
          el.animate(
            [{ transform: `translateY(${o - n}px)` }, { transform: "translateY(0)" }],
            { duration: 340, easing: "cubic-bezier(0.2,0.8,0.2,1)" },
          );
        }
      });
    }
    prev = next;
  };

  return { register, play };
}
