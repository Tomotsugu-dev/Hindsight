import type { Ref } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { MAX_CARDS, MIN_CARDS } from "../../state/insightCards";
import type { CardId, CardValue } from "../../utils/insightCards";
import type { InsightMode, InsightScope } from "../../utils/insightWindows";
import { useDurationFormatter } from "../../utils/duration";
import { CardValueView } from "./CardValue";
import { cardDesc, cardLabel, cardName, type CardContext } from "./cardText";
import styles from "./InsightTiles.module.css";

const K = "components.insightCards";

/** The bar over the cards while editing: the mode switch, reset, and done. */
export function EditBar({
  scope,
  mode,
  onMode,
  onReset,
  onDone,
  doneRef,
  modeRef,
}: {
  scope: InsightScope;
  mode: InsightMode;
  onMode: (mode: InsightMode) => void;
  onReset: () => void;
  onDone: () => void;
  doneRef: Ref<HTMLButtonElement>;
  modeRef: Ref<HTMLDivElement>;
}) {
  const { t } = useTranslation();
  return (
    <>
      <div className={styles.editBar}>
        <span className={styles.editTitle}>{t(`${K}.edit`)}</span>
        <div className={styles.editActions}>
          <button type="button" className={`${styles.btn} ${styles.btnQuiet}`} onClick={onReset}>
            {t(`${K}.reset`)}
          </button>
          <button
            ref={doneRef}
            type="button"
            className={`${styles.btn} ${styles.btnPrimary}`}
            onClick={onDone}
          >
            {t(`${K}.done`)}
          </button>
        </div>
      </div>
      <div className={styles.modeRow}>
        <span className={styles.modeLabel}>{t(`${K}.modeLabel`)}</span>
        {scope === "today" ? (
          <span>{t(`${K}.dayOnlySame`)}</span>
        ) : (
          <div ref={modeRef} className={styles.seg} role="group" aria-label={t(`${K}.modeLabel`)}>
            {(["same", "rolling"] as const).map((m) => (
              <button
                key={m}
                type="button"
                data-mode={m}
                aria-pressed={mode === m}
                onClick={() => onMode(m)}
              >
                {t(`${K}.mode.${m}`)}
              </button>
            ))}
          </div>
        )}
      </div>
    </>
  );
}

/** The cards not on the page, each with what it covers and its value right now. */
export function CardLibrary({
  ids,
  placed,
  ctx,
  values,
  onAdd,
  drop,
  dropName,
  landing,
}: {
  ids: readonly CardId[];
  placed: number;
  ctx: CardContext;
  values: Partial<Record<CardId, CardValue | null>>;
  onAdd: (id: CardId) => void;
  /** A card from the row is dragged over the library: it will be removed, or it cannot be. */
  drop?: "remove" | "blocked";
  dropName?: string;
  /** The card the dragged copy is flying back to; it is drawn once the copy lands. */
  landing?: CardId | null;
}) {
  const { t } = useTranslation();
  const duration = useDurationFormatter();
  const full = placed >= MAX_CARDS;
  return (
    <section
      className={styles.tray}
      data-library=""
      data-drop={drop}
      aria-label={t(`${K}.library`)}
    >
      <div className={styles.trayHead}>
        <h2>{t(`${K}.library`)}</h2>
        <span className={styles.count}>
          <Trans i18nKey={`${K}.placed`} values={{ count: placed }} components={{ b: <b /> }} />
          {" · "}
          {t(`${K}.limit`, { min: MIN_CARDS, max: MAX_CARDS })}
        </span>
      </div>
      {drop && (
        <p className={styles.dropNote}>
          {drop === "remove"
            ? t(`${K}.dropRemove`, { name: dropName })
            : t(`${K}.dropBlocked`, { count: MIN_CARDS })}
        </p>
      )}
      {ids.length > 0 ? (
        <div className={styles.trayList}>
          {ids.map((id) => {
            const name = cardName(t, id, ctx.scope);
            return (
              <button
                key={id}
                type="button"
                className={styles.pick}
                data-pick={id}
                data-landing={landing === id ? "" : undefined}
                aria-disabled={full || undefined}
                title={full ? t(`${K}.atMax`, { count: MAX_CARDS }) : undefined}
                aria-label={t(`${K}.add`, { name })}
                onClick={() => onAdd(id)}
              >
                <span className={styles.pickTop}>
                  <span className={styles.pickName}>{name}</span>
                  <Plus size={16} className={styles.pickPlus} aria-hidden />
                </span>
                <span className={styles.pickDesc}>{cardDesc(t, id, ctx)}</span>
                <span className={styles.pickPrev}>
                  <CardValueView value={values[id]} ctx={ctx} small />
                  <span className={styles.pickPrevLabel}>
                    {cardLabel(t, id, ctx, values[id], duration)}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      ) : (
        <p className={styles.trayEmpty}>{t(`${K}.libraryEmpty`)}</p>
      )}
    </section>
  );
}
