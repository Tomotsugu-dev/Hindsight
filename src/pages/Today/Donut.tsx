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
  /** 弧度，叠加到默认 12 点起点上 —— drill 视图用，让单段从父视图同位置开始绘制 */
  startAngleOffset?: number;
}

// ============================================================================
// 几何：每段一个 SVG <path>，端头圆角 + 段间留白。
//
// 算法对齐 d3-shape 的 arc.cornerRadius()：每段 = 外弧 + 4 个圆角 + 内弧。
// 圆角是真正的"药片形"两端，跟 stroke-linecap=round 不一样——cap 是 SVG 描边
// 几何上溢出的圆顶（往两边凸 stroke/2 px，相邻段必然撞）；这里圆角是 path 几何
// 的一部分，半径自动夹紧不超过段宽的一半，相邻段隔 padAngle 弧度后 100% 不重叠。
// ============================================================================

const PAD_ANGLE = 0.03; // 段与段之间的弧度间隔（~1.7°）

function polar(cx: number, cy: number, r: number, angle: number) {
  return { x: cx + r * Math.cos(angle), y: cy + r * Math.sin(angle) };
}

/**
 * 生成一段"药片形"环切片的 SVG path d。
 *
 * 输入角度按 SVG 数学角（x 轴正方向 0，CCW 正）。调用方自己把"12 点起顺时针"
 * 语义转换好（减 π/2）。
 */
function buildArcPath(
  cx: number,
  cy: number,
  innerR: number,
  outerR: number,
  startRad: number,
  endRad: number,
  cornerRadius: number,
): string {
  const da = endRad - startRad;
  if (da <= 0) return "";

  // 圆角夹紧：不能超过环厚一半；也不能超过段在内弧端的"半角余地"
  // 段两端圆角占角度 2 * θInner，必须 < da → θInner < da/2
  // θInner = asin(rc / (innerR + rc))；解 rc < innerR * sin(da/2) / (1 - sin(da/2))
  const ringHalf = (outerR - innerR) / 2;
  const sinHalfDa = Math.sin(Math.min(da / 2, Math.PI / 2 - 0.001));
  const maxByAngle = (innerR * sinHalfDa) / Math.max(1 - sinHalfDa, 0.0001);
  const rc = Math.max(0, Math.min(cornerRadius, ringHalf, maxByAngle));

  if (rc < 0.5) {
    // 圆角太小退化为普通楔形
    const o0 = polar(cx, cy, outerR, startRad);
    const o1 = polar(cx, cy, outerR, endRad);
    const i0 = polar(cx, cy, innerR, startRad);
    const i1 = polar(cx, cy, innerR, endRad);
    const large = da > Math.PI ? 1 : 0;
    return (
      `M${o0.x} ${o0.y}` +
      `A${outerR} ${outerR} 0 ${large} 1 ${o1.x} ${o1.y}` +
      `L${i1.x} ${i1.y}` +
      `A${innerR} ${innerR} 0 ${large} 0 ${i0.x} ${i0.y}` +
      `Z`
    );
  }

  // 圆角中心距：外圆角中心在半径 (outerR - rc)，内圆角中心在半径 (innerR + rc)
  // 沿径向偏移角度：sin(θ) = rc / centerR
  const θOuter = Math.asin(rc / (outerR - rc));
  const θInner = Math.asin(rc / (innerR + rc));

  // 切点：外/内弧上 path 实际开始/结束的位置
  const oStart = polar(cx, cy, outerR, startRad + θOuter);
  const oEnd = polar(cx, cy, outerR, endRad - θOuter);
  const iStart = polar(cx, cy, innerR, startRad + θInner);
  const iEnd = polar(cx, cy, innerR, endRad - θInner);

  // 圆角与径向直边相切的切点（直边上，距原点 = cos(θ) * centerR）
  const dOuter = (outerR - rc) * Math.cos(θOuter);
  const dInner = (innerR + rc) * Math.cos(θInner);
  const rStartOuter = polar(cx, cy, dOuter, startRad);
  const rEndOuter = polar(cx, cy, dOuter, endRad);
  const rStartInner = polar(cx, cy, dInner, startRad);
  const rEndInner = polar(cx, cy, dInner, endRad);

  const largeOuter = endRad - startRad - 2 * θOuter > Math.PI ? 1 : 0;
  const largeInner = endRad - startRad - 2 * θInner > Math.PI ? 1 : 0;

  // 闭合路径走法：
  //   起点 = 外弧起点切点 (oStart)
  //   外弧顺时针到外弧终点切点 (oEnd)
  //   终端外圆角 → 径向边外切点 (rEndOuter)
  //   径向直边 → 内圆角切点 (rEndInner)
  //   终端内圆角 → 内弧终点切点 (iEnd)
  //   内弧逆时针到内弧起点切点 (iStart)
  //   起端内圆角 → 径向边内切点 (rStartInner)
  //   径向直边 → 外圆角切点 (rStartOuter)
  //   起端外圆角 → 外弧起点切点 (oStart) 闭合
  return (
    `M${oStart.x.toFixed(3)} ${oStart.y.toFixed(3)}` +
    `A${outerR} ${outerR} 0 ${largeOuter} 1 ${oEnd.x.toFixed(3)} ${oEnd.y.toFixed(3)}` +
    `A${rc} ${rc} 0 0 1 ${rEndOuter.x.toFixed(3)} ${rEndOuter.y.toFixed(3)}` +
    `L${rEndInner.x.toFixed(3)} ${rEndInner.y.toFixed(3)}` +
    `A${rc} ${rc} 0 0 1 ${iEnd.x.toFixed(3)} ${iEnd.y.toFixed(3)}` +
    `A${innerR} ${innerR} 0 ${largeInner} 0 ${iStart.x.toFixed(3)} ${iStart.y.toFixed(3)}` +
    `A${rc} ${rc} 0 0 1 ${rStartInner.x.toFixed(3)} ${rStartInner.y.toFixed(3)}` +
    `L${rStartOuter.x.toFixed(3)} ${rStartOuter.y.toFixed(3)}` +
    `A${rc} ${rc} 0 0 1 ${oStart.x.toFixed(3)} ${oStart.y.toFixed(3)}` +
    `Z`
  );
}

/**
 * 圆环饼图。每段填充式 `<path>`，端头是真圆角（path 几何，不是 stroke cap）。
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
  startAngleOffset = 0,
}: Props) {
  const cx = size / 2;
  const cy = size / 2;
  const outerR = size / 2;
  const innerR = outerR - thickness;
  const midR = (outerR + innerR) / 2;
  const sum = total ?? segments.reduce((s, x) => s + x.value, 0);
  const interactive = !!onClick || !!onHover;
  // 圆角半径 = 环厚一半（完全药片头），具体段会被自动夹紧
  const CORNER_RADIUS = thickness / 2;

  // 累计角度：从 12 点起顺时针；SVG 数学角 = 顺时针角 - π/2
  // startAngleOffset > 0 时整圈起点顺时针偏移（drill 视图保持父视图相对位置）
  let acc = -Math.PI / 2 + startAngleOffset;

  return (
    <svg
      className={styles.donut}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      style={viewTransitionName ? { viewTransitionName } : undefined}
    >
      {/* 淡 track：环形轨道，小段缺失时不出现纯白空缺 */}
      <circle
        cx={cx}
        cy={cy}
        r={midR}
        fill="none"
        stroke="rgba(0, 0, 0, 0.04)"
        strokeWidth={thickness}
      />

      {segments.map((seg) => {
        if (sum <= 0 || seg.value <= 0) return null;
        const frac = seg.value / sum;
        const span = frac * 2 * Math.PI;
        const segStart = acc;
        const segEnd = acc + span;
        acc = segEnd;

        // PAD_ANGLE 留白：太小段跳过 padding 否则被吃光
        const usePad = span > PAD_ANGLE * 2.2;
        const a0 = usePad ? segStart + PAD_ANGLE / 2 : segStart;
        const a1 = usePad ? segEnd - PAD_ANGLE / 2 : segEnd;

        const isActive = activeId === seg.id;
        const dim = activeId != null && !isActive;
        // active 段单独把外径鼓 3 px，做"切出来"效果；内径不动保证内边平
        const oR = isActive ? outerR + 3 : outerR;

        const d = buildArcPath(cx, cy, innerR, oR, a0, a1, CORNER_RADIUS);
        if (!d) return null;

        return (
          <path
            key={seg.id}
            className={`${styles.segment} ${dim ? styles.segmentDim : ""} ${interactive ? styles.segmentInteractive : ""}`}
            d={d}
            fill={seg.color}
            onMouseEnter={() => onHover?.(seg.id)}
            onMouseLeave={() => onHover?.(null)}
            onClick={() => onClick?.(seg.id)}
          />
        );
      })}

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
          style={{ fontSize: size * 0.13 }}
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
