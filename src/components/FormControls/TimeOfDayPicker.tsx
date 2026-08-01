import { useRef, useState } from "react";
import { type AiSegment } from "../../api/hindsight";
import { resolveSegmentChip } from "../../utils/segmentColor";
import {
  clampHour,
  formatHour,
  hourFromRatio,
  nearestIndex,
  parseHour,
} from "./timeOfDayMath";
import styles from "./TimeOfDayPicker.module.css";

interface TimeOfDayPickerProps {
  /** "HH:MM" 列表(**保持添加顺序**);本控件按整点粒度工作,分钟位忽略 */
  values: string[];
  onChange: (next: string[]) => void;
  ariaLabel: string;
  /** 时段底色(与「时段划分」段条同源上色);不传 = 素色轨道 */
  bands?: AiSegment[];
}

/**
 * 一天内时刻选择:24 小时横轴上的一个或多个标记,整点粒度。
 * 拖拽/点击移动**离落点最近**的标记(多标记滑轨的标准交互);
 * 键盘 ←/→ 移动最近一次拖过的标记,Home/End 到 0/23。
 * 标记的增删由宿主负责(定时计划行的 +/− 按钮),本控件只管摆位。
 * 视觉与「时段划分」的 24h 段条同语言(胶囊轨道 + 底部刻度 + 悬浮感)。
 */
export function TimeOfDayPicker({
  values,
  onChange,
  ariaLabel,
  bands,
}: TimeOfDayPickerProps) {
  const hours = values.map(parseHour);
  const barRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  // 当前操作的标记下标(拖拽锁定 + 键盘目标);越界时回落最后一个
  const [activeIdx, setActiveIdx] = useState(0);
  const active = Math.min(activeIdx, Math.max(0, hours.length - 1));

  const setHourAt = (idx: number, h: number) => {
    if (hours[idx] === h) return;
    // 不许两个标记落同一小时:后端会把重复时刻去重,标记会"凭空消失";
    // 拖到被占的小时就停在原地(多滑块的碰撞即卡位交互)
    if (hours.some((v, i) => i !== idx && v === h)) return;
    const next = [...values];
    next[idx] = formatHour(h);
    onChange(next);
  };

  const ratioFrom = (clientX: number): number | null => {
    const el = barRef.current;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (r.width <= 0) return null;
    return (clientX - r.left) / r.width;
  };

  return (
    <div className={styles.wrap}>
      <div
        ref={barRef}
        className={styles.bar}
        role="slider"
        aria-label={ariaLabel}
        aria-valuemin={0}
        aria-valuemax={23}
        aria-valuenow={hours[active] ?? 0}
        aria-valuetext={values.join(" / ")}
        tabIndex={0}
        data-dragging={dragging || undefined}
        onPointerDown={(e) => {
          const ratio = ratioFrom(e.clientX);
          if (ratio == null || hours.length === 0) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          // 锁定离落点最近的标记,整段拖拽都只动它
          const idx = nearestIndex(hours, ratio * 24 - 0.5);
          setActiveIdx(idx);
          setDragging(true);
          setHourAt(idx, hourFromRatio(ratio));
        }}
        onPointerMove={(e) => {
          if (!dragging) return;
          const ratio = ratioFrom(e.clientX);
          if (ratio != null) setHourAt(active, hourFromRatio(ratio));
        }}
        onPointerUp={() => setDragging(false)}
        onPointerCancel={() => setDragging(false)}
        onKeyDown={(e) => {
          if (hours.length === 0) return;
          if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
            e.preventDefault();
            setHourAt(active, clampHour((hours[active] ?? 0) - 1));
          } else if (e.key === "ArrowRight" || e.key === "ArrowUp") {
            e.preventDefault();
            setHourAt(active, clampHour((hours[active] ?? 0) + 1));
          } else if (e.key === "Home") {
            e.preventDefault();
            setHourAt(active, 0);
          } else if (e.key === "End") {
            e.preventDefault();
            setHourAt(active, 23);
          }
        }}
      >
        {/* 剪裁层:色带裁进圆角;handle/气泡在层外不被剪 */}
        <div className={styles.clip} aria-hidden>
          {bands?.map((b, i) => (
            <div
              key={i}
              className={styles.band}
              style={{
                left: `${(b.startHour / 24) * 100}%`,
                width: `${((b.endHour - b.startHour) / 24) * 100}%`,
                background: resolveSegmentChip(b).background,
              }}
            />
          ))}
        </div>
        {hours.map((h, i) => (
          <div
            key={i}
            className={styles.handle}
            data-active={i === active || undefined}
            style={{ left: `${((h + 0.5) / 24) * 100}%` }}
          >
            <span className={styles.bubble}>{formatHour(h)}</span>
          </div>
        ))}
      </div>
      <div className={styles.ticks} aria-hidden>
        {[0, 6, 12, 18, 24].map((h) => (
          <span
            key={h}
            className={styles.tick}
            style={{ left: `${(h / 24) * 100}%` }}
          >
            <span className={styles.tickMark} />
            <span className={styles.tickLabel}>{h}</span>
          </span>
        ))}
      </div>
    </div>
  );
}
