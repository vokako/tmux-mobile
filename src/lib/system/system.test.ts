import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fmtBytes, fmtPair, fmtPairFull, sysDetail, sysParts, SYS_POLL_MS, SYS_POLL_MIN_MS } from './system.ts';

const G = 1024 ** 3;

test('fmtBytes: 1024 steps, one decimal under 10, degenerate → 0', () => {
  assert.equal(fmtBytes(0), '0');
  assert.equal(fmtBytes(-5), '0');
  assert.equal(fmtBytes(NaN), '0');
  assert.equal(fmtBytes(512), '512B');
  assert.equal(fmtBytes(3.1 * G), '3.1G');
  assert.equal(fmtBytes(16 * G), '16G');
  assert.equal(fmtBytes(1.2 * 1024 ** 4), '1.2T');
  // the trailing .0 is dropped, not shown
  assert.equal(fmtBytes(2 * G), '2G');
});

test('fmtPair: both numbers wear the TOTAL\'s unit, reading as a fraction', () => {
  assert.equal(fmtPair(210 * G, 473 * G), '210/473G');
  // used far below the total's unit stays IN that unit (a fraction, not two scales)
  assert.equal(fmtPair(0.5 * G, 2 * 1024 ** 4), '0/2T');
  // one decimal only under 10 — 15.3 rounds whole, 3.4 keeps its decimal
  assert.equal(fmtPair(3.4 * G, 15.3 * G), '3.4/15G');
  // degenerate totals say nothing
  assert.equal(fmtPair(5, 0), '');
  assert.equal(fmtPair(5, NaN), '');
  // negative used clamps to 0 instead of rendering a minus sign
  assert.equal(fmtPair(-1, 4 * G), '0/4G');
});

test('sysParts: null status and zero totals render NOTHING (verdict rule)', () => {
  assert.deepEqual(sysParts(null), []);
  assert.deepEqual(sysParts(undefined), []);
  assert.deepEqual(
    sysParts({ cpu_pct: null, mem_used: 0, mem_total: 0, disk_used: 0, disk_total: 0 }),
    [],
  );
});

test('sysParts: a first sample (cpu null) shows MEM/DISK and no CPU part', () => {
  const parts = sysParts({ cpu_pct: null, mem_used: 3 * G, mem_total: 16 * G, disk_used: 100 * G, disk_total: 500 * G });
  assert.deepEqual(parts.map((p) => p.k), ['MEM', 'DISK']);
});

test('sysParts: a full reading is CPU → MEM → DISK, cpu rounded and clamped', () => {
  const parts = sysParts({ cpu_pct: 12.6, mem_used: 3 * G, mem_total: 16 * G, disk_used: 210 * G, disk_total: 473 * G });
  assert.deepEqual(parts, [
    { k: 'CPU', v: '13%' },
    { k: 'MEM', v: '3/16G' },
    { k: 'DISK', v: '210/473G' },
  ]);
  assert.equal(sysParts({ cpu_pct: 250, mem_used: 0, mem_total: 0, disk_used: 0, disk_total: 0 })[0]?.v, '100%');
  assert.equal(sysParts({ cpu_pct: -3, mem_used: 0, mem_total: 0, disk_used: 0, disk_total: 0 })[0]?.v, '0%');
});

test('the poll tempo is LOW and floored', () => {
  // The owner asked for a low refresh; the server computes CPU% over this
  // very interval, so a fast poll would be both cost and noise.
  assert.ok(SYS_POLL_MS >= 10000, `default ${SYS_POLL_MS} is a low-frequency tempo`);
  assert.ok(SYS_POLL_MIN_MS >= 2000, 'a floor exists so no caller wires a hot loop');
});

test('sysDetail: the hover card’s full reading — one decimal CPU, two-decimal fractions with a percentage', () => {
  assert.deepEqual(sysDetail({ cpu_pct: 23.46, mem_used: 12.5 * G, mem_total: 64 * G, disk_used: 210 * G, disk_total: 473 * G }), [
    { k: 'CPU', v: '23.5%' },
    { k: 'MEM', v: '12.50/64.00G · 20%' },
    { k: 'DISK', v: '210.00/473.00G · 44%' },
  ]);
  // Same drop rule as sysParts: no cpu on the first sample, nothing at all for null.
  assert.deepEqual(sysDetail(null), []);
  assert.equal(sysDetail({ cpu_pct: null, mem_used: G, mem_total: 2 * G, disk_used: 0, disk_total: 0 }).map((r) => r.k).join(','), 'MEM');
  assert.equal(fmtPairFull(5, 0), '');
});
