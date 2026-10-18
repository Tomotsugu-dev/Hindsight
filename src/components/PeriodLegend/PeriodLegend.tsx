import { useCategories } from "../../state/categories";
import styles from "./PeriodLegend.module.css";

interface PeriodLegendProps {
  /** 工作时段图例条的文案；不传则不渲染那条（Today 才用，Week/Month 不用） */
  workHoursLabel?: string;
}

/** 卡片底部图例：分类色块 + 名字 (+ 可选工作时段条带)。
 *  抽自 Today/Week/Month 三页内嵌的 Legend 函数。 */
export function PeriodLegend({ workHoursLabel }: PeriodLegendProps) {
  const { categories } = useCategories();
  return (
    <div className={styles.legend}>
      {categories.filter((c) => c.id !== "hidden").map((c) => (
        <span key={c.id} className={styles.legendItem}>
          <span
            className={styles.legendDot}
            style={{ background: c.color }}
            aria-hidden
          />
          {c.name}
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
