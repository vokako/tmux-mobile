// A bounded least-recently-used cache over a Map.
//
// Map keeps insertion order, so "least recently used" is the FIRST key once
// every hit re-inserts its entry at the end. The previous bound in
// core/markdown.ts was `if (size > 500) clear()`: past 500 distinct bodies the
// whole cache vanished mid-render and every poll re-parsed every message with
// marked + KaTeX (review C, 2026-09-03). Evicting ONE oldest entry keeps the
// hot set warm at the same memory bound.
export class LruCache<K, V> {
  private readonly map = new Map<K, V>();
  readonly max: number;
  // No parameter properties: the project runs tests under node's strip-only
  // TypeScript, which accepts erasable syntax alone (conventions/frontend.md).
  constructor(max: number) {
    if (!(max >= 1)) throw new RangeError(`LruCache max must be >= 1, got ${max}`);
    this.max = max;
  }
  get size(): number { return this.map.size; }
  has(key: K): boolean { return this.map.has(key); }
  /** A hit makes the entry the most recently used. */
  get(key: K): V | undefined {
    const v = this.map.get(key);
    if (v === undefined && !this.map.has(key)) return undefined;
    this.map.delete(key);
    this.map.set(key, v as V);
    return v;
  }
  /** Inserts (or refreshes) and evicts the least recently used past `max`. */
  set(key: K, value: V): this {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.max) {
      const oldest = this.map.keys().next();
      if (oldest.done) break;
      this.map.delete(oldest.value);
    }
    return this;
  }
  delete(key: K): boolean { return this.map.delete(key); }
  clear(): void { this.map.clear(); }
  /** Oldest first — the eviction order. */
  keys(): IterableIterator<K> { return this.map.keys(); }
}
