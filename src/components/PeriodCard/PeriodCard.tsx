import type { ReactNode, RefObject } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useMouseGlow } from "../../hooks/useMouseGlow";
import styles from "./PeriodCard.module.css";

interface PeriodCardProps {
  /** 卡片小标题（如"时段分布" / "每日时长") */
  title: string;
  /** 中央 pill 显示的当前周期文案（如"今天" / "本周" / "本月"） */
  pillLabel: string;
  /** pill hover/title 提示（仅 offset !== 0 时给） */
  pillTooltip?: string;
  /** 上一/下一按钮的无障碍 label */
  prevAriaLabel: string;
  nextAriaLabel: string;
  /** 当前 offset：用于决定 pill 是否可点（偏离当前才可点回） */
  offset: number;
  /** 是否在过渡中（按钮 disable） */
  transitioning: boolean;
  /** 滑动动画的临时位移（由 usePeriodNavigation 给） */
  delta: number;
  /** swipeFrame 的容器 ref（usePeriodNavigation 用它读 clientWidth） */
  frameRef: RefObject<HTMLDivElement | null>;
  /** 能否往后翻 */
  canGoForward: boolean;
  onPrev: () => void;
  onNext: () => void;
  onJumpToCurrent: () => void;
  /** 卡片头右侧 dayNav 之前的内容（一般是 DevicePicker） */
  rightExtras?: ReactNode;
  /** 卡片头左侧 title 旁边的内容（如 ViewToggle 切「时段 / 占比」） */
  headLeftExtras?: ReactNode;
  /** 卡片底部内容（一般是 PeriodLegend） */
  footer?: ReactNode;
  /** 三个 slide 的渲染节点，按 [前一期, 当前, 后一期] 顺序传入 */
  slides: [ReactNode, ReactNode, ReactNode];
}

/**
 * Today / Week / Month 三页共用的"图表卡片"：
 * cardHead（标题 + 右侧 picker + 日期导航三按钮）+ 滑动 carousel + 可选 footer。
 *
 * 滑动状态机抽到 [usePeriodNavigation] hook，本组件只做渲染。
 */
export function PeriodCard({
  title,
  pillLabel,
  pillTooltip,
  prevAriaLabel,
  nextAriaLabel,
  offset,
  transitioning,
  delta,
  frameRef,
  canGoForward,
  onPrev,
  onNext,
  onJumpToCurrent,
  rightExtras,
  headLeftExtras,
  footer,
  slides,
}: PeriodCardProps) {
  const { ref: prevBtnRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: pillRef } = useMouseGlow<HTMLButtonElement>();
  const { ref: nextBtnRef } = useMouseGlow<HTMLButtonElement>();

  const pillClickable = offset !== 0;

  return (
    <section className={styles.card}>
      {/* 卡片头：title 单独一行（行 1），controls（ViewToggle + DevicePicker + day-nav）一行（行 2）
          —— 横向单行时四个东西挤一起，每个都被压窄；拆成两行让 title 喘口气，
             ViewToggle / day-nav 各自有完整宽度 */}
      <header className={styles.cardHead}>
        <div className={styles.cardTitleRow}>
          <h2 className={styles.cardTitle}>{title}</h2>
        </div>

        <div className={styles.cardControlsRow}>
          <div className={styles.cardHeadLeft}>{headLeftExtras}</div>

          <div className={styles.headRight}>
          {rightExtras}

          <div className={styles.dayNav}>
            <button
              ref={prevBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={onPrev}
              disabled={transitioning}
              aria-label={prevAriaLabel}
              title={prevAriaLabel}
            >
              <ChevronLeft size={14} strokeWidth={1.75} />
            </button>

            <button
              ref={pillRef}
              type="button"
              className={`${styles.dayPill} ${pillClickable ? styles.dayPillClickable : ""} glow`}
              onClick={onJumpToCurrent}
              disabled={!pillClickable || transitioning}
              title={pillClickable ? pillTooltip : undefined}
            >
              {pillLabel}
            </button>

            <button
              ref={nextBtnRef}
              type="button"
              className={`${styles.navBtn} glow`}
              onClick={onNext}
              disabled={!canGoForward || transitioning}
              aria-label={nextAriaLabel}
              title={nextAriaLabel}
            >
              <ChevronRight size={14} strokeWidth={1.75} />
            </button>
          </div>
          </div>
        </div>
      </header>

      <div className={styles.swipeFrame} ref={frameRef}>
        <div
          className={`${styles.swipeTrack} ${transitioning ? styles.swipeAnimated : ""}`}
          style={{ transform: `translate3d(calc(-100% + ${delta}px), 0, 0)` }}
        >
          {slides.map((node, i) => (
            <div className={styles.slide} key={i}>
              {node}
            </div>
          ))}
        </div>
      </div>

      {footer}
    </section>
  );
}
