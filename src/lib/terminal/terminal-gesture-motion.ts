const MOMENTUM_MAX_PX = 240;
const MOMENTUM_FRICTION = 0.95;
const MOMENTUM_MIN_V = 0.05;
const EDGE_SCROLL_ZONE_PX = 36;

export interface VelocitySample { v: number; t: number }
export interface LineStep { accumulated: number; lines: number; remainder: number }

export function scrollSamples(
  previous: readonly VelocitySample[], dy: number, now: number, previousMoveTime: number,
): VelocitySample[] {
  const dt = Math.max(1, now - previousMoveTime);
  const samples = [...previous, { v: dy / dt, t: now }];
  while (samples.length > 5 || (samples.length > 1 && now - samples[0]!.t > 100)) {
    samples.shift();
  }
  return samples;
}

/** Split input units into whole lines without rounding a negative fraction down. */
export function scrollStep(accumulated: number, unitsPerLine: number): LineStep {
  const lines = Math.trunc(accumulated / unitsPerLine);
  const remainder = lines !== 0 ? accumulated - lines * unitsPerLine : accumulated;
  return { accumulated, lines, remainder };
}

/** The caller retains the existing nonempty-sample and >0.1 start guards. */
export function releaseVelocity(samples: readonly VelocitySample[], lineHeight: number): number {
  let wSum = 0, wTotal = 0;
  for (let i = 0; i < samples.length; i++) {
    const w = i + 1;
    wSum += samples[i]!.v * w;
    wTotal += w;
  }
  const avgVelocity = wSum / wTotal;
  const maxPxPerFrame = MOMENTUM_MAX_PX;
  const cappedPx = Math.max(-maxPxPerFrame, Math.min(maxPxPerFrame, avgVelocity * 16));
  return cappedPx / lineHeight;
}

export function coastStep(velocity: number, accumulated: number): LineStep & { velocity: number; running: boolean } {
  velocity *= MOMENTUM_FRICTION;
  const step = scrollStep(accumulated + velocity, 1);
  return { ...step, velocity, running: Math.abs(velocity) > MOMENTUM_MIN_V };
}

export function edgeDirection(clientY: number, top: number, bottom: number): -1 | 0 | 1 {
  const topDist = clientY - top;
  const botDist = bottom - clientY;
  let dir: -1 | 0 | 1 = 0;
  if (topDist < EDGE_SCROLL_ZONE_PX) dir = -1;
  else if (botDist < EDGE_SCROLL_ZONE_PX) dir = 1;
  return dir;
}

export function edgeStep(
  accumulated: number, direction: number, clientY: number, top: number, bottom: number,
): LineStep {
  const dist = direction < 0 ? Math.max(0, clientY - top) : Math.max(0, bottom - clientY);
  const speed = 0.25 + (1 - Math.min(1, dist / EDGE_SCROLL_ZONE_PX)) * 1.75;
  return scrollStep(accumulated + speed * direction, 1);
}
