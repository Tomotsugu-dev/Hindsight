import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useDurationFormatter } from "../../utils/duration";
import type { CardId, CardValue } from "../../utils/insightCards";
import { cardLabel, dateText, hourText, weekdayText, type CardContext } from "./cardText";
import styles from "./InsightTiles.module.css";

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
