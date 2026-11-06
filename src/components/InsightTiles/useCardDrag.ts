/**
 * Dragging the stats cards in edit mode: within the row to reorder, from the card library into
 * the row to add, and from the row onto the library to remove.
 *
 * A copy of the card follows the pointer. When it is let go, the copy flies to where the card is
 * now, and that card stays hidden until the copy lands, so nothing jumps.
 */

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { MAX_CARDS, moveCard } from "../../state/insightCards";
import type { CardId } from "../../utils/insightCards";
import type { InsightScope } from "../../utils/insightWindows";

/** A press moves this far before it counts as a drag, so a click stays a click. */
const THRESHOLD_PX = 5;
/** Two swaps at least this far apart, so a card does not swap back and forth under the pointer. */
const SWAP_MS = 140;
const SLIDE = "180ms cubic-bezier(.2,.7,.2,1)";
const LAND_MS = 180;
/** The page scrolls while the pointer is this close to its top or bottom edge. */
const EDGE_PX = 48;
const MAX_SCROLL_PX = 14;

/** The nearest parent that scrolls up and down. */
function scrollerOf(el: HTMLElement | null): HTMLElement | null {
  for (let n = el?.parentElement ?? null; n; n = n.parentElement) {
    const { overflowY } = getComputedStyle(n);
    if ((overflowY === "auto" || overflowY === "scroll") && n.scrollHeight > n.clientHeight) {
      return n;
    }
  }
  return null;
}

const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Slides the cards from where they were to where they are after a change. Call the returned
 *  function just before the change; the slide plays after the next render. */
export function useFlip(container: RefObject<HTMLElement | null>): () => void {
  const before = useRef<Map<string, DOMRect> | null>(null);
  const capture = useCallback(() => {
    if (reducedMotion()) return;
    const rects = new Map<string, DOMRect>();
    container.current
      ?.querySelectorAll<HTMLElement>("[data-card]")
      .forEach((el) => rects.set(el.dataset.card ?? "", el.getBoundingClientRect()));
    before.current = rects;
  }, [container]);

  useLayoutEffect(() => {
    const rects = before.current;
    if (!rects) return;
    before.current = null;
    container.current?.querySelectorAll<HTMLElement>("[data-card]").forEach((el) => {
      const a = rects.get(el.dataset.card ?? "");
      // A landing card is hidden and has to report its real place to the copy
      if (!a || el.dataset.landing !== undefined) return;
      el.style.transition = "none";
      el.style.transform = "";
      const b = el.getBoundingClientRect();
      const dx = a.left - b.left;
      const dy = a.top - b.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
      el.style.transform = `translate(${dx}px, ${dy}px)`;
      void el.offsetWidth;
      el.style.transition = `transform ${SLIDE}`;
      el.style.transform = "";
      el.addEventListener("transitionend", () => (el.style.transition = ""), { once: true });
    });
  });

  return capture;
}

export interface CardDrag {
  id: CardId;
  from: "row" | "library";
  /** A card from the row is over the library: letting go removes it. */
  overLibrary: boolean;
  /** Where a card from the library goes in the row; null while it is not over the row. */
  insertAt: number | null;
  /** Let go: the copy flies to the card's place. */
  landing: boolean;
  width: number;
  height: number;
}

interface Press {
  id: CardId;
  from: CardDrag["from"];
  el: HTMLElement;
  pointerId: number;
  x0: number;
  y0: number;
  x: number;
  y: number;
  /** Where the copy is held, measured from its top left corner. */
  dx: number;
  dy: number;
  started: boolean;
  lastMove: number;
  warned: boolean;
  /** Where a card from the row started, to put it back when the drag is cancelled. */
  origin: number;
  scroller: HTMLElement | null;
  frame: number;
}

interface Options {
  scope: InsightScope;
  cards: readonly CardId[];
  enabled: boolean;
  /** Holds the row and the library. */
  root: RefObject<HTMLElement | null>;
  /** The copy that follows the pointer. */
  floating: RefObject<HTMLDivElement | null>;
  /** Call before the row changes, so the cards slide. */
  capture: () => void;
  onAdd: (id: CardId, at: number) => void;
  onRemove: (id: CardId) => void;
  /** A card from the library reached a full row. */
  onFull: () => void;
}

export function useCardDrag({
  scope,
  cards,
  enabled,
  root,
  floating,
  capture,
  onAdd,
  onRemove,
  onFull,
}: Options) {
  const [drag, setDragState] = useState<CardDrag | null>(null);
  // The window listeners read these between renders
  const dragRef = useRef<CardDrag | null>(null);
  const press = useRef<Press | null>(null);
  const latest = useRef({ scope, cards, capture, onAdd, onRemove, onFull });
  latest.current = { scope, cards, capture, onAdd, onRemove, onFull };

  const setDrag = useCallback((next: CardDrag | null) => {
    dragRef.current = next;
    setDragState(next);
  }, []);

  const onPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0 || !enabled || press.current || dragRef.current) return;
    const target = e.target as HTMLElement;
    if (target.closest("[data-no-drag]")) return;
    const tile = target.closest<HTMLElement>("[data-card]");
    const pick = tile ? null : target.closest<HTMLElement>("[data-pick]");
    const el = tile ?? pick;
    if (!el) return;
    press.current = {
      id: (tile ? tile.dataset.card : pick?.dataset.pick) as CardId,
      from: tile ? "row" : "library",
      el,
      pointerId: e.pointerId,
      x0: e.clientX,
      y0: e.clientY,
      x: e.clientX,
      y: e.clientY,
      dx: 0,
      dy: 0,
      started: false,
      lastMove: 0,
      warned: false,
      origin: -1,
      scroller: null,
      frame: 0,
    };
  };

  useEffect(() => {
    const place = (p: Press) => {
      const el = floating.current;
      if (el) el.style.transform = `translate(${p.x - p.dx}px, ${p.y - p.dy}px) rotate(-1deg)`;
    };

    const start = (p: Press) => {
      const r = p.el.getBoundingClientRect();
      // A card from the library takes the size of a card in the row
      const sample =
        p.from === "row" ? p.el : root.current?.querySelector<HTMLElement>("[data-card]");
      const box = sample ? sample.getBoundingClientRect() : r;
      // Held at the same spot, relative to its size, so the copy starts right under the pointer
      p.dx = ((p.x0 - r.left) / r.width) * box.width;
      p.dy = ((p.y0 - r.top) / r.height) * box.height;
      p.started = true;
      p.origin = latest.current.cards.indexOf(p.id);
      p.scroller = scrollerOf(root.current);
      p.frame = requestAnimationFrame(() => autoScroll(p));
      document.body.dataset.cardDragging = "";
      setDrag({
        id: p.id,
        from: p.from,
        overLibrary: false,
        insertAt: null,
        landing: false,
        width: box.width,
        height: box.height,
      });
    };

    const hover = (p: Press) => {
      const d = dragRef.current;
      if (!d || d.landing) return;
      const { cards: row, capture: slide } = latest.current;
      const under = document.elementFromPoint(p.x, p.y) as HTMLElement | null;
      const now = performance.now();
      if (p.from === "row") {
        const overLibrary = !!under?.closest("[data-library]");
        if (overLibrary !== d.overLibrary) {
          setDrag({ ...d, overLibrary });
          return;
        }
        const hit = under?.closest<HTMLElement>("[data-card]")?.dataset.card as CardId | undefined;
        if (overLibrary || !hit || hit === p.id || now - p.lastMove < SWAP_MS) return;
        const to = row.indexOf(hit);
        if (to < 0) return;
        slide();
        moveCard(latest.current.scope, p.id, to);
        p.lastMove = now;
        return;
      }
      if (!under?.closest("[data-tiles]")) {
        if (d.insertAt !== null) {
          slide();
          setDrag({ ...d, insertAt: null });
        }
        return;
      }
      if (row.length >= MAX_CARDS) {
        if (!p.warned) latest.current.onFull();
        p.warned = true;
        return;
      }
      const hitEl = under.closest<HTMLElement>("[data-card]");
      const hit = hitEl?.dataset.card as CardId | undefined;
      let at = d.insertAt;
      if (hitEl && hit && hit !== p.id) {
        const r = hitEl.getBoundingClientRect();
        at = row.indexOf(hit) + (p.x > r.left + r.width / 2 ? 1 : 0);
      } else if (at === null) {
        at = row.length;
      }
      if (at !== d.insertAt && (d.insertAt === null || now - p.lastMove >= SWAP_MS)) {
        slide();
        setDrag({ ...d, insertAt: at });
        p.lastMove = now;
      }
    };

    // The card library often sits below the fold: scroll while the pointer is near an edge
    const autoScroll = (p: Press) => {
      if (press.current !== p || !p.scroller) return;
      const r = p.scroller.getBoundingClientRect();
      const top = Math.max(r.top, 0);
      const bottom = Math.min(r.bottom, window.innerHeight);
      const near =
        p.y < top + EDGE_PX
          ? -(top + EDGE_PX - p.y)
          : p.y > bottom - EDGE_PX
            ? p.y - (bottom - EDGE_PX)
            : 0;
      if (near !== 0) {
        const step = Math.max(-1, Math.min(1, near / EDGE_PX)) * MAX_SCROLL_PX;
        const before = p.scroller.scrollTop;
        p.scroller.scrollTop += step;
        if (p.scroller.scrollTop !== before) hover(p);
      }
      p.frame = requestAnimationFrame(() => autoScroll(p));
    };

    const finish = (cancelled: boolean) => {
      const p = press.current;
      press.current = null;
      const d = dragRef.current;
      if (p) cancelAnimationFrame(p.frame);
      if (!p?.started || !d) return;
      delete document.body.dataset.cardDragging;
      // The press ends with a click on whatever is under the pointer; a library card would add itself
      const block = (e: MouseEvent) => {
        e.stopPropagation();
        e.preventDefault();
      };
      window.addEventListener("click", block, { capture: true, once: true });
      window.setTimeout(() => window.removeEventListener("click", block, true), 0);
      latest.current.capture();
      // Swaps are saved as they happen; a cancelled drag puts the card back
      if (cancelled && d.from === "row") moveCard(latest.current.scope, d.id, p.origin);
      if (!cancelled && d.from === "row" && d.overLibrary) latest.current.onRemove(d.id);
      if (!cancelled && d.from === "library" && d.insertAt !== null) {
        latest.current.onAdd(d.id, d.insertAt);
      }
      setDrag({ ...d, overLibrary: false, insertAt: null, landing: true });
    };

    const onMove = (e: PointerEvent) => {
      const p = press.current;
      if (!p || e.pointerId !== p.pointerId) return;
      p.x = e.clientX;
      p.y = e.clientY;
      if (!p.started) {
        if (Math.hypot(p.x - p.x0, p.y - p.y0) < THRESHOLD_PX) return;
        start(p);
      }
      e.preventDefault();
      place(p);
      hover(p);
    };
    const onUp = (e: PointerEvent) => {
      if (e.pointerId === press.current?.pointerId) finish(false);
    };
    const onCancel = () => finish(true);
    // Before the editor's own Esc, which would leave edit mode
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !press.current?.started) return;
      e.preventDefault();
      finish(true);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKey, true);
      delete document.body.dataset.cardDragging;
    };
  }, [floating, root, setDrag]);

  // The copy is drawn after the drag starts: put it under the pointer before the first paint
  useLayoutEffect(() => {
    const p = press.current;
    const el = floating.current;
    if (drag && !drag.landing && p && el) {
      el.style.transform = `translate(${p.x - p.dx}px, ${p.y - p.dy}px) rotate(-1deg)`;
    }
  }, [drag, floating]);

  // Let go: fly the copy to wherever the card is now, its slot in the row or back in the library
  const landing = drag?.landing ? drag.id : null;
  useLayoutEffect(() => {
    if (!landing) return;
    const done = () => setDrag(null);
    const el = floating.current;
    const target = root.current?.querySelector<HTMLElement>(
      `[data-card="${landing}"], [data-pick="${landing}"]`,
    );
    if (!el || !target || reducedMotion()) {
      done();
      return;
    }
    const r = target.getBoundingClientRect();
    const ease = `${LAND_MS}ms cubic-bezier(.2,.7,.2,1)`;
    el.style.transition = `transform ${ease}, width ${ease}, height ${ease}, box-shadow ${ease}`;
    el.style.width = `${r.width}px`;
    el.style.height = `${r.height}px`;
    el.style.transform = `translate(${r.left}px, ${r.top}px)`;
    el.dataset.landed = "";
    el.addEventListener("transitionend", done, { once: true });
    const timer = window.setTimeout(done, LAND_MS + 80);
    return () => window.clearTimeout(timer);
  }, [landing, floating, root, setDrag]);

  return { drag, onPointerDown };
}
