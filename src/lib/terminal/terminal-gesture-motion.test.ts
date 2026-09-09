import assert from 'node:assert/strict';
import test from 'node:test';
import { scrollSamples, scrollStep, releaseVelocity, coastStep, edgeDirection, edgeStep } from './terminal-gesture-motion.ts';
import type { VelocitySample } from './terminal-gesture-motion.ts';

const near = (actual: number, expected: number) => assert.ok(Math.abs(actual - expected) < 1e-12, `${actual} != ${expected}`);

test('sample collection returns a bounded copy and retains existing sample identity', () => {
  const old = Object.freeze({ v: 2, t: 10 });
  const previous = Object.freeze([old]);
  const next = scrollSamples(previous, 32, 26, 10);
  assert.deepEqual(next, [{ v: 2, t: 10 }, { v: 2, t: 26 }]);
  assert.equal(next[0], old);
  assert.notEqual(next, previous);
  assert.deepEqual(previous, [{ v: 2, t: 10 }]);
});

test('the sample window keeps exactly 100ms but expires the next millisecond', () => {
  const sample = { v: 3, t: 0 };
  assert.deepEqual(scrollSamples([sample], 1, 100, 99), [sample, { v: 1, t: 100 }]);
  assert.deepEqual(scrollSamples([sample], 1, 101, 100), [{ v: 1, t: 101 }]);
});

test('the sample window keeps only the latest five entries and always keeps the new entry', () => {
  const samples = Array.from({ length: 5 }, (_, t) => ({ v: t + 1, t }));
  assert.deepEqual(scrollSamples(samples, 6, 5, 4), [...samples.slice(1), { v: 6, t: 5 }]);
  assert.deepEqual(scrollSamples(samples, 6, 1000, 999), [{ v: 6, t: 1000 }]);
});

test('same-time and backward clocks retain the original one-millisecond denominator floor', () => {
  assert.deepEqual(scrollSamples([], -7, 100, 100), [{ v: -7, t: 100 }]);
  assert.deepEqual(scrollSamples([], 7, 100, 200), [{ v: 7, t: 100 }]);
  const future = { v: 2, t: 200 };
  assert.deepEqual(scrollSamples([future], 7, 100, 200), [future, { v: 7, t: 100 }]);
});

test('line splitting truncates signed fractions rather than flooring negative movement', () => {
  assert.deepEqual(scrollStep(23, 10), { accumulated: 23, lines: 2, remainder: 3 });
  assert.deepEqual(scrollStep(-23, 10), { accumulated: -23, lines: -2, remainder: -3 });
  assert.deepEqual(scrollStep(5, 10), { accumulated: 5, lines: 0, remainder: 5 });
  assert.deepEqual(scrollStep(-5, 10), { accumulated: -5, lines: -0, remainder: -5 });
  assert.deepEqual(scrollStep(-0, 10), { accumulated: -0, lines: -0, remainder: -0 });
});

test('sub-line movement accumulates across events and survives a direction reversal', () => {
  let step = scrollStep(6, 10);
  step = scrollStep(step.remainder + 6, 10);
  assert.equal(step.lines, 1);
  assert.equal(step.remainder, 2);
  step = scrollStep(step.remainder - 5, 10);
  assert.equal(step.lines, -0);
  assert.equal(step.remainder, -3);
  assert.equal(scrollStep(-20, 10).remainder, 0);
});

test('fractional line sizes preserve the whole-line and remainder invariant', () => {
  for (const size of [1, 7.5, 16]) for (let value = -29.5; value <= 29.5; value += 0.5) {
    const step = scrollStep(value, size);
    assert.ok(Math.abs(step.remainder) < size);
    near(step.lines * size + step.remainder, value);
  }
});

test('release weights newer samples more heavily in insertion order', () => {
  near(releaseVelocity([{ v: 1, t: 0 }, { v: 2, t: 1 }, { v: 3, t: 2 }], 16), 14 / 6);
  near(releaseVelocity([{ v: 1, t: 200 }, { v: 3, t: 100 }], 16), 7 / 3);
  assert.ok(Number.isNaN(releaseVelocity([], 16)), 'the unchanged nonempty guard belongs to the caller');
});

test('release caps pixels per frame before converting to current line height', () => {
  assert.equal(releaseVelocity([{ v: 100, t: 0 }], 10), 24);
  assert.equal(releaseVelocity([{ v: -100, t: 0 }], 10), -24);
  assert.equal(releaseVelocity([{ v: 15, t: 0 }], 16), 15);
  assert.equal(releaseVelocity([{ v: 14, t: 0 }], 16), 14);
  assert.equal(releaseVelocity([{ v: 0.1, t: 0 }], 16), 0.1);
});

test('coast damps before adding movement and preserves signed fractional carry', () => {
  const positive = coastStep(1, 0.25), negative = coastStep(-1, -0.25);
  assert.equal(positive.velocity, 0.95);
  near(positive.accumulated, 1.2);
  assert.equal(positive.lines, 1);
  near(positive.remainder, 0.2);
  assert.equal(negative.velocity, -0.95);
  assert.equal(negative.lines, -1);
  near(negative.remainder, -0.2);
});

test('coast stops at the existing inclusive 0.05 boundary', () => {
  const at = coastStep(0.05 / 0.95, 0);
  assert.equal(at.velocity, 0.05);
  assert.equal(at.running, false);
  assert.equal(coastStep(-0.05 / 0.95, 0).running, false);
  assert.equal(coastStep((0.05 + Number.EPSILON) / 0.95, 0).running, true);
});

test('a complete coast is deterministic and symmetric without a clock or timer', () => {
  function run(velocity: number) {
    let accumulated = 0;
    const trace: number[] = [];
    for (let i = 0; i < 200; i++) {
      const step = coastStep(velocity, accumulated);
      velocity = step.velocity;
      accumulated = step.remainder;
      trace.push(step.lines);
      if (!step.running) return trace;
    }
    assert.fail('coast did not stop');
  }
  const down = run(6), up = run(-6);
  assert.deepEqual(up, down.map(value => -value));
  assert.ok(down.length > 80 && down.length < 110);
});

test('edge direction uses strict 36px zones and top priority when they overlap', () => {
  assert.equal(edgeDirection(135.999, 100, 300), -1);
  assert.equal(edgeDirection(136, 100, 300), 0);
  assert.equal(edgeDirection(264, 100, 300), 0);
  assert.equal(edgeDirection(264.001, 100, 300), 1);
  assert.equal(edgeDirection(125, 100, 150), -1);
  assert.equal(edgeDirection(99, 100, 300), -1);
  assert.equal(edgeDirection(301, 100, 300), 1);
});

test('edge speed keeps its clamp and proximity ramp in rows per frame', () => {
  assert.equal(edgeStep(0, -1, 99, 100, 300).lines, -2);
  assert.equal(edgeStep(0, 1, 301, 100, 300).lines, 2);
  const midpoint = edgeStep(0, 1, 282, 100, 300);
  assert.equal(midpoint.accumulated, 1.125);
  assert.equal(midpoint.lines, 1);
  assert.equal(midpoint.remainder, 0.125);
  assert.equal(edgeStep(0, 1, 264, 100, 300).remainder, 0.25);
  assert.equal(edgeStep(0, 1, 200, 100, 300).remainder, 0.25);
});

test('slow edge steps accumulate and reverse using the same signed remainder rule', () => {
  let accumulated = 0;
  for (let i = 0; i < 3; i++) {
    const step = edgeStep(accumulated, -1, 136, 100, 300);
    assert.equal(step.lines, -0);
    accumulated = step.remainder;
  }
  const fourth = edgeStep(accumulated, -1, 136, 100, 300);
  assert.equal(fourth.lines, -1);
  assert.equal(fourth.remainder, 0);
  assert.equal(edgeStep(-0.75, 1, 264, 100, 300).remainder, -0.5);
});

test('sample objects remain read-only when release follows multiple collection steps', () => {
  let samples: VelocitySample[] = [];
  for (let i = 1; i <= 8; i++) samples = scrollSamples(samples, i, i * 16, (i - 1) * 16);
  const before = structuredClone(samples);
  releaseVelocity(samples, 16);
  assert.deepEqual(samples, before);
});
