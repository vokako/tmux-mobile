/**
 * The hover card's ONE state (motion.md §2, board #86: "悬浮到 agent 卡片上，
 * 悬浮到侧边栏上的不同选项上等等，都动态给我展示一些信息"). A single fixed
 * card (`ui/HoverCard.svelte`, mounted once in App) shows whatever the
 * `hoverInfo` action last asked for; there is never a second tooltip
 * species, and a native `title` is removed wherever the card takes over.
 */
import type { AnchorRect } from './placement.ts';

export interface HoverLine { label: string; value: string; tone?: 'ok' | 'warn' | 'danger' | 'accent' }
export interface HoverInfo {
  /** First line, the thing's name. */
  title?: string;
  /** One-line description under the title. */
  text?: string;
  /** label → value rows (state, model, path…). */
  lines?: HoverLine[];
  /** A quiet last line: a hint, a shortcut, an age. */
  note?: string;
}

interface Shown { anchor: AnchorRect; info: HoverInfo; align: 'left' | 'right' }

let shown = $state<Shown | null>(null);
/** When the last card hid — a hop between two neighbours reopens without the delay. */
let hiddenAt = 0;

export const hoverCard = {
  get current(): Shown | null { return shown; },
  show(anchor: AnchorRect, info: HoverInfo, align: 'left' | 'right' = 'left') {
    shown = { anchor, info, align };
  },
  hide() {
    if (shown) hiddenAt = Date.now();
    shown = null;
  },
  /** True right after a hide: the pointer is wandering between targets. */
  get warm(): boolean { return Date.now() - hiddenAt < 400; },
};
