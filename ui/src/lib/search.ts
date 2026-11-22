// Full-text search of the window, insensitive to accents and case (spec 017; the Journal tab of spec 018 reuses
// it). Pure functions: no DOM, no HTML. Highlighting returns text segments that the components render as text, so a
// description written by a server is never interpreted.

const DIACRITICS = /\p{M}/gu

/** `Développeur` → `developpeur`: NFD, combining marks removed, lower case. */
export function normalize(text: string): string {
  return text.normalize('NFD').replace(DIACRITICS, '').toLowerCase()
}

/** The words of a query, normalized, without duplicates; empty for a blank query. */
export function terms(query: string): string[] {
  const words = normalize(query)
    .split(/\s+/u)
    .filter((w) => w.length > 0)
  return [...new Set(words)]
}

/** Whether `term` (already normalized) occurs in `text`. */
export function contains(text: string, term: string): boolean {
  return normalize(text).includes(term)
}

/** Whether one of `texts` contains `term`. */
export function anyContains(texts: readonly string[], term: string): boolean {
  return texts.some((t) => contains(t, term))
}

export interface Segment {
  text: string
  match: boolean
}

/**
 * `text` cut into segments, those that match one of `words` (normalized) marked. Positions are found in the
 * normalized text and mapped back to the original characters, so that `kibána` highlights `Kibana` and `Kibana`
 * highlights `Kibána` whole.
 */
export function highlight(text: string, words: readonly string[]): Segment[] {
  if (text.length === 0) return []
  // For each character of the normalized text, the index of the original character it comes from.
  const chars = Array.from(text)
  let normalized = ''
  const origin: number[] = []
  chars.forEach((char, index) => {
    const n = normalize(char)
    normalized += n
    for (let unit = 0; unit < n.length; unit += 1) origin.push(index)
  })
  const marked = Array.from({ length: chars.length }, () => false)
  for (const word of words) {
    if (word.length === 0) continue
    let from = normalized.indexOf(word)
    while (from !== -1) {
      for (let i = from; i < from + word.length; i += 1) {
        const at = origin[i]
        if (at !== undefined) marked[at] = true
      }
      from = normalized.indexOf(word, from + 1)
    }
  }
  const segments: Segment[] = []
  chars.forEach((char, index) => {
    const match = marked[index] ?? false
    const last = segments[segments.length - 1]
    if (last && last.match === match) last.text += char
    else segments.push({ text: char, match })
  })
  return segments
}
