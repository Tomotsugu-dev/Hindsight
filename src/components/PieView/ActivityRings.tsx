import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronUp, Ellipsis, type LucideIcon } from "lucide-react";
import { AppIcon } from "../AppIcon/AppIcon";
import type { BreakdownSlice } from "../../hooks/useSuperCategoryBreakdown";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { useCategories } from "../../state/categories";
import { useIsDark } from "../../hooks/useTheme";
import { adjustCategoryColor } from "../../utils/categoryColor";
import { useDurationFormatter } from "../../utils/duration";
import { useWheelSteps } from "../../hooks/useWheelSteps";
import {
  fitSegments,
  offsetToShow,
  OTHERS_ID,
  ringWindow,
  type RingFocusApp,
  type RingItem,
} from "./ringItems";
import styles from "./ActivityRings.module.css";

interface Props {
  /** SVG 边长 px */
  size: number;
  slices: BreakdownSlice[];
  total: number;
  /** 同心环最多几圈；小类更多时最后一圈是「其他」 */
  rings: number;
  /** 点击钉住的大类：里面换成一个完整的圆环，按小类占这个大类的比例分段 */
  pinnedSuperId: string | null;
  /** 鼠标停在右边列表某个大类上：只强调这个大类下的圈，不换结构 */
  highlightSuperId: string | null;
  /** 点外框的某个大类、或钉住时点圆心：交给父侧钉住 / 取消钉住 */
  onSuperClick?: (superId: string) => void;
  /** 鼠标停在圆环下面那排小类的某一个上：只强调它，不换结构 */
  highlightCatId: string | null;
  /** 选中的小类：亮它那一圈或那一段；不在露出的几圈里时滚到能看见它 */
  selectedCatId: string | null;
  /** 点同心环的一圈、或单环的一段：交给父侧选中 / 取消选中这个小类 */
  onCatClick?: (catId: string) => void;
  /** 已经露出第一名时还往上滚：交给父侧回到只有大类的那一层 */
  onZoomOut?: () => void;
  /** 鼠标停在下方应用列表的某个应用上：画成它所属大类的单环，突出这个应用 */
  focusApp?: RingFocusApp | null;
  interactive: boolean;
  /** 只给当前 slide 挂，prev/next 挂了会同名冲突 */
  viewTransitionName?: string;
}

/** 「其他 N 个」那一圈的颜色：暖中性灰，跟「未归入大类」同色 */
const OTHERS_COLOR = "#a8a29e";
/** 外框细线的粗细：平时、所属大类亮起时 */
const FRAME = 4;
const FRAME_BOLD = 6;
/** 外框各段之间的缝（px） */
const FRAME_GAP = 4;
/** 外框或单环的一段要在开头放图标时，图标圆片后面至少还要露出的线段长度（px） */
const ICON_MARGIN = 6;
/** 同心环每圈的粗细和圈与圈的间隔（px）；7 圈时圆心还要留得下两行字 */
const RING = 9;
const RING_GAP = 3.5;
/** 起点图标圆片的半径（px） */
const CHIP = RING / 2 + 3;
/** 停在应用上时单环和最里一圈缩到原来的这么大，外面腾出位置放引线和气泡 */
const SHRINK = 0.8;
/** 应用图标气泡的直径（px） */
const BUBBLE = 26;
/** 钉住大类时那一个完整圆环段与段之间的缝（px）；粗细跟同心环一样是 RING */
const SINGLE_GAP = 4;
/** 一段除了两头圆头和缝之外，至少还要露出这么长（px），否则看不出是一段 */
const MIN_BODY = 2;

const TOP = -Math.PI / 2;

function polar(c: number, r: number, angle: number) {
  return { x: c + r * Math.cos(angle), y: c + r * Math.sin(angle) };
}

/** 半径 r 上从 a0 到 a1（SVG 数学角）的一段圆弧，给描边用 */
function arcPath(c: number, r: number, a0: number, a1: number): string {
  // 整圈的起点终点重合，弧画不出来，留一点点不闭合
  const end = Math.min(a1, a0 + 2 * Math.PI - 0.001);
  const p0 = polar(c, r, a0);
  const p1 = polar(c, r, end);
  const large = end - a0 > Math.PI ? 1 : 0;
  return `M${p0.x.toFixed(3)} ${p0.y.toFixed(3)}A${r} ${r} 0 ${large} 1 ${p1.x.toFixed(3)} ${p1.y.toFixed(3)}`;
}

/**
 * 把一圈按比例切成首尾相接的几段，每段留缝；圆头会从端点伸出半个线宽，按线宽把两头往里收。
 * 太短收不下圆头的段缩成一个点。返回 SVG 角度。
 */
function segmentAngles(
  spans: number[],
  r: number,
  width: number,
  gap: number,
): [number, number][] {
  const cap = width / 2 / r;
  const pad = gap / r;
  let acc = TOP;
  return spans.map((span) => {
    const start = acc;
    acc += span;
    const a0 = start + pad / 2 + cap;
    const a1 = start + span - pad / 2 - cap;
    const mid = start + span / 2;
    return a1 > a0 ? [a0, a1] : [mid, mid + 0.001];
  });
}

/** 同心环里要画的一圈：排第几圈、颜色、分钟数、起点放什么图标 */
interface DrawRing {
  key: string;
  index: number;
  color: string;
  minutes: number;
  icon: LucideIcon;
  /** 所属大类，用来跟外框对应；「其他」那一圈为 null */
  superId: string | null;
  hoverId: string;
}

/**
 * 占比视图的圆环（原型）。最外一圈分段细线是大类的比例，里面有两种样子：
 * - 平时：几圈同心进度环，是时长最多的几个小类，每圈从 12 点顺时针长，长度 = 占全部时间的比例，
 *   起点放小类图标。鼠标停在大类上只强调它下面的圈，不换圈。
 * - 点击钉住某个大类，或鼠标停在下方应用列表的某个应用上：只剩一个完整的圆环，一整圈就是
 *   这个大类（应用所属的大类），按小类占它的比例分段。停在应用上时整个圆环缩小、淡化，
 *   应用在它的小类那段里占的部分画成全色，引线指到外面的应用图标；圆心是应用占全部时间的比例。
 * 点同心环的一圈、或单环的一段，是选中这个小类，跟点圆环下面那排小类一样。
 */
export function ActivityRings({
  size,
  slices,
  total,
  rings,
  pinnedSuperId,
  highlightSuperId,
  onSuperClick,
  highlightCatId,
  selectedCatId,
  onCatClick,
  onZoomOut,
  focusApp = null,
  interactive,
  viewTransitionName,
}: Props) {
  const { t } = useTranslation();
  const fmtHM = useDurationFormatter();
  const isDark = useIsDark();
  const { getCategory } = useCategories();
  // 鼠标停着的：外框上的大类、同心环的某一圈、钉住时圆环的某一段；只用来强调
  const [frameHover, setFrameHover] = useState<string | null>(null);
  const [ringHover, setRingHover] = useState<string | null>(null);
  const [segHover, setSegHover] = useState<string | null>(null);
  // 同心环露出的那扇窗从第几名开始；滚轮或下面的箭头改它
  const [ringOffset, setRingOffset] = useState(0);
  // 上次滚到能看见的那个选中小类；选中的变了才滚一次，之后照常能滑走
  const [shownCatId, setShownCatId] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

  const c = size / 2;
  const frameR = size / 2 - FRAME_BOLD / 2;
  const firstRingR = frameR - FRAME_BOLD / 2 - 8 - RING / 2;
  const ringR = (i: number) => firstRingR - i * (RING + RING_GAP);
  const catIcon = (catId: string) => resolveCategoryIcon(getCategory(catId)?.icon);
  const pct = (minutes: number, of: number) => `${Math.round((minutes / of) * 100)}%`;

  const focus = interactive ? focusApp : null;
  const focusSlice = focus
    ? (slices.find((s) => s.cats.some((cat) => cat.id === focus.categoryId)) ?? null)
    : null;
  const pinnedSlice =
    interactive && pinnedSuperId ? (slices.find((s) => s.id === pinnedSuperId) ?? null) : null;
  // 画成单环的大类：停在应用上时是应用所属的大类，否则是钉住的大类
  const pinned = focusSlice ?? pinnedSlice;

  // —— 平时：全部大类的小类按时长排好，同心环只露出 rings 圈，可以滑动换一批 ——
  const allCats: RingItem[] = slices.flatMap((s) =>
    s.cats.map((cat) => ({
      id: cat.id,
      name: cat.name,
      color: cat.color,
      minutes: cat.minutes,
      superId: s.id,
    })),
  );
  if (shownCatId !== selectedCatId) {
    setShownCatId(selectedCatId);
    if (selectedCatId) setRingOffset(offsetToShow(allCats, rings, ringOffset, selectedCatId));
  }
  const ringWin = ringWindow(pinned ? [] : allCats, rings, ringOffset);
  const ringItems = ringWin.visible;
  const maxOffset = Math.max(0, ringWin.total - rings);
  const scrollable = interactive && !pinned && maxOffset > 0;
  // 在圆环上滚动：往下换下一批小类；往上换上一批，已经露出第一名（或者是单环）时回到只有大类的那一层
  useWheelSteps(
    wrapRef,
    interactive,
    (down) => (down ? scrollable && ringWin.offset < maxOffset : ringWin.offset > 0 || !!onZoomOut),
    (steps) => {
      if (steps < 0 && ringWin.offset === 0) {
        onZoomOut?.();
        return true;
      }
      setRingOffset(Math.min(maxOffset, Math.max(0, ringWin.offset + steps)));
    },
  );
  const catOf = (id: string | null) => (id ? (allCats.find((i) => i.id === id) ?? null) : null);
  const superHover = frameHover ?? highlightSuperId;
  // 圆环下面那排小类里停着的，没有就是选中的
  const tagCatId = highlightCatId ?? selectedCatId;
  // 平时亮哪个小类：停在某一圈上的；停在外框或右边列表的大类上时不亮小类；否则是 tagCatId
  const litCat = ringHover ? catOf(ringHover) : superHover ? null : catOf(tagCatId);
  // 亮的小类露在窗口里就只亮那一圈，没露出来就亮它的大类
  const litRingId = litCat && ringItems.some((r) => r.id === litCat.id) ? litCat.id : null;
  const emphasisSuperId = litCat ? litCat.superId : superHover;

  const draw: DrawRing[] = ringItems.map((item, i) => ({
    key: item.id,
    index: i,
    color: item.color,
    minutes: item.minutes,
    icon: catIcon(item.id),
    superId: item.superId,
    hoverId: item.id,
  }));

  // —— 单环：一个完整的圆环，粗细和位置跟同心环第一圈一样 ——
  // 一段至少要放得下两头的圆头（合起来一个线宽）、段间的缝和一点本体，不够的并进「其他」
  const singleR = firstRingR;
  const minFraction = (RING + SINGLE_GAP + MIN_BODY) / singleR / (2 * Math.PI);
  const single = pinned
    ? fitSegments(
        pinned.cats.map((cat) => ({ ...cat, merged: 0 })),
        minFraction,
        OTHERS_COLOR,
      )
    : { segments: [], fractions: [] };
  const singleSegs = segmentAngles(
    single.fractions.map((f) => f * 2 * Math.PI),
    singleR,
    RING,
    SINGLE_GAP,
  );
  const hoveredSeg = single.segments.find((s) => s.id === segHover) ?? null;
  const singleIconMinSpan = (CHIP * 2 + ICON_MARGIN) / singleR;
  // 单环上亮哪一段：鼠标停着的那段；停在应用上时是它的小类（小类太小并进了「其他」就亮「其他」）
  const focusSegId = focus
    ? single.segments.some((s) => s.id === focus.categoryId)
      ? focus.categoryId
      : OTHERS_ID
    : null;
  // tagCatId 在单环上是哪一段（太小并进了「其他」就是「其他」）
  const tagCat = pinned && tagCatId ? (pinned.cats.find((cat) => cat.id === tagCatId) ?? null) : null;
  const tagSegId = tagCat
    ? single.segments.some((s) => s.id === tagCat.id)
      ? tagCat.id
      : OTHERS_ID
    : null;
  const litSegId = segHover ?? focusSegId ?? tagSegId;
  // 单环一段的明暗：有亮着的段时其余变淡；停在应用上时连它的小类那段也半透明（应用那部分另画全色）
  const segClass = (segId: string) =>
    litSegId !== null && litSegId !== segId
      ? styles.dim
      : focus && segId === litSegId
        ? styles.faded
        : "";

  // —— 停在应用上：应用是单环上它的小类那一段里的一部分，从这段开头起，按应用占小类的比例画成全色 ——
  const focusSegIndex = focusSegId ? single.segments.findIndex((s) => s.id === focusSegId) : -1;
  const appArc =
    focus && focusSegIndex >= 0
      ? (() => {
          const [a0, a1] = singleSegs[focusSegIndex];
          const seg = single.segments[focusSegIndex];
          const frac = Math.min(1, focus.categoryMinutes / Math.max(seg.minutes, 1));
          return { a0, end: a0 + (a1 - a0) * frac };
        })()
      : null;
  // 停在应用上时整个圆环（外圈、内圈）一起缩到 SHRINK。
  // 引线和气泡都在缩小后的坐标里算：气泡放在缩小后的外圈外面，引线从气泡往里，
  // 穿过外圈（这个大类）、内圈（它的小类），指到应用那部分的外沿
  const leader = appArc
    ? (() => {
        const angle = (appArc.a0 + Math.max(appArc.end, appArc.a0)) / 2;
        const bubbleR = (frameR + FRAME_BOLD / 2) * SHRINK + 2 + BUBBLE / 2;
        return {
          from: polar(c, (singleR + RING / 2) * SHRINK + 1, angle),
          to: polar(c, bubbleR - BUBBLE / 2 - 1, angle),
          bubble: polar(c, bubbleR, angle),
        };
      })()
    : null;

  const holeR = pinned
    ? (singleR - RING / 2) * (focus && appArc ? SHRINK : 1)
    : ringR(Math.max(...draw.map((d) => d.index), 0)) - RING / 2;

  // 外框亮哪个大类：单环的大类，或平时强调的大类
  const litSuperId = pinned ? pinned.id : emphasisSuperId;

  // —— 外框：大类按占比分段，从 12 点顺时针 ——
  const frameSegs = segmentAngles(
    slices.map((s) => (s.minutes / total) * 2 * Math.PI),
    frameR,
    FRAME_BOLD,
    FRAME_GAP,
  );
  // 外框每段的开头放大类图标，跟同心环一样像进度条的起点；圆片的前沿对齐线段圆头的前沿。
  // 弧长放不下图标圆片再加一点线段的段不放，免得叠在一起
  const frameIconMinSpan = (CHIP * 2 + ICON_MARGIN) / frameR;
  // 钻进某个大类时（钉住，或停在应用上），外圈只画这个大类，一整圈 = 它，跟内圈同一个标准；
  // 这样引线从哪个方向穿进来，经过的都是这个大类。停在应用上时淡化
  const zoomFrame = pinned
    ? (() => {
        const color = adjustCategoryColor(pinned.color, isDark);
        const cap = FRAME_BOLD / 2 / frameR;
        const a0 = TOP + cap;
        const a1 = TOP + 2 * Math.PI - cap - FRAME_GAP / frameR;
        const Icon = resolveCategoryIcon(pinned.icon);
        const at = polar(c, frameR, a0 + (CHIP - FRAME_BOLD / 2) / frameR);
        return (
          <g
            key={`zoom:${pinned.id}`}
            className={`${styles.frameGroup} ${focus ? styles.faded : ""} ${interactive ? styles.interactive : ""}`}
            onClick={() => interactive && onSuperClick?.(pinned.id)}
          >
            <path
              className={styles.frame}
              d={arcPath(c, frameR, a0, a1)}
              stroke={color}
              style={{ strokeWidth: FRAME_BOLD }}
            />
            <g className={styles.chip}>
              <circle cx={at.x} cy={at.y} r={CHIP} fill={color} />
              <Icon
                x={at.x - 5.5}
                y={at.y - 5.5}
                size={11}
                color="#fff"
                strokeWidth={2.6}
                aria-hidden
              />
            </g>
          </g>
        );
      })()
    : null;
  const frame = zoomFrame ?? slices.map((s, i) => {
    const lit = litSuperId === null || litSuperId === s.id;
    const bold = litSuperId === s.id;
    const color = adjustCategoryColor(s.color, isDark);
    const span = (s.minutes / total) * 2 * Math.PI;
    const Icon = span >= frameIconMinSpan ? resolveCategoryIcon(s.icon) : null;
    const at = polar(c, frameR, frameSegs[i][0] + (CHIP - FRAME_BOLD / 2) / frameR);
    return (
      <g
        key={s.id}
        className={`${styles.frameGroup} ${lit ? "" : styles.dim} ${interactive ? styles.interactive : ""}`}
        onMouseEnter={() => interactive && setFrameHover(s.id)}
        onMouseLeave={() => interactive && setFrameHover(null)}
        onClick={() => interactive && onSuperClick?.(s.id)}
      >
        <path
          className={styles.frame}
          d={arcPath(c, frameR, frameSegs[i][0], frameSegs[i][1])}
          stroke={color}
          style={{ strokeWidth: bold ? FRAME_BOLD : FRAME }}
        />
        {Icon && (
          <g className={styles.chip}>
            <circle cx={at.x} cy={at.y} r={CHIP} fill={color} />
            <Icon
              x={at.x - 5.5}
              y={at.y - 5.5}
              size={11}
              color="#fff"
              strokeWidth={2.6}
              aria-hidden
            />
          </g>
        )}
      </g>
    );
  });

  // —— 圆心 ——
  const maxChars = holeR < 45 ? 5 : holeR < 70 ? 7 : 10;
  const short = (s: string) => (s.length > maxChars ? `${s.slice(0, maxChars - 1)}…` : s);
  const nameOf = (item: { id: string; name: string; merged: number }) =>
    item.id === OTHERS_ID ? t("today.pie.others", { count: item.merged }) : item.name;
  let center: { big: string; small: string[] } | null = null;
  if (focus) {
    // 停在应用上：这个应用占全部时间的比例、名字、时长
    center = { big: pct(focus.minutes, total), small: [short(focus.name), fmtHM(focus.minutes)] };
  } else if (pinned) {
    // 停在某一段上显示真实比例（「其他」画的时候可能被抬到最小比例）；
    // 其次是 tagCatId 那个小类占这个大类的比例（它并进了「其他」也显示它自己）；
    // 都没有时：这个大类占全部时间的比例、名字、时长
    center = hoveredSeg
      ? {
          big: pct(hoveredSeg.minutes, pinned.minutes),
          small: [short(nameOf(hoveredSeg)), fmtHM(hoveredSeg.minutes)],
        }
      : tagCat
        ? {
            big: pct(tagCat.minutes, pinned.minutes),
            small: [short(tagCat.name), fmtHM(tagCat.minutes)],
          }
        : {
            big: pct(pinned.minutes, total),
            small: [short(pinned.name), fmtHM(pinned.minutes)],
          };
  } else if (litCat) {
    center = {
      big: pct(litCat.minutes, total),
      small: [short(litCat.name), fmtHM(litCat.minutes)],
    };
  }
  const smallLines = center?.small ?? [];
  // 有几行小字，大字就往上挪多少，整块在圆心里居中
  const centerTop = c - (smallLines.length - 1) * holeR * 0.16;

  return (
    // 只用来在鼠标离开整个圆环时清掉停留状态、接滚轮，不是可操作的控件
    <div
      ref={wrapRef}
      className={styles.wrap}
      style={{ width: size, height: size }}
      role="presentation"
      onMouseLeave={() => {
        setFrameHover(null);
        setRingHover(null);
        setSegHover(null);
      }}
    >
      <svg
        className={styles.svg}
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        style={viewTransitionName ? { viewTransitionName } : undefined}
      >
        {/* 停在应用上时整个圆环一起缩到 SHRINK，外面腾出位置放气泡 */}
        <g
          className={styles.shrink}
          style={{
            transform: focus && appArc ? `scale(${SHRINK})` : undefined,
            transformOrigin: `${c}px ${c}px`,
          }}
        >
        {frame}

        {pinned ? (
          // 单环：一整圈 = 这个大类，各段 = 小类占它的比例；换大类时换 key，各段重新画出来
          <g key={`single:${pinned.id}`}>
            {/* 分三层画：各段的线 → 应用那部分的全色线 → 各段开头的图标，图标不会被全色线盖住 */}
            {single.segments.map((seg, i) => (
              <path
                key={seg.id}
                className={`${styles.segment} ${segClass(seg.id)} ${seg.id === OTHERS_ID ? "" : styles.interactive}`}
                d={arcPath(c, singleR, singleSegs[i][0], singleSegs[i][1])}
                stroke={adjustCategoryColor(seg.color, isDark)}
                strokeWidth={RING}
                pathLength={1}
                style={{ animationDelay: `${i * 60}ms` }}
                onMouseEnter={() => setSegHover(seg.id)}
                onMouseLeave={() => setSegHover(null)}
                onClick={() => seg.id !== OTHERS_ID && onCatClick?.(seg.id)}
              />
            ))}
            {/* 停在应用上：它在小类那段里占的部分画成全色；换应用时换 key，重新画出来 */}
            {focus && appArc && (
              <path
                key={`app:${focus.groupId}`}
                className={styles.progress}
                d={arcPath(c, singleR, appArc.a0, Math.max(appArc.end, appArc.a0 + 0.001))}
                stroke={adjustCategoryColor(focus.color, isDark)}
                strokeWidth={RING}
                pathLength={1}
              />
            )}
            {single.segments.map((seg, i) => {
              // 段的开头放小类图标，圆片的前沿对齐圆头的前沿；放不下圆片再加一点线段的段不放
              const span = single.fractions[i] * 2 * Math.PI;
              if (span < singleIconMinSpan) return null;
              const Icon = seg.id === OTHERS_ID ? Ellipsis : catIcon(seg.id);
              const at = polar(c, singleR, singleSegs[i][0] + (CHIP - RING / 2) / singleR);
              // 明暗放在外层：弹出动画结束后会一直占着 opacity，跟明暗放在同一个元素上就不生效
              return (
                <g
                  key={`chip:${seg.id}`}
                  className={`${styles.ring} ${segClass(seg.id)} ${seg.id === OTHERS_ID ? "" : styles.interactive}`}
                  onMouseEnter={() => setSegHover(seg.id)}
                  onMouseLeave={() => setSegHover(null)}
                  onClick={() => seg.id !== OTHERS_ID && onCatClick?.(seg.id)}
                >
                  <g className={styles.chip} style={{ animationDelay: `${i * 60}ms` }}>
                    <circle
                      cx={at.x}
                      cy={at.y}
                      r={CHIP}
                      fill={adjustCategoryColor(seg.color, isDark)}
                    />
                    <Icon
                      x={at.x - 5.5}
                      y={at.y - 5.5}
                      size={11}
                      color="#fff"
                      strokeWidth={2.6}
                      aria-hidden
                    />
                  </g>
                </g>
              );
            })}
            {/* 钉住时点圆心取消钉住，回到同心环；停在应用上时圆心不接点击 */}
            {pinnedSlice && !focus && (
              <circle
                className={styles.interactive}
                cx={c}
                cy={c}
                r={holeR}
                fill="transparent"
                onClick={() => onSuperClick?.(pinnedSlice.id)}
              />
            )}
          </g>
        ) : (
          <g key="rings">
            {draw.map((ring) => {
              const r = ringR(ring.index);
              const color = adjustCategoryColor(ring.color, isDark);
              const span = Math.min(ring.minutes / total, 1) * 2 * Math.PI;
              const dim =
                litRingId !== null
                  ? litRingId !== ring.hoverId
                  : emphasisSuperId !== null && ring.superId !== emphasisSuperId;
              const delay = { animationDelay: `${ring.index * 80}ms` };
              const Icon = ring.icon;
              return (
                <g
                  key={ring.key}
                  className={`${styles.ring} ${dim ? styles.dim : ""} ${interactive ? styles.interactive : ""}`}
                  onMouseEnter={() => interactive && setRingHover(ring.hoverId)}
                  onMouseLeave={() => interactive && setRingHover(null)}
                  onClick={() => interactive && onCatClick?.(ring.hoverId)}
                >
                  {/* 底下的淡轨道：整圈，环有多长一眼看得出 */}
                  <circle
                    className={styles.track}
                    cx={c}
                    cy={c}
                    r={r}
                    stroke={`color-mix(in oklab, ${color} 14%, transparent)`}
                    strokeWidth={RING}
                  />
                  <path
                    className={styles.progress}
                    d={arcPath(c, r, TOP, TOP + Math.max(span, 0.001))}
                    stroke={color}
                    strokeWidth={RING}
                    pathLength={1}
                    style={delay}
                  />
                  {/* 起点的图标：圆形色块里放分类图标 */}
                  <g className={styles.chip} style={delay}>
                    <circle cx={c} cy={c - r} r={CHIP} fill={color} />
                    <Icon
                      x={c - 5.5}
                      y={c - r - 5.5}
                      size={11}
                      color="#fff"
                      strokeWidth={2.6}
                      aria-hidden
                    />
                  </g>
                </g>
              );
            })}
          </g>
        )}
        </g>

        {/* 引线：从气泡往里穿过大类那段、小类那段，指到应用那段（缩完才开始画）；换应用时换 key，重新画 */}
        {focus && leader && (
          <path
            key={`leader:${focus.groupId}`}
            className={styles.leader}
            d={`M${leader.from.x.toFixed(2)} ${leader.from.y.toFixed(2)}L${leader.to.x.toFixed(2)} ${leader.to.y.toFixed(2)}`}
            stroke={adjustCategoryColor(focus.color, isDark)}
            pathLength={1}
          />
        )}

        {center ? (
          <>
            <text
              className={styles.centerBig}
              x={c}
              y={centerTop}
              textAnchor="middle"
              style={{ fontSize: Math.min(26, holeR * 0.5) }}
            >
              {center.big}
            </text>
            {smallLines.map((line, i) => (
              <text
                key={i}
                className={styles.centerSmall}
                x={c}
                y={centerTop + holeR * (0.38 + i * 0.3)}
                textAnchor="middle"
                style={{ fontSize: Math.max(10, Math.min(13, holeR * 0.28)) }}
              >
                {line}
              </text>
            ))}
          </>
        ) : (
          <text
            className={styles.centerBig}
            x={c}
            y={c + 5}
            textAnchor="middle"
            style={{ fontSize: Math.max(11, holeR * 0.32) }}
          >
            {fmtHM(total)}
          </text>
        )}
      </svg>

      {/* 小类比圈数多时，右下角显示露出的是第几到第几名，箭头也能翻；滚轮在圆环上滚也行 */}
      {scrollable && (
        <div className={styles.pager}>
          <button
            type="button"
            className={styles.pagerBtn}
            disabled={ringWin.offset <= 0}
            onClick={() => setRingOffset(ringWin.offset - 1)}
            aria-label={t("today.pie.ringsUp")}
            title={t("today.pie.ringsUp")}
          >
            <ChevronUp size={12} strokeWidth={2.2} />
          </button>
          <span className={styles.pagerText}>
            {ringWin.offset + 1}–{ringWin.offset + ringItems.length} / {ringWin.total}
          </span>
          <button
            type="button"
            className={styles.pagerBtn}
            disabled={ringWin.offset >= maxOffset}
            onClick={() => setRingOffset(ringWin.offset + 1)}
            aria-label={t("today.pie.ringsDown")}
            title={t("today.pie.ringsDown")}
          >
            <ChevronDown size={12} strokeWidth={2.2} />
          </button>
        </div>
      )}

      {/* 引线头上的应用图标气泡：真实图标是 HTML 图片，叠在 SVG 上面；引线画到头时弹出来 */}
      {focus && leader && (
        <div
          key={`bubble:${focus.groupId}`}
          className={styles.bubble}
          style={{
            left: leader.bubble.x - BUBBLE / 2,
            top: leader.bubble.y - BUBBLE / 2,
            width: BUBBLE,
            height: BUBBLE,
            borderColor: adjustCategoryColor(focus.color, isDark),
          }}
          aria-hidden
        >
          <AppIcon processName={focus.iconProcess} fallbackColor={focus.color} size={16} />
        </div>
      )}

    </div>
  );
}
