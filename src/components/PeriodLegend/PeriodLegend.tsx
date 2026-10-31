import { useTranslation } from "react-i18next";
import { useCategories } from "../../state/categories";
import type { BarLevel } from "../../state/statsView";
import { useSuperCategories } from "../../state/superCategories";
import { legendItems } from "../../lib/segments";
import styles from "./PeriodLegend.module.css";

interface PeriodLegendProps {
  /** 工作时段图例条的文案；不传则不渲染那条（Today 才用，Week/Month 不用） */
  workHoursLabel?: string;
  /** The level the bars are colored by. */
  level?: BarLevel;
  /** Shows the super category / category switch at the start of the legend. */
  onLevelChange?: (level: BarLevel) => void;
}

const LEVELS: BarLevel[] = ["supers", "cats"];

/** 卡片底部图例：分类色块 + 名字 (+ 可选工作时段条带)。
 *  抽自 Today/Week/Month 三页内嵌的 Legend 函数。 */
export function PeriodLegend({ workHoursLabel, level = "cats", onLevelChange }: PeriodLegendProps) {
  const { t } = useTranslation();
  const { categories } = useCategories();
  const { supers } = useSuperCategories();
  const items = legendItems(level, categories, supers, t("categories.super.orphanLabel"));
  return (
    <div className={styles.legend}>
      {onLevelChange ? (
        <div
          className={styles.level}
          role="group"
          aria-label={t("components.periodLegend.levelAria")}
        >
          {LEVELS.map((l) => (
            <button
              key={l}
              type="button"
              className={styles.levelBtn}
              data-active={level === l || undefined}
              aria-pressed={level === l}
              onClick={() => onLevelChange(l)}
            >
              {t(`components.periodLegend.${l}`)}
            </button>
          ))}
        </div>
      ) : null}
      {items.map((item) => (
        <span key={item.id} className={styles.legendItem}>
          <span
            className={styles.legendDot}
            style={{ background: item.color }}
            aria-hidden
          />
          {item.name}
        </span>
      ))}
      {workHoursLabel ? (
        <span className={styles.legendItem}>
          <span className={styles.legendBand} aria-hidden />
          {workHoursLabel}
        </span>
      ) : null}
    </div>
  );
}
