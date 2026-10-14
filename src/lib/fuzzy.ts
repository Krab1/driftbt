/**
 * Subsequence match of `query` inside `text`, case-insensitive. Returns a score (higher is better) or null
 * when the letters don't appear in order. Consecutive letters and letters at the start of a word score more,
 * so "pau" finds "Pause all" before "Reorder paused".
 */
export function fuzzy(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let from = 0;
  let prev = -2;
  for (const ch of q) {
    if (ch === " ") continue;
    const at = t.indexOf(ch, from);
    if (at < 0) return null;
    score += 1;
    if (at === prev + 1) score += 3; // consecutive
    if (at === 0 || " -_/.".includes(t[at - 1])) score += 4; // start of a word
    prev = at;
    from = at + 1;
  }
  // Shorter texts win ties: the query covers more of them.
  return score - t.length * 0.01;
}

/** Items that match, best first. An empty query keeps the original order. */
export function rank<T>(items: T[], query: string, text: (item: T) => string): T[] {
  if (!query.trim()) return items;
  return items
    .map((item) => ({ item, s: fuzzy(query, text(item)) }))
    .filter((x): x is { item: T; s: number } => x.s !== null)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.item);
}
