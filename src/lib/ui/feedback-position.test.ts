import assert from 'node:assert/strict';
import test from 'node:test';
import { feedbackLayout } from './feedback-position.ts';

test('short-tail feedback flips above Copy and stays out of the input below its scrollport (#167)', () => {
  const trigger = { left: 300, right: 344, top: 560, bottom: 604 };
  const bounds = { left: 0, right: 390, top: 80, bottom: 610 };
  const placed = feedbackLayout(trigger, bounds, { w: 220, h: 70 }, { w: 390, h: 844 })!;
  assert.ok(placed.y + 70 < trigger.top);
  assert.ok(placed.y >= bounds.top && placed.y + 70 <= bounds.bottom);
  assert.equal(placed.y, 484);
});

test('feedback uses the scrollport origin and caps oversized content through the shared placement rules (#167)', () => {
  const trigger = { left: 410, right: 454, top: 520, bottom: 564 };
  const bounds = { left: 100, right: 480, top: 150, bottom: 580 };
  const placed = feedbackLayout(trigger, bounds, { w: 600, h: 900 }, { w: 500, h: 700 })!;
  assert.equal(placed.maxWidth, 364);
  assert.equal(placed.maxHeight, 356);
  assert.equal(placed.x, 108);
  assert.ok(placed.y >= 158 && placed.y + placed.maxHeight < trigger.top);
});

test('unmeasured or off-scrollport triggers cannot show an unplaced feedback box (#167)', () => {
  const bounds = { left: 0, right: 390, top: 80, bottom: 610 };
  assert.equal(feedbackLayout({ left: 0, right: 0, top: 0, bottom: 0 }, bounds, { w: 200, h: 70 }, { w: 390, h: 844 }), null);
  assert.equal(feedbackLayout({ left: 20, right: 64, top: 630, bottom: 674 }, bounds, { w: 200, h: 70 }, { w: 390, h: 844 }), null);
});

test('content bounds are clipped by the actual ancestor scrollport (#167)', () => {
  const bounds = { left: 20, right: 380, top: 100, bottom: 1000 };
  const scrollport = { left: 10, right: 390, top: 200, bottom: 600 };
  const view = { w: 400, h: 844 };
  const pos = feedbackLayout({ left: 300, right: 344, top: 500, bottom: 528 },
    bounds, { w: 180, h: 70 }, view, { scrollport })!;
  assert.equal(pos.y, 424);
  assert.ok(pos.y + 70 <= scrollport.bottom);
  assert.equal(feedbackLayout({ left: 300, right: 344, top: 610, bottom: 638 },
    bounds, { w: 180, h: 70 }, view, { scrollport }), null);
});

test('adjacent toolbar or timestamp height stays clear without shifting the horizontal anchor (#167)', () => {
  const header = feedbackLayout({ left: 400, right: 500, top: 14, bottom: 27.5 },
    { left: 200, right: 1440, top: 0, bottom: 960 }, { w: 150, h: 45 }, { w: 1440, h: 960 },
    { keepClear: { left: 200, right: 1440, top: 0, bottom: 42 } })!;
  assert.equal(header.y, 48);
  assert.equal(header.x, 350);
  const message = feedbackLayout({ left: 280, right: 308, top: 600, bottom: 628 },
    { left: 0, right: 390, top: 40, bottom: 640 }, { w: 180, h: 70 }, { w: 390, h: 844 },
    { keepClear: { left: 260, right: 310, top: 586, bottom: 596 } })!;
  assert.ok(message.y + 70 < 586);
});
