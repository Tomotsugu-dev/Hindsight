import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useAutoAnimate } from "@formkit/auto-animate/react";
import { ChevronDown, ChevronUp } from "lucide-react";
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
  /** 仅 app 排行用：该 app 的小分类 id，便于按 super-category 过滤 */
  categoryId?: string;
}

interface RankedListProps {
  items: RankedItem[];
  /** 用于计算条形百分比；不传则取最大项作为 100% */
  totalMinutes?: number;
  /** 默认显示的最大行数；超出则折叠并显示展开按钮。null/0 = 不折叠。 */
  defaultLimit?: number;
}

export function RankedList({ items, totalMinutes, defaultLimit = 10 }: RankedListProps) {
  const { t } = useTranslation();
  const denom = totalMinutes ?? Math.max(...items.map((i) => i.minutes), 1);
  // 切日 / 切设备 / 选时段 → items 重排时，让 row 平滑滑到新位置；
  // 新增 / 消失 fade。`key={item.id}` 是稳定 key，库据此识别 reorder vs add/remove。
  const [listRef] = useAutoAnimate<HTMLOListElement>({
    duration: 250,
    easing: "ease-in-out",
  });

  // 折叠/展开状态。每次切日 / 切设备时不主动 reset——用户的展开偏好沿用。
  const [expanded, setExpanded] = useState(false);
  const canExpand = defaultLimit > 0 && items.length > defaultLimit;
  const visibleItems = canExpand && !expanded ? items.slice(0, defaultLimit) : items;

  // 排行行的时长格式化 —— 复用 common.duration.* 资源
  const fmtTime = (minutes: number): string => {
    if (minutes < 60) return t("common.duration.minutesPlain", { count: minutes });
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m === 0
      ? t("common.duration.hoursPlain", { count: h })
      : t("common.duration.hoursAndMinutesShort", { hours: h, minutes: m });
  };

  return (
    <ol ref={listRef} className={styles.list}>
      {visibleItems.map((item, idx) => {
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
      {canExpand && (
        <li key="__toggle" className={styles.toggleRow}>
          <button
            type="button"
            className={styles.toggleBtn}
            onClick={() => setExpanded((v) => !v)}
            aria-label={
              expanded
                ? t("components.rankedList.collapse")
                : t("components.rankedList.expand", { count: items.length - defaultLimit })
            }
            title={
              expanded
                ? t("components.rankedList.collapse")
                : t("components.rankedList.expand", { count: items.length - defaultLimit })
            }
          >
            {expanded ? (
              <ChevronUp size={16} strokeWidth={2} />
            ) : (
              <ChevronDown size={16} strokeWidth={2} />
            )}
          </button>
        </li>
      )}
    </ol>
  );
}
