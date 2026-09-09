export type ReadingDirection = 'up' | 'down';

interface ReadingBox {
  top: number;
  height: number;
}

export interface HeldAnchor {
  key: string;
  edge: 'top' | 'bottom' | '';
  held: boolean;
}

/** Momentum jitter must travel 16px against the committed direction to flip it. */
export function readingDirection(delta: number, direction: ReadingDirection, travel: number): {
  direction: ReadingDirection;
  travel: number;
} {
  if (delta !== 0) {
    if ((delta > 0) === (direction === 'down')) {
      travel = 0;
    } else {
      travel += Math.abs(delta);
      if (travel >= 16) {
        direction = delta > 0 ? 'down' : 'up';
        travel = 0;
      }
    }
  }
  return { direction, travel };
}

/** Keep the same bubble on its held edge through 8px of layout/scroll jitter. */
export function heldAnchor(
  picked: Pick<HeldAnchor, 'key' | 'edge'>,
  chosen: ReadingBox | undefined,
  current: HeldAnchor,
  top: number,
  bottom: number,
): HeldAnchor {
  let edge = picked.edge;
  let held = false;
  if (chosen) {
    if (current.held && current.key === picked.key && current.edge === 'top' && chosen.top <= top + 8) {
      edge = 'top'; held = true;
    } else if (current.held && current.key === picked.key && current.edge === 'bottom'
      && chosen.top + chosen.height >= bottom - 8) {
      edge = 'bottom'; held = true;
    } else {
      held = edge === 'top' ? chosen.top <= top + 1
        : edge === 'bottom' ? chosen.top + chosen.height >= bottom - 1
        : false;
    }
  }
  return { key: picked.key, edge, held };
}

/** Refold only after the whole expanded bubble is beyond the 120px margin. */
export function refoldEligible(box: ReadingBox, top: number, bottom: number): boolean {
  return box.top + box.height < top - 120 || box.top > bottom + 120;
}
