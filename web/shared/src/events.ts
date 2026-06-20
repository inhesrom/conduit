import type { Event } from "./protocol";

/** Tag of an externally-tagged serde enum member (each member is a
 * single-key object: `{"WorkspaceList": {...}}`). */
type TagOf<T> = T extends Record<string, unknown> ? keyof T & string : never;
type PayloadOf<T, K extends string> = T extends Record<K, infer P> ? P : never;

export type EventTag = TagOf<Event>;

/** The wire `Event` normalized into a TS discriminated union so `switch
 * (evt.type)` narrows. Derived from the generated bindings — stays in sync
 * when the protocol is regenerated. */
export type ConduitEvent = {
  [K in EventTag]: { type: K } & PayloadOf<Event, K>;
}[EventTag];

export type ConduitEventOf<K extends EventTag> = Extract<ConduitEvent, { type: K }>;

/** Tolerant decode: unknown tags or malformed frames return null (a newer
 * server must not crash an older client). */
export function decodeEvent(json: string): ConduitEvent | null {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return null;
  }
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) return null;
  const keys = Object.keys(raw);
  if (keys.length !== 1) return null;
  const tag = keys[0]!;
  const payload = (raw as Record<string, unknown>)[tag];
  if (payload === null || typeof payload !== "object") return null;
  return { type: tag, ...(payload as object) } as ConduitEvent;
}
