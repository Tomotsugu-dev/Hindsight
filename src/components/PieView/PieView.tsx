import { useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Mouse } from "lucide-react";
import { useDurationFormatter } from "../../utils/duration";
import { EmptyHint } from "../../components/EmptyHint/EmptyHint";
import type { BreakdownSlice } from "../../hooks/useSuperCategoryBreakdown";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { useCategories } from "../../state/categories";
import { useIsDark } from "../../hooks/useTheme";
import { useWheelSteps } from "../../hooks/useWheelSteps";
import type { PieDepth } from "../../state/statsView";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { withViewTransition } from "../../utils/viewTransition";
import { ActivityRings } from "./ActivityRings";
import { Donut } from "./Donut";
import type { RingFocusApp } from "./ringItems";
import styles from "./PieView.module.css";

/** 同心环一次露出几圈；小类更多时在圆环上滚动换一批（测试用 4，定下来再调） */
const RINGS = 4;

interface Props {
  slices: BreakdownSlice[];
  total: number;
  /** false 时禁用 hover/click + 不挂 view-transition-name（给 day-swipe 的 prev/next slide） */
  interactive?: boolean;
  /** 父侧 pin 住的切片 id（drill 状态）→ 持久高亮，hover 仍可临时覆盖 */
  pinnedId?: string | null;
  /** 点击切片或行：父侧 toggle drillId（点同一片取消，点新片切换） */
  onDrill?: (superId: string) => void;
  /** 鼠标停在下方应用列表的某个应用上：画成它所属大类的单环，突出这个应用 */
  focusApp?: RingFocusApp | null;
  /** 父侧选中的小类 id；下方应用列表只列它的应用 */
  pickedCatId?: string | null;
  /** 点圆环下面的小类、或圆环上的一圈 / 一段：父侧 toggle 选中的小类 */
  onCatPick?: (catId: string) => void;
  /** 显示哪一层；prev/next slide 也要传，滑动时跟当前那张一样 */
  depth?: PieDepth;
  /** 在环上滚动换层：交给父侧 */
  onDepthChange?: (depth: PieDepth) => void;
}

/**
 * 占比视图，两层，在环上用滚轮切换：
 * - 只有大类的环 + 右边大类行。往下滚进入下一层
 * - 小类同心环 + 右边大类行 + 下面一排小类。已经露出第一名时往上滚回到上一层
 *
 * 停在大类行或小类上只强调，不换圆环的结构。点大类钉住它（父侧 onDrill），点小类选中它（父侧 onCatPick）。
 * interactive=false（prev/next slide）：渲染但所有交互禁用，view-transition-name 不挂。
 */
export function PieView({
  slices,
  total,
  interactive = true,
  pinnedId,
  onDrill,
  focusApp = null,
  pickedCatId = null,
  onCatPick,
  depth = "supers",
  onDepthChange,
}: Props) {
  const { t } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();
  const { getCategory } = useCategories();
  const [hover, setHover] = useState<string | null>(null);
  const [tagHover, setTagHover] = useState<string | null>(null);
  const donutRef = useRef<HTMLDivElement>(null);
  const empty = slices.length === 0 || total <= 0;

  // 换层时环从一个样子变形成另一个（两层的 SVG 挂同一个 view-transition-name）
  const changeDepth = (next: PieDepth) => withViewTransition(() => onDepthChange?.(next));
  // 只有大类的那一层：在环上往下滚进入小类同心环；往上滚不拦截，页面照常滚。
  // 没数据时环不在，数据到了 enabled 变了才挂得上监听
  useWheelSteps(
    donutRef,
    interactive && !empty && depth === "supers" && !!onDepthChange,
    (down) => down,
    (steps) => {
      if (steps <= 0) return false;
      changeDepth("cats");
      return true;
    },
  );

  if (empty) {
    return (
      <div className={styles.body}>
        <div className={styles.empty}>
          <EmptyHint />
        </div>
      </div>
    );
  }

  const superOf = (catId: string | null) =>
    catId ? (slices.find((s) => s.cats.some((c) => c.id === catId))?.id ?? null) : null;
  // 大类行亮哪个：突出应用时是它所属的大类；否则 hover 优先，再到钉住的大类、选中小类的大类
  const activeId = interactive
    ? focusApp
      ? superOf(focusApp.categoryId)
      : (hover ?? pinnedId ?? superOf(pickedCatId))
    : null;

  const handleClick = (id: string) => {
    if (!interactive || !onDrill) return;
    onDrill(id);
  };

  // 下面那排小类：钉住大类时只列它下面的，占比按占这个大类算；否则列全部，按占全部时间算
  const pinnedSlice = interactive && pinnedId ? (slices.find((s) => s.id === pinnedId) ?? null) : null;
  const tagOf = pinnedSlice ? pinnedSlice.minutes : total;
  const tags = (pinnedSlice ? [pinnedSlice] : slices)
    .flatMap((s) => s.cats.map((cat) => ({ ...cat, superId: s.id })))
    .filter((cat) => cat.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes);
  // 小类亮哪个：突出应用时是它的小类，否则是选中的
  const litTagId = interactive ? (focusApp ? focusApp.categoryId : pickedCatId) : null;

  // 只有大类的那一层，圆心：停在应用上时是这个应用；否则是亮着的大类；都没有时是总时长
  const hovered = activeId ? slices.find((s) => s.id === activeId) : null;
  const pctOf = (minutes: number) => `${Math.round((minutes / total) * 100)}%`;
  const appName = focusApp
    ? focusApp.name.length > 10
      ? `${focusApp.name.slice(0, 9)}…`
      : focusApp.name
    : null;

  return (
    <div
      className={styles.body}
      data-depth={depth}
      style={
        interactive
          ? ({ viewTransitionName: "pie-body" })
          : undefined
      }
    >
      <div ref={donutRef} className={styles.donutWrap}>
        {depth === "cats" ? (
          <ActivityRings
            size={270}
            slices={slices}
            total={total}
            rings={RINGS}
            pinnedSuperId={interactive ? (pinnedId ?? null) : null}
            highlightSuperId={interactive ? hover : null}
            onSuperClick={interactive ? handleClick : undefined}
            highlightCatId={interactive ? tagHover : null}
            selectedCatId={interactive ? pickedCatId : null}
            onCatClick={interactive ? onCatPick : undefined}
            onZoomOut={interactive && onDepthChange ? () => changeDepth("supers") : undefined}
            focusApp={focusApp}
            interactive={interactive}
            viewTransitionName={interactive ? "super-donut" : undefined}
          />
        ) : (
          <>
            <Donut
              size={180}
              thickness={20}
              segments={slices.map((s) => ({
                id: s.id,
                color: adjustCategoryColor(s.color, isDark),
                value: s.minutes,
              }))}
              total={total}
              activeId={activeId}
              onHover={interactive ? setHover : undefined}
              onClick={interactive ? handleClick : undefined}
              centerTitle={fmtHM(focusApp ? focusApp.minutes : hovered ? hovered.minutes : total)}
              centerSub={appName ?? hovered?.name}
              centerPctTop={
                focusApp ? pctOf(focusApp.minutes) : hovered ? pctOf(hovered.minutes) : undefined
              }
              viewTransitionName={interactive ? "super-donut" : undefined}
            />
            {/* 这一层看不出往下滚还有一层，放个提示；prev/next slide 也放，滑动时高度一样 */}
            <p className={styles.wheelHint}>
              <Mouse size={12} strokeWidth={2} aria-hidden />
              {t("today.pie.wheelHint")}
            </p>
          </>
        )}
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
                style={
                  { "--row-color": adjustCategoryColor(s.color, isDark) } as CSSProperties
                }
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

      {depth === "cats" && (
        <ul className={styles.tags}>
          {tags.map((cat) => {
            const share = Math.round((cat.minutes / tagOf) * 100);
            const isActive = litTagId === cat.id;
            // 有亮着的小类时其余变淡；否则停在某个大类行上时，不属于它的变淡
            const dim = litTagId !== null ? !isActive : hover !== null && cat.superId !== hover;
            const Icon = resolveCategoryIcon(getCategory(cat.id)?.icon);
            return (
              <li key={cat.id}>
                <button
                  type="button"
                  className={styles.tag}
                  style={
                    { "--row-color": adjustCategoryColor(cat.color, isDark) } as CSSProperties
                  }
                  data-active={isActive || undefined}
                  data-dim={dim || undefined}
                  aria-pressed={interactive ? pickedCatId === cat.id : undefined}
                  title={`${cat.name} · ${fmtHM(cat.minutes)}`}
                  onMouseEnter={() => interactive && setTagHover(cat.id)}
                  onMouseLeave={() => interactive && setTagHover(null)}
                  onFocus={() => interactive && setTagHover(cat.id)}
                  onBlur={() => interactive && setTagHover(null)}
                  onClick={() => interactive && onCatPick?.(cat.id)}
                  disabled={!interactive}
                >
                  <span className={styles.tagIcon} aria-hidden>
                    <Icon size={12} strokeWidth={2.2} />
                  </span>
                  <span className={styles.tagName}>{cat.name}</span>
                  <span className={styles.tagPct}>{share < 1 ? "<1%" : `${share}%`}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
