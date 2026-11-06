import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useInsightData, type InsightFocus } from "../../hooks/useInsightData";
import { useInsightCards, useInsightMode } from "../../state/insightCards";
import { useDurationFormatter } from "../../utils/duration";
import type { CardId, CardValue } from "../../utils/insightCards";
import type { InsightScope } from "../../utils/insightWindows";
import {
  cardContext,
  cardLabel,
  dateText,
  hourText,
  scopeNote,
  weekdayText,
  type CardContext,
} from "./cardText";
import styles from "./InsightTiles.module.css";

interface Props {
  scope: InsightScope;
  offset: number;
  focus: InsightFocus;
  /** The color of the drilled super category or the hovered app; the cards take it on. */
  accent?: string;
}

/** Columns of a 12-column grid: 3 and 4 cards fill one row, 5 go 3 then 2, 6 go 3 and 3. */
function spanOf(count: number, index: number): number {
  if (count === 4) return 3;
  if (count === 5) return index < 3 ? 4 : 6;
  return 4;
}

/** Whether the card starts a row, where the flat layout draws no divider. */
const startsRow = (count: number, index: number) => index === 0 || (count > 4 && index === 3);

/** The big value of a card; `small` is the preview in the card library. */
export function CardValueView({
  value,
  ctx,
  small = false,
}: {
  value: CardValue | null | undefined;
  ctx: CardContext;
  small?: boolean;
}) {
  const { t } = useTranslation();
  const duration = useDurationFormatter();
  const cls = small ? styles.preview : styles.value;
  if (!value) return <span className={`${cls} ${styles.dash}`}>—</span>;
  switch (value.kind) {
    case "delta": {
      if (value.minutes === 0) {
        return <span className={cls}>{t("components.insightCards.flat")}</span>;
      }
      const up = value.minutes > 0;
      const Arrow = up ? ArrowUp : ArrowDown;
      return (
        <span className={`${cls} ${up ? styles.up : styles.down}`}>
          <Arrow size={14} strokeWidth={2.4} aria-hidden />
          {duration(Math.abs(value.minutes))}
        </span>
      );
    }
    case "duration":
      return <span className={cls}>{duration(value.minutes)}</span>;
    case "peakHour":
      return <span className={cls}>{hourText(value.hour)}</span>;
    case "peakDay":
      // A week compared up to the same point reads best by weekday; a rolling one crosses weeks
      return (
        <span className={cls}>
          {ctx.scope === "week" && ctx.mode === "same"
            ? weekdayText(t, value.date)
            : dateText(t, value.date)}
        </span>
      );
    case "share":
      return (
        <span className={cls}>
          <span className={styles.swatch} style={{ background: value.color }} aria-hidden />
          {value.name} {value.pct}%
        </span>
      );
    case "edges":
      return (
        <span className={cls}>
          {hourText(value.start)} – {hourText(value.end)}
        </span>
      );
    case "hourCount":
      return (
        <span className={cls}>
          {t("components.insightCards.hourCount", { count: value.hours })}
        </span>
      );
    case "devices":
      return (
        <span className={`${cls} ${styles.devbar}`} aria-hidden>
          {value.shares.map((s) => (
            <span key={s.id} style={{ flex: s.pct, background: s.color ?? "var(--text-faint)" }} />
          ))}
        </span>
      );
  }
}

/** One card: its value and the label under it. */
export function CardBody({
  id,
  value,
  ctx,
}: {
  id: CardId;
  value: CardValue | null | undefined;
  ctx: CardContext;
}) {
  const { t } = useTranslation();
  const duration = useDurationFormatter();
  return (
    <>
      <CardValueView value={value} ctx={ctx} />
      <span className={styles.label}>{cardLabel(t, id, ctx, value, duration)}</span>
    </>
  );
}

/** The stats cards under the share view: 3 to 6 cards, each counted over the same stretch. */
export function InsightTiles({ scope, offset, focus, accent }: Props) {
  const { t } = useTranslation();
  const cards = useInsightCards(scope);
  const mode = useInsightMode(scope);
  const data = useInsightData({ scope, offset, mode, cards, focus });
  const ctx = cardContext(scope, mode, data.windows, data.now, focus);
  const style = accent ? ({ "--tile-accent": accent } as CSSProperties) : undefined;

  return (
    <section className={styles.area} aria-label={t("components.insightCards.listLabel")}>
      <div className={styles.head}>
        <p className={styles.note}>{scopeNote(t, ctx, data.windows, data.now)}</p>
      </div>
      <div className={styles.tiles} role="list" data-drill={accent ? "" : undefined} style={style}>
        {cards.map((id, i) => (
          <div
            key={id}
            role="listitem"
            className={styles.tile}
            data-row-start={startsRow(cards.length, i) ? "" : undefined}
            style={{ "--span": spanOf(cards.length, i) } as CSSProperties}
          >
            <CardBody id={id} value={data.values[id]} ctx={ctx} />
          </div>
        ))}
      </div>
    </section>
  );
}
