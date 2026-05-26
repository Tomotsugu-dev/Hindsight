import styles from "./Donut.module.css";

export interface DonutSegment {
  id: string;
  color: string;
  value: number;
}

interface Props {
  /** SVG 边长 px，默认 180 */
  size?: number;
  /** 环厚度 px，默认 28 */
  thickness?: number;
  segments: DonutSegment[];
  /** 不传 = sum(segments.value) */
  total?: number;
  /** 当前 hover/active 的 id，亮起；其他 dim */
  activeId?: string | null;
  onHover?: (id: string | null) => void;
  onClick?: (id: string) => void;
  /** 中央上方大字。不传 = 格式化 total 分钟数 */
  centerTitle?: string;
  /** 中央下方小字。不传 = i18n("today.pie.donut.totalLabel") */
  centerSub?: string;
  /** 中央顶上额外百分比小字（drill 视图用） */
  centerPctTop?: string;
  /** 设了就给 SVG 挂 view-transition-name，让 PieView↔PieDrillDetail morph。
   *  prev/next slide 的 Donut 必须不设（否则跟 current 的同名冲突） */
  viewTransitionName?: string;
}

/**
 * SVG 圆环。`stroke-dasharray` + `stroke-dashoffset` 切弧，整组 `rotate(-90)` 让起点
 * 落到 12 点。底色一道极淡 track 确保零值时也能看到环位。
 *
 * 共享元素 `view-transition-name: super-donut` 让 PieView ↔ PieDrillDetail 之间走
 * morph 动画（尺寸 180→156 + 中心字交叉淡入）。
 */
export function Donut({
  size = 180,
  thickness = 28,
  segments,
  total,
  activeId,
  onHover,
  onClick,
  centerTitle,
  centerSub,
  centerPctTop,
  viewTransitionName,
}: Props) {
  const cx = size / 2;
  const cy = size / 2;
  const r = (size - thickness) / 2;
  const circ = 2 * Math.PI * r;
  const sum = total ?? segments.reduce((s, x) => s + x.value, 0);
  const interactive = !!onClick || !!onHover;

  let acc = 0;
  return (
    <svg
      className={styles.donut}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      style={viewTransitionName ? { viewTransitionName } : undefined}
    >
      {/* 淡 track，零值或缺漏不至于空缺 */}
      <circle
        cx={cx}
        cy={cy}
        r={r}
        fill="none"
        stroke="rgba(0,0,0,0.04)"
        strokeWidth={thickness}
      />
      <g transform={`rotate(-90 ${cx} ${cy})`}>
        {segments.map((seg) => {
          if (sum <= 0) return null;
          const frac = seg.value / sum;
          const dash = frac * circ;
          // active 加宽 4px；非 active 在 hover 时 dim
          const isActive = activeId === seg.id;
          const dim = activeId !== null && activeId !== undefined && !isActive;
          const w = isActive ? thickness + 4 : thickness;
          const el = (
            <circle
              key={seg.id}
              className={`${styles.segment} ${dim ? styles.segmentDim : ""} ${interactive ? styles.segmentInteractive : ""}`}
              cx={cx}
              cy={cy}
              r={r}
              fill="none"
              stroke={seg.color}
              strokeWidth={w}
              strokeLinecap="butt"
              strokeDasharray={`${dash} ${circ}`}
              strokeDashoffset={-acc}
              onMouseEnter={() => onHover?.(seg.id)}
              onMouseLeave={() => onHover?.(null)}
              onClick={() => onClick?.(seg.id)}
            />
          );
          acc += dash;
          return el;
        })}
      </g>

      {/* 中央文案：完全可选，不传则空圆心
          —— PieView idle 不传任何，hover 时由父穿入；PieDrillDetail 永远传 pct + 名 */}
      {centerPctTop && (
        <text
          className={styles.centerPct}
          x={cx}
          y={cy - size * 0.16}
          textAnchor="middle"
        >
          {centerPctTop}
        </text>
      )}
      {centerTitle && (
        <text
          className={styles.centerTitle}
          x={cx}
          y={cy + (centerPctTop ? size * 0.02 : size * 0.04)}
          textAnchor="middle"
          style={{ fontSize: size * 0.18 }}
        >
          {centerTitle}
        </text>
      )}
      {centerSub && (
        <text
          className={styles.centerSub}
          x={cx}
          /* sub 跟 title 共存时排在下方；单独存在（idle watermark）时垂直居中 */
          y={
            centerTitle || centerPctTop
              ? cy + size * 0.18
              : cy + size * 0.035
          }
          textAnchor="middle"
          style={{
            fontSize:
              centerTitle || centerPctTop ? size * 0.085 : size * 0.1,
          }}
        >
          {centerSub}
        </text>
      )}
    </svg>
  );
}
