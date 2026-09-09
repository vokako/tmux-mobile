import { gapWalkStep } from './hub.ts';

interface GapWalkRequest {
  session: string;
  floorTs: number;
  cursor: number | null | undefined;
}

interface GapWalkDeps<T extends { ts?: number }> {
  readPage(session: string, cursor: number): Promise<Parameters<typeof gapWalkStep<T>>[0]>;
  stillCurrent(session: string): boolean;
  mergePage(messages: T[]): void;
}

/** Walk the existing incremental gap, merging each page before requesting the
 * next. False means a room switch stopped the walk; read errors propagate. */
export async function walkFeedGap<T extends { ts?: number }>(
  { session, floorTs, cursor }: GapWalkRequest,
  { readPage, stillCurrent, mergePage }: GapWalkDeps<T>,
): Promise<boolean> {
  for (let i = 0; i < 50 && cursor; i++) {
    const page = await readPage(session, cursor);
    if (!stillCurrent(session)) return false;
    const { newer, next } = gapWalkStep(page, floorTs);
    if (newer.length) mergePage(newer);
    cursor = next;
  }
  return true;
}
