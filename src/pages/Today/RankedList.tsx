import type { ReactNode } from "react";
import styles from "./RankedList.module.css";

export interface RankedItem {
  /** 唯一 key */
  id: string;
  /** 主显示名 */
  name: string;
  /** 副标签（如分类名）— 可选 */
  subtitle?: string;
  /** 颜色（用于条形和默认圆点；当 leading 提供时不再画圆点） */
  color: string;
  /** 该项分钟数 */
  minutes: number;
  /** 自定义前置图标（如真实 app 图标）；不传则用 color 画圆点 */
  leading?: ReactNode;
  /** 文字与进度条之间的额外内容（如分类的常用应用堆叠图标） */
  extras?: ReactNode;
}

interface RankedListProps {
  items: RankedItem[];
  /** 用于计算条形百分比；不传则取最大项作为 100% */
  totalMinutes?: number;
}

function fmtTime(minutes: number): string {
  if (minutes < 60) return `${minutes} 分`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h} 小时` : `${h}h ${m}m`;
}

export function RankedList({ items, totalMinutes }: RankedListProps) {
  const denom = totalMinutes ?? Math.max(...items.map((i) => i.minutes), 1);

  return (
    <ol className={styles.list}>
      {items.map((item, idx) => {
        const pct = (item.minutes / denom) * 100;
        return (
          <li key={item.id} className={styles.row}>
            <span className={styles.rank}>{idx + 1}</span>
            {item.leading ?? (
              <span
                className={styles.dot}
                style={{ background: item.color }}
                aria-hidden
              />
            )}
            <div className={styles.text}>
              <div className={styles.name}>{item.name}</div>
              {item.subtitle ? (
                <div className={styles.subtitle}>{item.subtitle}</div>
              ) : null}
            </div>
            <div className={styles.extras}>{item.extras}</div>
            <div className={styles.barWrap}>
              <div
                className={styles.barFill}
                style={{
                  width: `${pct}%`,
                  background: `color-mix(in oklab, ${item.color} 75%, transparent)`,
                }}
              />
            </div>
            <span className={styles.time}>{fmtTime(item.minutes)}</span>
          </li>
        );
      })}
    </ol>
  );
}
