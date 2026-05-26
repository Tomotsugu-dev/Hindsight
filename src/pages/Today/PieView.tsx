import { useState, type CSSProperties } from "react";
import { useDurationFormatter } from "../../utils/duration";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import type { BreakdownSlice } from "../../hooks/useSuperCategoryBreakdown";
import { withViewTransition } from "../../utils/viewTransition";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { Donut } from "./Donut";
import styles from "./PieView.module.css";

interface Props {
  slices: BreakdownSlice[];
  total: number;
  /** false 时禁用 hover/click + 不挂 view-transition-name（给 day-swipe 的 prev/next slide） */
  interactive?: boolean;
  /** 点击切片或行 → 通知父切换到 PieDrillDetail */
  onDrill?: (superId: string) => void;
}

/**
 * 占比视图的「列表层」：左 Donut + 右 super-cat 行。
 * - hover 一行或一切片 → 双向高亮（圆环 stroke 加宽 + 列表行变白底紫边 + 其他 dim）
 * - 点击 → `withViewTransition(onDrill(id))`，父切到 PieDrillDetail，圆环 morph
 * - interactive=false（prev/next slide）：渲染但所有交互禁用，view-transition-name 不挂
 *
 * Idle 时圆心放 top-1 大类名做 watermark（不显示总时长，那个信息已经在页 header
 * "2026-05-26 · 已采集 5 小时 19 分" 里）。Hover 时 watermark 让位给该切片的 pct/时长/名。
 */
export function PieView({ slices, total, interactive = true, onDrill }: Props) {
  const fmtHM = useDurationFormatter();
  const [hover, setHover] = useState<string | null>(null);

  if (slices.length === 0 || total <= 0) {
    return (
      <div className={styles.body}>
        <div className={styles.empty}>
          <EmptyHint />
        </div>
      </div>
    );
  }

  const activeId = interactive ? hover : null;
  const hovered = activeId ? slices.find((s) => s.id === activeId) : null;
  const top = slices[0]; // 已按 minutes 降序

  const handleClick = (id: string) => {
    if (!interactive || !onDrill) return;
    withViewTransition(() => onDrill(id));
  };

  return (
    <div
      className={styles.body}
      style={
        interactive
          ? ({ viewTransitionName: "pie-body" } as CSSProperties)
          : undefined
      }
    >
      <div className={styles.donutWrap}>
        <Donut
          size={180}
          thickness={20}
          segments={slices.map((s) => ({ id: s.id, color: s.color, value: s.minutes }))}
          total={total}
          activeId={activeId}
          onHover={interactive ? setHover : undefined}
          onClick={interactive ? handleClick : undefined}
          /* Hover → pct 大字 + 时长 + 名（tooltip 等价）；Idle → 仅 top-1 大类名做 watermark */
          centerTitle={hovered ? fmtHM(hovered.minutes) : undefined}
          centerSub={hovered ? hovered.name : top.name}
          centerPctTop={
            hovered ? `${Math.round((hovered.minutes / total) * 100)}%` : undefined
          }
          viewTransitionName={interactive ? "super-donut" : undefined}
        />
      </div>

      <ul className={styles.list}>
        {slices.map((s) => {
          const pct = Math.round((s.minutes / total) * 100);
          const isActive = activeId === s.id;
          const dim = activeId !== null && !isActive;
          const Icon = resolveCategoryIcon(s.icon);
          return (
            <li key={s.id}>
              <button
                type="button"
                className={styles.row}
                style={{ "--row-color": s.color } as CSSProperties}
                data-active={isActive || undefined}
                data-dim={dim || undefined}
                onMouseEnter={() => interactive && setHover(s.id)}
                onMouseLeave={() => interactive && setHover(null)}
                onClick={() => handleClick(s.id)}
                disabled={!interactive}
              >
                <span className={styles.iconWrap} aria-hidden>
                  <Icon size={14} strokeWidth={2} />
                </span>
                <span className={styles.name}>{s.name}</span>
                <span className={styles.barWrap}>
                  <span
                    className={styles.barFill}
                    style={{ width: `${pct}%` }}
                  />
                </span>
                <span className={styles.num}>
                  <span className={styles.pct}>{pct}%</span>
                  <span className={styles.numSep}>·</span>
                  <span className={styles.time}>{fmtHM(s.minutes)}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
