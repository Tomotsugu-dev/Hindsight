import { useLayoutEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import type { AiSegment } from "../../../api/hindsight";
import styles from "./SegmentList.module.css";

interface Props {
  segments: AiSegment[];
  onChange: (next: AiSegment[]) => void;
}

type DragKind = "left-edge" | "between" | "right-edge";
interface DragState {
  kind: DragKind;
  /** between: 左侧段在 sorted 数组里的下标；left/right-edge 不用 */
  index: number;
}

const TICKS = [0, 3, 6, 9, 12, 15, 18, 21, 24];

// —— 按"一天的色温"配色：深夜 → 黎明 → 上午 → 下午 → 黄昏 → 夜。
//    全部走 pastel 区间（L 68-86），早亮晚暗在这个区间内渐变。 ——
type HSL = { h: number; s: number; l: number };
const COLOR_STOPS: Array<{ hour: number; hsl: HSL }> = [
  { hour: 0, hsl: { h: 228, s: 50, l: 70 } }, // 深夜：柔蓝紫
  { hour: 5, hsl: { h: 258, s: 55, l: 76 } }, // 黎明前：薰衣草
  { hour: 7.5, hsl: { h: 28, s: 78, l: 86 } }, // 清晨：嫩桃（最亮）
  { hour: 10.5, hsl: { h: 45, s: 75, l: 82 } }, // 上午：暖黄
  { hour: 14, hsl: { h: 22, s: 65, l: 80 } }, // 午后：奶橘
  { hour: 17, hsl: { h: 345, s: 62, l: 82 } }, // 黄昏：浅玫瑰
  { hour: 20, hsl: { h: 275, s: 45, l: 76 } }, // 夜初：丁香紫
  { hour: 23, hsl: { h: 228, s: 50, l: 68 } }, // 深夜：略深蓝紫
  { hour: 24, hsl: { h: 228, s: 50, l: 70 } },
];

function lerpHue(a: number, b: number, t: number): number {
  const diff = (((b - a + 540) % 360) - 180);
  return (a + diff * t + 360) % 360;
}

function hourColor(h: number): HSL {
  const clamped = Math.max(0, Math.min(24, h));
  for (let i = 0; i < COLOR_STOPS.length - 1; i++) {
    const a = COLOR_STOPS[i];
    const b = COLOR_STOPS[i + 1];
    if (clamped >= a.hour && clamped <= b.hour) {
      const t = (clamped - a.hour) / (b.hour - a.hour);
      return {
        h: lerpHue(a.hsl.h, b.hsl.h, t),
        s: a.hsl.s + (b.hsl.s - a.hsl.s) * t,
        l: a.hsl.l + (b.hsl.l - a.hsl.l) * t,
      };
    }
  }
  return COLOR_STOPS[0].hsl;
}

function hslStr({ h, s, l }: HSL): string {
  return `hsl(${h.toFixed(0)}, ${s.toFixed(0)}%, ${l.toFixed(0)}%)`;
}

/** L > 60 视作浅色，正文用深字；否则白字 */
function isLightBg(hsl: HSL): boolean {
  return hsl.l > 60;
}

function fmt(h: number): string {
  // 段都是整点，省略 ":00"，给时间文字腾大一些
  return String(h).padStart(2, "0");
}

export function SegmentList({ segments, onChange }: Props) {
  const barRef = useRef<HTMLDivElement | null>(null);
  // 仅触发重渲染用，宽度真值通过 getBoundingClientRect 实时取
  const [, setTick] = useState(0);
  const [editing, setEditing] = useState<{ index: number; value: string } | null>(null);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const dragRef = useRef<DragState | null>(null);

  // hover 在 segment / X 之间切换时 80ms 防抖，避免穿越间隙时闪一下
  const enterSegment = (i: number) => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
    setHoverIndex(i);
  };
  const leaveSegment = () => {
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = window.setTimeout(() => {
      setHoverIndex(null);
      hoverTimerRef.current = null;
    }, 80);
  };

  useLayoutEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setTick((t) => t + 1));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 始终按 startHour 排好序展示，避免上层数据顺序乱掉时显示错乱
  const sorted = [...segments]
    .filter((s) => s.endHour > s.startHour)
    .sort((a, b) => a.startHour - b.startHour);

  const pct = (h: number) => `${(h / 24) * 100}%`;

  const startDrag = (e: React.PointerEvent, drag: DragState) => {
    if (!barRef.current) return;
    e.preventDefault();
    e.stopPropagation();
    dragRef.current = drag;
    document.body.style.cursor = "ew-resize";

    const onMove = (ev: PointerEvent) => {
      if (!barRef.current || !dragRef.current) return;
      const rect = barRef.current.getBoundingClientRect();
      const x = ev.clientX - rect.left;
      const newHour = Math.max(0, Math.min(24, Math.round((x / rect.width) * 24)));
      apply(dragRef.current, newHour);
    };
    const onUp = () => {
      dragRef.current = null;
      document.body.style.cursor = "";
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const apply = (drag: DragState, newHour: number) => {
    const next = sorted.slice();
    if (drag.kind === "left-edge") {
      const seg = next[0];
      const h = Math.max(0, Math.min(seg.endHour - 1, newHour));
      next[0] = { ...seg, startHour: h };
    } else if (drag.kind === "right-edge") {
      const seg = next[next.length - 1];
      const h = Math.max(seg.startHour + 1, Math.min(24, newHour));
      next[next.length - 1] = { ...seg, endHour: h };
    } else {
      const left = next[drag.index];
      const right = next[drag.index + 1];
      const h = Math.max(left.startHour + 1, Math.min(right.endHour - 1, newHour));
      next[drag.index] = { ...left, endHour: h };
      next[drag.index + 1] = { ...right, startHour: h };
    }
    onChange(next);
  };

  const removeAt = (i: number) => {
    if (sorted.length === 1) {
      onChange([]);
      return;
    }
    const next = sorted.slice();
    const seg = next[i];
    if (i + 1 < next.length) {
      // 范围并入右邻居
      next[i + 1] = { ...next[i + 1], startHour: seg.startHour };
    } else {
      // 没有右邻居：并入左邻居
      next[i - 1] = { ...next[i - 1], endHour: seg.endHour };
    }
    next.splice(i, 1);
    onChange(next);
  };

  const addSegment = () => {
    if (sorted.length === 0) {
      onChange([{ label: "新段", startHour: 9, endHour: 12 }]);
      return;
    }
    // 找最长可切（>=2h）的段，对半切
    let bestI = -1;
    let bestLen = 0;
    sorted.forEach((s, i) => {
      const len = s.endHour - s.startHour;
      if (len >= 2 && len > bestLen) {
        bestLen = len;
        bestI = i;
      }
    });
    if (bestI < 0) return; // 所有段都是 1h，没法切
    const seg = sorted[bestI];
    const mid = seg.startHour + Math.floor(bestLen / 2);
    const next = sorted.slice();
    next[bestI] = { ...seg, endHour: mid };
    next.splice(bestI + 1, 0, {
      label: "新段",
      startHour: mid,
      endHour: seg.endHour,
    });
    onChange(next);
  };

  const commitEdit = () => {
    if (!editing) return;
    const trimmed = editing.value.trim();
    if (trimmed && trimmed !== sorted[editing.index].label) {
      const next = sorted.slice();
      next[editing.index] = { ...next[editing.index], label: trimmed };
      onChange(next);
    }
    setEditing(null);
  };

  return (
    <div className={styles.wrap}>
      <div className={styles.barWrap}>
        <div className={styles.bar} ref={barRef}>
          {/* 圆角剪裁层：off 斜纹 + 段块都画在这里。handle 在外面，避免边缘被裁。 */}
          <div className={styles.barClip}>
          <div className={styles.off} />

          {sorted.map((s, i) => {
            const mid = (s.startHour + s.endHour) / 2;
            const hsl = hourColor(mid);
            const light = isLightBg(hsl);
            const isEditing = editing?.index === i;
            return (
              <div
                key={i}
                className={styles.segment}
                onMouseEnter={() => enterSegment(i)}
                onMouseLeave={leaveSegment}
                style={
                  {
                    left: pct(s.startHour),
                    width: pct(s.endHour - s.startHour),
                    background: hslStr(hsl),
                    color: light ? "#3a3f55" : "#fff",
                    "--time-opacity": light ? "0.72" : "0.9",
                    "--text-shadow": light
                      ? "none"
                      : "0 1px 2px rgba(0, 0, 0, 0.18)",
                  } as React.CSSProperties
                }
              >
                {isEditing ? (
                  <input
                    className={styles.editInput}
                    autoFocus
                    value={editing!.value}
                    onChange={(e) =>
                      setEditing({ index: i, value: e.target.value })
                    }
                    onBlur={commitEdit}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitEdit();
                      else if (e.key === "Escape") setEditing(null);
                    }}
                    maxLength={12}
                  />
                ) : (
                  <button
                    type="button"
                    className={styles.segmentBody}
                    onClick={() => setEditing({ index: i, value: s.label })}
                    title="点一下改名"
                  >
                    <span className={styles.segmentLabel}>{s.label}</span>
                  </button>
                )}
              </div>
            );
          })}

          {sorted.length === 0 && (
            <div className={styles.empty}>还没有段，点下面"添加段"开始</div>
          )}
          </div>

          {sorted.length > 0 && (
            <div className={styles.handles}>
              <Handle
                position={pct(sorted[0].startHour)}
                onPointerDown={(e) =>
                  startDrag(e, { kind: "left-edge", index: -1 })
                }
                ariaLabel="活动起始时刻"
              />
              {sorted.slice(0, -1).map((s, i) => (
                <Handle
                  key={`h-${i}`}
                  position={pct(s.endHour)}
                  onPointerDown={(e) =>
                    startDrag(e, { kind: "between", index: i })
                  }
                  ariaLabel="段间分隔"
                />
              ))}
              <Handle
                position={pct(sorted[sorted.length - 1].endHour)}
                onPointerDown={(e) =>
                  startDrag(e, { kind: "right-edge", index: -1 })
                }
                ariaLabel="活动结束时刻"
              />
            </div>
          )}

          {/* 删除按钮浮层：hover 段时弹在条正上方，红色半透明 */}
          {sorted.length > 0 && (
            <div className={styles.popupLayer}>
              {sorted.map((s, i) => {
                const cx = (s.startHour + s.endHour) / 2;
                const visible = hoverIndex === i;
                return (
                  <button
                    key={i}
                    type="button"
                    className={`${styles.removePopup} ${visible ? styles.removePopupOn : ""}`}
                    style={{ left: pct(cx) }}
                    onMouseEnter={() => enterSegment(i)}
                    onMouseLeave={leaveSegment}
                    onClick={(e) => {
                      e.stopPropagation();
                      removeAt(i);
                    }}
                    aria-label="删除段"
                    title={
                      i + 1 < sorted.length
                        ? "删除（范围并入右邻居）"
                        : "删除（范围并入左邻居）"
                    }
                    tabIndex={visible ? 0 : -1}
                  >
                    <X size={13} strokeWidth={2.4} />
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className={styles.ticks}>
          {TICKS.map((h) => (
            <div key={h} className={styles.tick} style={{ left: pct(h) }}>
              <div className={styles.tickMark} />
              <div className={styles.tickLabel}>{h}</div>
            </div>
          ))}
        </div>

        {/* 时间气泡浮层：hover 段时显示在条正下方（覆盖刻度），不抢条本身的视觉 */}
        {sorted.length > 0 && (
          <div className={styles.timeLayer} aria-hidden>
            {sorted.map((s, i) => {
              const cx = (s.startHour + s.endHour) / 2;
              const visible = hoverIndex === i && editing?.index !== i;
              return (
                <div
                  key={i}
                  className={`${styles.timePill} ${visible ? styles.timePillOn : ""}`}
                  style={{ left: pct(cx) }}
                >
                  {fmt(s.startHour)}–{fmt(s.endHour)}
                </div>
              );
            })}
          </div>
        )}
      </div>

      <button type="button" className={styles.addBtn} onClick={addSegment}>
        <Plus size={14} strokeWidth={2} />
        添加段
      </button>
    </div>
  );
}

interface HandleProps {
  position: string;
  onPointerDown: (e: React.PointerEvent) => void;
  ariaLabel: string;
}

function Handle({ position, onPointerDown, ariaLabel }: HandleProps) {
  return (
    <div
      className={styles.handle}
      style={{ left: position }}
      onPointerDown={onPointerDown}
      role="separator"
      aria-label={ariaLabel}
      aria-orientation="vertical"
    >
      <div className={styles.handleGrip}>
        <span className={styles.dot} />
        <span className={styles.dot} />
        <span className={styles.dot} />
      </div>
    </div>
  );
}
