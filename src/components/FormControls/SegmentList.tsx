import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AiSegment } from "../../api/hindsight";
import { resolveSegmentChip } from "../../utils/segmentColor";
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

/** 调色板：一排 pastel 预设，覆盖一天的色温区间。最后一项是"自动"（清空 color）。
 *  labelKey 指向 i18n 中 components.segmentList.swatches 下的子键。 */
const SWATCHES: Array<{ hex: string; labelKey: string }> = [
  { hex: "#a3b6e8", labelKey: "blue" },
  { hex: "#c2bdf3", labelKey: "lavender" },
  { hex: "#cda7e5", labelKey: "lilac" },
  { hex: "#f0b3c6", labelKey: "rose" },
  { hex: "#f5b89e", labelKey: "creamOrange" },
  { hex: "#fbcfb0", labelKey: "peach" },
  { hex: "#f6e2a3", labelKey: "warmYellow" },
  { hex: "#a7d8c5", labelKey: "mint" },
];

function fmt(h: number): string {
  // 段都是整点，省略 ":00"，给时间文字腾大一些
  return String(h).padStart(2, "0");
}

export function SegmentList({ segments, onChange }: Props) {
  const { t } = useTranslation();
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  // 仅触发重渲染用，宽度真值通过 getBoundingClientRect 实时取
  const [, setTick] = useState(0);
  const [editing, setEditing] = useState<{ index: number; value: string } | null>(null);
  // 颜色调色板浮层目标段；editing 时一定 == editing.index，
  // 但点段非 label 区只开 picker、不进 editing 时也会单独被设
  const [pickerIndex, setPickerIndex] = useState<number | null>(null);
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const dragRef = useRef<DragState | null>(null);

  const closeAll = () => {
    setEditing(null);
    setPickerIndex(null);
  };

  const openPicker = (i: number) => {
    setPickerIndex(i);
  };

  const openEditor = (i: number) => {
    setPickerIndex(i);
    setEditing({ index: i, value: sorted[i].label });
  };

  // 点条 + popover 外面关掉编辑 / picker
  useEffect(() => {
    if (editing == null && pickerIndex == null) return;
    const onDown = (e: MouseEvent) => {
      const root = wrapRef.current;
      if (!root) return;
      if (e.target instanceof Node && root.contains(e.target)) return;
      closeAll();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [editing, pickerIndex]);

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
    const newLabel = t("components.segmentList.newSegmentLabel");
    if (sorted.length === 0) {
      onChange([{ label: newLabel, startHour: 9, endHour: 12, color: "" }]);
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
      label: newLabel,
      startHour: mid,
      endHour: seg.endHour,
      color: "",
    });
    onChange(next);
  };

  const setColor = (i: number, color: string) => {
    const next = sorted.slice();
    next[i] = { ...next[i], color };
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
    closeAll();
  };

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <div className={styles.barWrap}>
        <div className={styles.bar} ref={barRef}>
          {/* 圆角剪裁层：off 斜纹 + 段块都画在这里。handle 在外面，避免边缘被裁。 */}
          <div className={styles.barClip}>
          <div className={styles.off} />

          {sorted.map((s, i) => {
            const { background: bg, isLight: light } = resolveSegmentChip(s);
            const isEditing = editing?.index === i;
            return (
              <div
                key={i}
                className={styles.segment}
                role="button"
                tabIndex={0}
                onMouseEnter={() => enterSegment(i)}
                onMouseLeave={leaveSegment}
                onClick={() => {
                  // 自己已经在 editing 不要再处理；其它情况点段块 = 仅开 picker 改色
                  if (editing?.index === i) return;
                  openPicker(i);
                }}
                onKeyDown={(e) => {
                  if (editing?.index === i) return;
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openPicker(i);
                  }
                }}
                style={
                  {
                    left: pct(s.startHour),
                    width: pct(s.endHour - s.startHour),
                    background: bg,
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
                    // 编辑模式刚展开就让光标进 input 是用户预期，键盘 user 也希望立刻能输入
                    // eslint-disable-next-line jsx-a11y/no-autofocus
                    autoFocus
                    value={editing.value}
                    onChange={(e) =>
                      setEditing({ index: i, value: e.target.value })
                    }
                    onBlur={commitEdit}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitEdit();
                      else if (e.key === "Escape") closeAll();
                    }}
                    maxLength={12}
                  />
                ) : (
                  <div className={styles.segmentBody}>
                    <span
                      className={styles.segmentLabel}
                      onClick={(e) => {
                        e.stopPropagation();
                        openEditor(i);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          e.stopPropagation();
                          openEditor(i);
                        }
                      }}
                      role="button"
                      tabIndex={0}
                    >
                      {s.label}
                    </span>
                  </div>
                )}
              </div>
            );
          })}

          {sorted.length === 0 && (
            <div className={styles.empty}>{t("components.segmentList.empty")}</div>
          )}
          </div>

          {sorted.length > 0 && (
            <div className={styles.handles}>
              <Handle
                position={pct(sorted[0].startHour)}
                onPointerDown={(e) =>
                  startDrag(e, { kind: "left-edge", index: -1 })
                }
                ariaLabel={t("components.segmentList.leftEdgeAria")}
              />
              {sorted.slice(0, -1).map((s, i) => (
                <Handle
                  key={`h-${i}`}
                  position={pct(s.endHour)}
                  onPointerDown={(e) =>
                    startDrag(e, { kind: "between", index: i })
                  }
                  ariaLabel={t("components.segmentList.betweenAria")}
                />
              ))}
              <Handle
                position={pct(sorted[sorted.length - 1].endHour)}
                onPointerDown={(e) =>
                  startDrag(e, { kind: "right-edge", index: -1 })
                }
                ariaLabel={t("components.segmentList.rightEdgeAria")}
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
                    aria-label={t("components.segmentList.removeAria")}
                    title={
                      i + 1 < sorted.length
                        ? t("components.segmentList.removeMergeRight")
                        : t("components.segmentList.removeMergeLeft")
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

        {/* 调色板浮层：editing 或 picker 模式时，浮在条下方一排预设色 + "自动" */}
        {pickerIndex != null && sorted[pickerIndex] && (
          // mousedown 阻止默认，防止点 swatch 时 input 先 blur 把 editing 关掉
          // 同时让 popover 不冒泡触发段 onClick / outside-click 关闭
          // 仅作为事件容器，内部 swatch 按钮才是真正的交互目标
          // eslint-disable-next-line jsx-a11y/no-static-element-interactions
          <div
            className={styles.swatchLayer}
            onMouseDown={(e) => e.preventDefault()}
          >
            <div
              className={styles.swatchPopover}
              style={{
                left: pct(
                  (sorted[pickerIndex].startHour +
                    sorted[pickerIndex].endHour) /
                    2,
                ),
              }}
            >
              {SWATCHES.map((sw) => {
                const active =
                  sorted[pickerIndex].color.toLowerCase() ===
                  sw.hex.toLowerCase();
                const swatchName = t(
                  `components.segmentList.swatches.${sw.labelKey}`,
                );
                return (
                  <button
                    key={sw.hex}
                    type="button"
                    className={`${styles.swatch} ${active ? styles.swatchOn : ""}`}
                    style={{ background: sw.hex }}
                    onClick={() => setColor(pickerIndex, sw.hex)}
                    title={swatchName}
                    aria-label={t("components.segmentList.swatchAria", {
                      name: swatchName,
                    })}
                  />
                );
              })}
              <button
                type="button"
                className={`${styles.swatchAuto} ${
                  sorted[pickerIndex].color === "" ? styles.swatchOn : ""
                }`}
                onClick={() => setColor(pickerIndex, "")}
                title={t("components.segmentList.swatchAutoTitle")}
              >
                {t("components.segmentList.swatchAuto")}
              </button>
            </div>
          </div>
        )}
      </div>

      <button type="button" className={styles.addBtn} onClick={addSegment}>
        <Plus size={14} strokeWidth={2} />
        {t("components.segmentList.addBtn")}
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
