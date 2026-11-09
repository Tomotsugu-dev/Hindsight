/** Where a dragged app lands on the Assign apps page: a group row, a new group, or nowhere */
export type PairingDrop = { kind: "group"; groupId: string } | { kind: "newGroup" } | null;

/** The top and bottom of an element on screen. A DOMRect is one */
export interface Band {
  top: number;
  bottom: number;
}

/**
 * Returns the drop under the cursor: the group row it is over, or the new-group zone below the
 * last row.
 *
 * `rows` is read in order and can be lazy, so the caller can measure each row only when it is
 * reached. `zone` is null while the zone is not shown.
 */
export function pickPairingDrop(
  clientY: number,
  rows: Iterable<[string, Band]>,
  zone: Band | null,
): PairingDrop {
  for (const [groupId, band] of rows) {
    if (clientY >= band.top && clientY <= band.bottom) return { kind: "group", groupId };
  }
  if (zone && clientY >= zone.top && clientY <= zone.bottom) return { kind: "newGroup" };
  return null;
}

/**
 * Whether the app can move into a group of its own. The app a group is named after is already
 * in its own group, and the backend refuses to move it there.
 */
export function canMakeOwnGroup(processName: string, groupId: string): boolean {
  return processName !== groupId;
}

/** Whether two drops point at the same place */
export function sameDrop(a: PairingDrop, b: PairingDrop): boolean {
  if (a === null || b === null) return a === b;
  if (a.kind === "group" && b.kind === "group") return a.groupId === b.groupId;
  return a.kind === b.kind;
}
