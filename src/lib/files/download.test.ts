import test from 'node:test';
import assert from 'node:assert/strict';
import { download, memorySink, nativeSink, partId, PIECE_BYTES, DownloadUnreachable } from './download.ts';

// A /dl stand-in: a file with a version tag, answering like download.rs.
function server(file: Uint8Array, etag: string, opts: { cutAt?: number[]; ignoreRange?: boolean } = {}) {
  const seen: Record<string, string>[] = [];
  const cuts = [...(opts.cutAt ?? [])];
  const fetchImpl = (async (_url: string, init: RequestInit) => {
    const h = (init.headers ?? {}) as Record<string, string>;
    seen.push({ ...h });
    const m = /^bytes=(\d+)-$/u.exec(h.Range ?? '');
    let start = m ? Number(m[1]) : 0;
    const honour = m && !opts.ignoreRange && (!h['If-Range'] || h['If-Range'] === etag);
    if (!honour) start = 0;
    if (honour && start >= file.length) {
      return new Response(null, { status: 416, headers: { 'content-range': `bytes */${file.length}` } });
    }
    const cut = cuts.shift();
    const end = cut === undefined ? file.length : Math.min(file.length, cut);
    // Pull-based, so a cut delivers every byte before it (error() on a
    // stream drops what is still queued).
    let at = start;
    const body = new ReadableStream<Uint8Array>({
      pull(c) {
        if (at < end) { c.enqueue(file.slice(at, Math.min(end, at + 1000))); at += 1000; return; }
        if (cut === undefined) c.close(); else c.error(new Error('network down'));
      },
    });
    return honour
      ? new Response(body, { status: 206, headers: { etag, 'content-range': `bytes ${start}-${file.length - 1}/${file.length}` } })
      : new Response(body, { status: 200, headers: { etag, 'content-length': String(file.length) } });
  }) as unknown as typeof globalThis.fetch;
  return { fetch: fetchImpl, seen };
}
const bytes = (n: number, seed = 1) => Uint8Array.from({ length: n }, (_, i) => (i * 7 + seed) & 0xff);
const fast = { sleep: async () => {}, freshUrl: async () => 'u2', url: 'u1' };

// A part store that behaves like downloads.rs, in memory.
function disk() {
  const parts = new Map<string, { data: number[]; etag: string | null }>();
  const calls: string[] = [];
  const invoke = async (cmd: string, a: any) => {
    calls.push(cmd);
    const p = parts.get(a.id);
    if (cmd === 'download_open') return p ? { received: p.data.length, etag: p.etag } : { received: 0, etag: null };
    if (cmd === 'download_reset') { parts.set(a.id, { data: [], etag: a.etag }); return null; }
    if (cmd === 'download_chunk') {
      if (!p) throw new Error('open part: missing');
      p.data.push(...Buffer.from(a.data, 'base64')); return null;
    }
    if (cmd === 'download_abort') { parts.delete(a.id); return null; }
    if (cmd === 'download_finish') { return `/saved/${a.name}`; }
    throw new Error(cmd);
  };
  return { parts, calls, invoke };
}

test('a whole download streams into the sink, then reports its tag (#305)', async () => {
  const file = bytes(5000);
  const { fetch, seen } = server(file, '"v1"');
  const sink = memorySink();
  const fractions: (number | null)[] = [];
  const out = await download({ ...fast, fetch, sink, onProgress: (f) => fractions.push(f) });
  assert.deepEqual(sink.bytes(), file);
  assert.equal(out.etag, '"v1"');
  assert.deepEqual(seen, [{}], 'a fresh download sends no Range');
  assert.equal(fractions.at(-1), 1);
});

test('a later session resumes a part of the same version with Range + If-Range (#305)', async () => {
  const file = bytes(9000);
  const d = disk();
  const id = partId('wss://h', '/f.bin');
  // Session 1 dies at 4000 bytes with the retries exhausted.
  const s1 = server(file, '"v1"', { cutAt: [4000, 4000, 4000] });
  await assert.rejects(download({ ...fast, maxRetries: 0, fetch: s1.fetch, sink: nativeSink(d.invoke, id, 1024) }), /network down/u);
  const left = { ...d.parts.get(id)!, size: d.parts.get(id)!.data.length };
  assert.equal(left.etag, '"v1"');
  assert.equal(left.size, 4000, 'giving up on the network flushes what arrived, so the part keeps all of it');
  // Session 2: the same file, unchanged.
  const s2 = server(file, '"v1"');
  let restarted = false;
  const sink2 = nativeSink(d.invoke, id, 1024);
  await download({ ...fast, fetch: s2.fetch, sink: sink2, onRestart: () => { restarted = true; } });
  assert.deepEqual(s2.seen[0], { Range: `bytes=${left.size}-`, 'If-Range': '"v1"' });
  assert.deepEqual(Uint8Array.from(d.parts.get(id)!.data), file, 'part + the rest = the file');
  assert.equal(restarted, false);
});

test('a changed file answers 200: the part is reset and the restart is told once (#305)', async () => {
  const d = disk();
  const id = partId('wss://h', '/f.bin');
  d.parts.set(id, { data: [...bytes(3000)], etag: '"v1"' });
  const newer = bytes(6000, 9);
  const s = server(newer, '"v2"');
  let restarts = 0;
  const out = await download({ ...fast, fetch: s.fetch, sink: nativeSink(d.invoke, id, 1024), onRestart: () => { restarts++; } });
  assert.equal(s.seen[0]!['If-Range'], '"v1"');
  assert.deepEqual(Uint8Array.from(d.parts.get(id)!.data), newer, 'only the new version, from byte 0');
  assert.equal(d.parts.get(id)!.etag, '"v2"');
  assert.equal(out.etag, '"v2"');
  assert.equal(restarts, 1);
});

test('a 200 inside one attempt (a proxy that drops Range) overwrites instead of appending (#305)', async () => {
  const file = bytes(5000);
  const s = server(file, '"v1"', { cutAt: [2500], ignoreRange: true });
  const sink = memorySink();
  let restarts = 0;
  await download({ ...fast, fetch: s.fetch, sink, onRestart: () => { restarts++; } });
  assert.equal(s.seen[1]!.Range, 'bytes=2500-', 'the retry asked to resume');
  assert.deepEqual(sink.bytes(), file, 'the 200 replaced the first half, never duplicated it');
  assert.equal(restarts, 0, 'nothing from an EARLIER attempt was thrown away, so no notice');
});

test('a part that already holds the whole file (save cancelled) completes from the 416 (#305)', async () => {
  const file = bytes(2000);
  const d = disk();
  const id = partId('s', '/x');
  d.parts.set(id, { data: [...file], etag: '"v1"' });
  const s = server(file, '"v1"');
  await download({ ...fast, fetch: s.fetch, sink: nativeSink(d.invoke, id) });
  assert.deepEqual(Uint8Array.from(d.parts.get(id)!.data), file);
  assert.equal(s.seen.length, 1);
});

test('a sink failure is not retried and is the error the caller sees (#305)', async () => {
  const s = server(bytes(5000), '"v1"');
  let writes = 0;
  const sink = { open: async () => ({ received: 0, etag: null }), reset: async () => {}, flush: async () => {},
    write: async () => { writes++; throw new Error('disk full'); } };
  await assert.rejects(download({ ...fast, fetch: s.fetch, sink }), /disk full/u);
  assert.equal(writes, 1);
  assert.equal(s.seen.length, 1, 'no second request');
});

test('zero bytes twice is the unreachable marker for the WS fallback (#305)', async () => {
  const fetch = (async () => { throw new TypeError('Failed to fetch'); }) as unknown as typeof globalThis.fetch;
  await assert.rejects(download({ ...fast, fetch, sink: memorySink() }), (e) => e instanceof DownloadUnreachable && e.code === 'DL_HTTP_UNREACHABLE');
});

test('the native sink sends ≤ one piece per call, never the whole file (#305)', async () => {
  const sent: number[] = [];
  const invoke = async (cmd: string, a: any) => { if (cmd === 'download_chunk') sent.push(Buffer.from(a.data, 'base64').length); return null; };
  const sink = nativeSink(invoke, 'abcdefabcdefabcd', 1000);
  await sink.write(bytes(2500)); await sink.write(bytes(700)); await sink.flush();
  assert.deepEqual(sent, [1000, 1000, 1000, 200]);
  assert.equal(PIECE_BYTES * 4 / 3, 4 * 1024 * 1024, 'the default piece is exactly 4 MiB of base64');
});

test('the part id is 16 hex digits, stable for server + path, different otherwise (#305)', () => {
  const a = partId('wss://studio:8443/ws', '/home/u/v.mp4');
  assert.match(a, /^[0-9a-f]{16}$/u);
  assert.equal(a, partId('wss://studio:8443/ws', '/home/u/v.mp4'));
  assert.notEqual(a, partId('wss://other:8443/ws', '/home/u/v.mp4'));
  assert.notEqual(a, partId('wss://studio:8443/ws', '/home/u/w.mp4'));
  assert.equal(partId('', ''), 'af63c74c8601c8dd', 'fnv1a64 of "\\n" (offset basis cbf29ce484222325, prime 100000001b3)');
});
