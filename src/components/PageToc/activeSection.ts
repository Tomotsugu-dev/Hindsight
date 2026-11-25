/** Where a section starts, in the same coordinates as the line passed to {@link activeSection}. */
export interface SectionTop {
  id: string;
  top: number;
}

/**
 * Picks the section the reader is in: the last one whose top has reached the line.
 *
 * Before the first section reaches the line, it is still the first one. At the bottom of the page
 * it is the last one, since a short last section may never reach the line.
 */
export function activeSection(
  tops: readonly SectionTop[],
  line: number,
  atBottom: boolean,
): string | null {
  if (tops.length === 0) return null;
  if (atBottom) return tops[tops.length - 1].id;
  let active = tops[0].id;
  for (const t of tops) {
    if (t.top <= line) active = t.id;
  }
  return active;
}
