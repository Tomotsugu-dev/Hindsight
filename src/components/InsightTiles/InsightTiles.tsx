import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { GripVertical, SlidersHorizontal, X } from "lucide-react";
import { useInsightData, type InsightFocus } from "../../hooks/useInsightData";
import {
  addCard,
  CARD_LIBRARY,
  getCards,
  MAX_CARDS,
  MIN_CARDS,
  moveCard,
  removeCard,
  resetCards,
  setMode,
  useInsightCards,
  useInsightMode,
} from "../../state/insightCards";
import type { CardId } from "../../utils/insightCards";
import type { InsightMode, InsightScope } from "../../utils/insightWindows";
import { CardBody } from "./CardValue";
import { cardContext, cardName, scopeNote } from "./cardText";
import { CardLibrary, EditBar } from "./InsightEditor";
import styles from "./InsightTiles.module.css";
import { useCardDrag, useFlip } from "./useCardDrag";

const K = "components.insightCards";
const MESSAGE_MS = 3500;

interface Props {
  scope: InsightScope;
  offset: number;
  focus: InsightFocus;
  /** The color of the drilled super category or the hovered app; the cards take it on. */
  accent?: string;
}

/** Where focus goes after the render that follows an action. */
type FocusTarget =
  | { kind: "grip"; id: CardId }
  | { kind: "edit" }
  | { kind: "done" }
  | { kind: "mode"; mode: InsightMode };

/** Columns of a 12-column grid: 3 and 4 cards fill one row, 5 go 3 then 2, 6 go 3 and 3. */
function spanOf(count: number, index: number): number {
  if (count === 4) return 3;
  if (count === 5) return index < 3 ? 4 : 6;
  return 4;
}

/** Whether the card starts a row, where the flat layout draws no divider. */
const startsRow = (count: number, index: number) => index === 0 || (count > 4 && index === 3);

/**
 * The stats cards under the share view: 3 to 6 cards, each counted over the same stretch.
 * In edit mode they can be removed, moved and added from the card library, and the week and
 * month pages can switch how all of them count. Every change is saved at once.
 */
export function InsightTiles({ scope, offset, focus, accent }: Props) {
  const { t } = useTranslation();
  const cards = useInsightCards(scope);
  const mode = useInsightMode(scope);
  const [editing, setEditing] = useState(false);
  // `at` makes the same message twice count as a new one
  const [message, setMessage] = useState<{ text: string; at: number } | null>(null);
  const data = useInsightData({
    scope,
    offset,
    mode,
    cards: editing ? CARD_LIBRARY[scope] : cards,
    focus,
  });
  const ctx = cardContext(scope, mode, data.windows, data.now, focus);

  const rootRef = useRef<HTMLElement>(null);
  const floatRef = useRef<HTMLDivElement>(null);
  const editRef = useRef<HTMLButtonElement>(null);
  const doneRef = useRef<HTMLButtonElement>(null);
  const modeRef = useRef<HTMLDivElement>(null);
  const grips = useRef(new Map<CardId, HTMLButtonElement>());
  const pendingFocus = useRef<FocusTarget | null>(null);
  const slide = useFlip(rootRef);

  // A moved card's node is moved, and a removed one is gone, so focus is put back by hand
  useLayoutEffect(() => {
    const target = pendingFocus.current;
    if (!target) return;
    pendingFocus.current = null;
    const el =
      target.kind === "grip"
        ? grips.current.get(target.id)
        : target.kind === "edit"
          ? editRef.current
          : target.kind === "done"
            ? doneRef.current
            : modeRef.current?.querySelector<HTMLButtonElement>(`[data-mode="${target.mode}"]`);
    el?.focus();
  });

  useEffect(() => {
    if (!message) return;
    const timer = window.setTimeout(() => setMessage(null), MESSAGE_MS);
    return () => window.clearTimeout(timer);
  }, [message]);

  const say = (text: string) => setMessage({ text, at: Date.now() });
  const nameOf = (id: CardId) => cardName(t, id, scope);

  const exit = useCallback(() => {
    setEditing(false);
    setMessage(null);
    pendingFocus.current = { kind: "edit" };
  }, []);
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) exit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, exit]);

  const start = () => {
    setEditing(true);
    pendingFocus.current = { kind: "done" };
  };

  // A dropped card keeps focus where it was; a button press moves it to the card it changed
  const remove = (id: CardId, moveFocus = true) => {
    const index = cards.indexOf(id);
    slide();
    if (removeCard(scope, id) === "min") {
      say(t(`${K}.atMin`, { count: MIN_CARDS }));
      if (moveFocus) pendingFocus.current = { kind: "grip", id };
      return;
    }
    const left = getCards(scope);
    say(t(`${K}.removed`, { name: nameOf(id), count: left.length }));
    const next = left[Math.min(index, left.length - 1)];
    if (moveFocus) pendingFocus.current = next ? { kind: "grip", id: next } : { kind: "done" };
  };

  const add = (id: CardId, at?: number) => {
    slide();
    if (addCard(scope, id, at) === "max") {
      say(t(`${K}.atMax`, { count: MAX_CARDS }));
      return;
    }
    say(t(`${K}.added`, { name: nameOf(id), count: getCards(scope).length }));
    if (at === undefined) pendingFocus.current = { kind: "grip", id };
  };

  const move = (id: CardId, delta: number) => {
    const to = cards.indexOf(id) + delta;
    pendingFocus.current = { kind: "grip", id };
    if (to < 0 || to >= cards.length) return;
    slide();
    moveCard(scope, id, to);
    say(t(`${K}.moved`, { name: nameOf(id), position: to + 1 }));
  };

  const { drag, onPointerDown } = useCardDrag({
    scope,
    cards,
    enabled: editing,
    root: rootRef,
    floating: floatRef,
    capture: slide,
    onAdd: add,
    onRemove: (id) => remove(id, false),
    onFull: () => say(t(`${K}.atMax`, { count: MAX_CARDS })),
  });

  const onGripKey = (id: CardId) => (e: KeyboardEvent) => {
    if (e.key === "ArrowLeft" || e.key === "ArrowUp") move(id, -1);
    else if (e.key === "ArrowRight" || e.key === "ArrowDown") move(id, 1);
    else if (e.key === "Delete" || e.key === "Backspace") remove(id);
    else return;
    e.preventDefault();
  };

  const changeMode = (next: InsightMode) => {
    pendingFocus.current = { kind: "mode", mode: next };
    if (scope === "today" || next === mode) return;
    setMode(scope, next);
    say(t(`${K}.modeChanged`, { mode: t(`${K}.mode.${next}`) }));
  };

  const reset = () => {
    slide();
    resetCards(scope);
    say(t(`${K}.resetDone`));
  };

  const atMin = cards.length <= MIN_CARDS;
  const style = accent ? ({ "--tile-accent": accent } as CSSProperties) : undefined;
  const moving = drag && !drag.landing ? drag : null;
  // A card from the library held over the row shows where it would go
  const pending = moving?.from === "library" && moving.insertAt !== null ? moving : null;
  const row = pending
    ? [...cards.slice(0, pending.insertAt ?? 0), pending.id, ...cards.slice(pending.insertAt ?? 0)]
    : cards;
  const landing = drag?.landing ? drag.id : null;
  const overLibrary = moving?.from === "row" && moving.overLibrary;
  const note = scopeNote(t, ctx, data.windows, data.now);
  // While editing, the day page shows the note in the mode row instead
  const noteInModeRow = editing && scope === "today";

  return (
    <section
      ref={rootRef}
      className={styles.area}
      aria-label={t(`${K}.listLabel`)}
      onPointerDown={onPointerDown}
    >
      {editing && (
        <EditBar
          scope={scope}
          mode={mode}
          note={note}
          onMode={changeMode}
          onReset={reset}
          onDone={exit}
          doneRef={doneRef}
          modeRef={modeRef}
        />
      )}
      {!noteInModeRow && (
        <div className={styles.head}>
          <p className={styles.note}>{note}</p>
          {!editing && (
            <button
              ref={editRef}
              type="button"
              className={`${styles.btn} ${styles.btnPlain}`}
              onClick={start}
            >
              <SlidersHorizontal size={14} aria-hidden />
              {t(`${K}.edit`)}
            </button>
          )}
        </div>
      )}
      <div
        className={styles.tiles}
        role="list"
        data-tiles=""
        data-editing={editing ? "" : undefined}
        data-drill={accent ? "" : undefined}
        style={style}
      >
        {row.map((id, i) => (
          <div
            key={id}
            role="listitem"
            className={styles.tile}
            data-card={id}
            data-row-start={startsRow(row.length, i) ? "" : undefined}
            data-ghost={moving?.from === "row" && moving.id === id ? "" : undefined}
            data-pending={pending?.id === id ? "" : undefined}
            data-landing={landing === id ? "" : undefined}
            style={{ "--span": spanOf(row.length, i) } as CSSProperties}
          >
            <CardBody id={id} value={data.values[id]} ctx={ctx} />
            {editing && (
              <>
                <button
                  type="button"
                  data-no-drag=""
                  className={styles.tileX}
                  aria-label={t(`${K}.remove`, { name: nameOf(id) })}
                  aria-disabled={atMin || undefined}
                  title={atMin ? t(`${K}.keepMin`, { count: MIN_CARDS }) : undefined}
                  onClick={() => remove(id)}
                >
                  <X size={14} aria-hidden />
                </button>
                <button
                  ref={(el) => {
                    if (el) grips.current.set(id, el);
                    else grips.current.delete(id);
                  }}
                  type="button"
                  className={styles.tileGrip}
                  aria-label={t(`${K}.move`, { name: nameOf(id) })}
                  onKeyDown={onGripKey(id)}
                >
                  <GripVertical size={14} aria-hidden />
                </button>
              </>
            )}
          </div>
        ))}
      </div>
      {editing && (
        <>
          <CardLibrary
            ids={CARD_LIBRARY[scope].filter((id) => !row.includes(id))}
            placed={cards.length}
            ctx={ctx}
            values={data.values}
            onAdd={(id) => add(id)}
            drop={overLibrary ? (atMin ? "blocked" : "remove") : undefined}
            dropName={overLibrary ? nameOf(moving.id) : undefined}
            landing={landing}
          />
          <p className={styles.msg} role="status" aria-live="polite">
            {message?.text ?? ""}
          </p>
        </>
      )}
      {drag &&
        createPortal(
          <div
            ref={floatRef}
            className={`${styles.tile} ${styles.floating}`}
            style={{ width: drag.width, height: drag.height }}
            aria-hidden
          >
            <CardBody id={drag.id} value={data.values[drag.id]} ctx={ctx} />
          </div>,
          document.body,
        )}
    </section>
  );
}
