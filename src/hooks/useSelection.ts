import { useEffect, useMemo, useState } from "react";

/** An open menu or dialog takes Escape for itself */
const POPUP =
  '[role="dialog"], [role="alertdialog"], [role="menu"], [aria-haspopup][aria-expanded="true"]';

/**
 * Select mode of a list: while it is on, rows can be picked for a batch action, by id.
 *
 * Only rows still in `visible` count as selected, in the order of `visible`. A row that was
 * deleted or filtered out is never acted on. Turning select mode off, or pressing Escape while
 * no menu or dialog is open, drops the picks.
 */
export function useSelection(visible: readonly string[]) {
  const [active, setActive] = useState(false);
  const [picked, setPicked] = useState<ReadonlySet<string>>(() => new Set());
  const selected = useMemo(() => visible.filter((id) => picked.has(id)), [visible, picked]);
  const allSelected = visible.length > 0 && selected.length === visible.length;

  const stop = () => {
    setActive(false);
    setPicked(new Set());
  };

  // Capture runs before the popups' own Escape handlers close them, so they are still there
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || document.querySelector(POPUP)) return;
      setActive(false);
      setPicked(new Set());
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [active]);

  const toggle = (id: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const toggleAll = () => setPicked(allSelected ? new Set() : new Set(visible));

  return {
    active,
    start: () => setActive(true),
    stop,
    selected,
    allSelected,
    isPicked: (id: string) => picked.has(id),
    toggle,
    toggleAll,
  };
}

export type Selection = ReturnType<typeof useSelection>;
