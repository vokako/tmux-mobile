// The meaning of a `git status --porcelain` code, in words (motion.md
// principle 16: the hover card explains the terse `M ` / `??` glyph). Pure, so
// the mapping is tested without a component. `label` is the i18n lookup, so
// the words follow the app's locale.

export type GitStatusWord =
  | 'gitUntracked' | 'gitIgnored' | 'gitAdded' | 'gitModified' | 'gitDeleted'
  | 'gitRenamed' | 'gitCopied' | 'gitUnmerged' | 'gitTypeChanged' | 'gitStaged' | 'gitUnstaged';

const WORDS: Record<string, GitStatusWord> = {
  A: 'gitAdded', M: 'gitModified', D: 'gitDeleted', R: 'gitRenamed', C: 'gitCopied', U: 'gitUnmerged', T: 'gitTypeChanged',
};

/**
 * `XY` — X is the index column, Y the work tree. "modified, staged · modified,
 * unstaged" for `MM`; "untracked" for `??`. An unknown letter falls back to the
 * raw code so nothing is ever blank.
 */
export function gitStatusMeaning(code: string, label: (key: GitStatusWord) => string): string {
  if (code === '??') return label('gitUntracked');
  if (code === '!!') return label('gitIgnored');
  const [x = ' ', y = ' '] = code;
  const parts: string[] = [];
  if (x !== ' ' && x !== '?') parts.push(`${WORDS[x] ? label(WORDS[x]) : x}, ${label('gitStaged')}`);
  if (y !== ' ') parts.push(`${WORDS[y] ? label(WORDS[y]) : y}, ${label('gitUnstaged')}`);
  return parts.join(' · ') || code;
}
