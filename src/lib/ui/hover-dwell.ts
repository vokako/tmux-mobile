/** The hover card's timing, pure (tested without a DOM or runes). */
export const HOVER_DWELL_MS = 380;
export const HOVER_HOP_MS = 60;

/** The dwell before a card opens: a hop between neighbouring cards is near-instant. */
export function dwellFor(warm: boolean): number {
  return warm ? HOVER_HOP_MS : HOVER_DWELL_MS;
}
