import { useState } from "react";

/**
 * Holds whether a category row shows its name in an input box.
 *
 * A new row starts with the box open, so typing replaces the default name. The "new" mark
 * can arrive after the row mounts, because the list may draw before the page records the
 * new id. The box opens then too. It never closes when the mark turns off, which would drop
 * what the user is typing.
 */
export function useNameEditing(justCreated: boolean) {
  const [editing, setEditing] = useState(justCreated);
  const [markSeen, setMarkSeen] = useState(justCreated);
  // Set during render, not in an effect, so the box shows in the same frame as the mark
  if (justCreated !== markSeen) {
    setMarkSeen(justCreated);
    if (justCreated) setEditing(true);
  }
  return [editing, setEditing] as const;
}
