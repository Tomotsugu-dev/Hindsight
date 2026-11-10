import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useCategories } from "../../state/categories";
import { useSuperCategories } from "../../state/superCategories";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import type { Category, SuperCategory } from "../../api/hindsight";
import { CategoryRow } from "./CategoryRow";
import { SuperRow, OrphanRow, BuiltinRow } from "./superRows";
import { creationScrollTarget } from "./creationFeedback";
import {
  DRAG_AUTO_SCROLL_EDGE,
  findScrollContainer,
  useDragAutoScroll,
} from "../../hooks/useDragAutoScroll";
import {
  insertBlockAtSuperPosition,
  pickPromoteGap,
  planDemoteOrder,
  sameCategoryDrop,
  sameSuperDrop,
  moveBeforeOrAfter,
  pickRowDrop,
  sameOrder,
  type CategoryDropIntent,
  type GapBand,
  type RowBand,
  type SuperDropIntent,
} from "./dragOrder";
import styles from "./SuperCategoriesTable.module.css";
import catStyles from "./Categories.module.css";

/** orphan 行用这个 sentinel 作 super-id（避免跟真 super id 冲突） */
const ORPHAN_KEY = "__ORPHAN__";
/** builtin（隐藏）行用这个 sentinel；不进任何 drop 目标 */
const BUILTIN_KEY = "__BUILTIN__";

/** super 区快照：Y 用内容坐标，body 的左右边界用屏幕坐标（竖向滚动不影响）。 */
interface SuperBand {
  top: number;
  bottom: number;
  bodyLeft: number;
  bodyRight: number;
}

interface CatPreviewGeometry {
  kind: "row" | "promote";
  top: number;
  left: number;
  width: number;
}

interface DragState {
  catId: string;
  displayName: string;
  color: string;
  iconKey: string;
  /** 源行的**左边** X（屏幕坐标）—— fly-chip 的 left 直接用这个值
   *  （`.catFlyChip` 是 position:fixed 但没 translateX(-50%) 居中，所以这里必须传左边
   *  不能传中心 X，否则视觉上整个 fly-chip 往右偏了半个行宽） */
  lockedX: number;
  /** 源行宽度 / 高度（fly-chip 复用同尺寸做"原行复刻"的视觉） */
  width: number;
  height: number;
  /** 源大类 id；orphan 用 ORPHAN_KEY */
  sourceSuperKey: string;
}

/** 大类卡片拖动状态——独立于 cat drag 的 DragState，两条 pipeline 互斥。 */
interface SuperDragState {
  superId: string;
  displayName: string;
  color: string;
  iconKey: string;
  /** 源大类卡片的左边 X */
  lockedX: number;
  /** 源大类卡片宽 / 高（fly-chip 复用） */
  width: number;
  height: number;
}

interface CatLanding {
  preview: DragState;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
}

interface DragError {
  kind: "promote" | "demote";
  rollback: "complete" | "partial";
}

/**
 * 「分类」页主体：大类容器 + CategoryRow + 老 FLIP 风格的拖拽归类。
 *
 * 拖拽机制（PairingSection / 老 DraggableCategoryList 同款）：
 *   1. 在 CategoryRow grip 上 mousedown → 启动 drag，快照源行 rect + 所有大类行 Y 范围
 *   2. mousemove：fly-chip portal X 锁源列 / Y 跟鼠标；
 *      Y-band 命中检测决定 hotSuperKey（属于哪一大类行）
 *   3. mouseup：hotSuperKey ≠ sourceSuperKey → 调 assignCategory；都清掉 drag state
 *
 * builtin（hidden）行的 grip mousedown 被 CategoryRow 内部 return 拦掉，不会触发 drag。
 */
interface SuperCategoriesTableProps {
  /** 刚新建的分类 / 大类 id：滚到它、亮一下、进改名态。null = 无新建 */
  justCreatedCatId?: string | null;
  justCreatedSuperId?: string | null;
}

export function SuperCategoriesTable({
  justCreatedCatId = null,
  justCreatedSuperId = null,
}: SuperCategoriesTableProps = {}) {
  const { t } = useTranslation();
  const { categories, reorder: reorderCategories } = useCategories();
  const {
    supers,
    reorder: reorderSupers,
    moveCategoryToSuper,
    promoteCategory,
    demoteSuper,
  } = useSuperCategories();

  // super-card / orphan-card 的 DOM ref —— 拍 Y 范围快照判断 cursor 落在哪个 super 区
  const superRowRefs = useRef<Map<string, HTMLElement>>(new Map());
  const superRowRefSetters = useRef<Map<string, (el: HTMLElement | null) => void>>(new Map());
  const getSuperRowRef = useCallback((key: string) => {
    const existing = superRowRefSetters.current.get(key);
    if (existing) return existing;
    const setter = (el: HTMLElement | null) => {
      if (el) superRowRefs.current.set(key, el);
      else superRowRefs.current.delete(key);
    };
    superRowRefSetters.current.set(key, setter);
    return setter;
  }, []);

  // 每行 CategoryRow 的 DOM ref —— 同大类内 reorder 时拍 Y 范围做行命中
  const catRowRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const registerCatRowRef = useCallback((id: string, el: HTMLDivElement | null) => {
    if (el) catRowRefs.current.set(id, el);
    else catRowRefs.current.delete(id);
  }, []);

  // 新建后滚到它：新建按钮在页头，新分类落在下方的「未归入」行、新大类落在表格末尾，
  // 不滚过去的话用户点完看不到任何变化。
  // 用 useEffect（不是 layout effect）：子组件的 registerRowRef 也在 effect 里跑，
  // React 先跑子 effect 再跑父 effect，所以这里读 map 一定拿得到刚挂载的行。
  const scrolledForRef = useRef<string | null>(null);
  useEffect(() => {
    const target = creationScrollTarget(
      justCreatedCatId,
      justCreatedSuperId,
      scrolledForRef.current,
    );
    if (!target) return;
    const el =
      target.kind === "cat"
        ? catRowRefs.current.get(target.key)
        : superRowRefs.current.get(target.key);
    if (!el) return;
    scrolledForRef.current = target.key;
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    el.scrollIntoView({
      behavior: reduced ? "auto" : "smooth",
      block: "center",
    });
  }, [justCreatedCatId, justCreatedSuperId]);

  const [drag, setDrag] = useState<DragState | null>(null);
  const [catDrop, setCatDrop] = useState<CategoryDropIntent | null>(null);
  const [catLanding, setCatLanding] = useState<CatLanding | null>(null);
  // 拖拽坐标基准。快照存的是内容坐标（相对滚动内容顶部），命中时用
  // clientY - 容器顶部 + 当前 scrollTop 换算；这样滚动不会让按下时的快照失效。
  const scrollRef = useRef<HTMLElement | null>(null);
  const containerTopRef = useRef(0);
  const containerBottomRef = useRef(0);
  const cursorYRef = useRef(0);
  const cursorXRef = useRef(0);
  const catDragRef = useRef<DragState | null>(null);
  const superDragRef = useRef<SuperDragState | null>(null);
  const catDropRef = useRef<CategoryDropIntent | null>(null);
  const catPreviewRef = useRef<CatPreviewGeometry | null>(null);
  const catOverlayRef = useRef<HTMLDivElement>(null);
  const catFlyRef = useRef<HTMLDivElement>(null);
  const superFlyRef = useRef<HTMLDivElement>(null);
  const superDropRef = useRef<SuperDropIntent | null>(null);
  // super 级 Y 范围快照
  const snapshotYRanges = useRef<Map<string, SuperBand>>(new Map());
  const snapshotGapBands = useRef<GapBand[]>([]);
  // 所有可拖拽行的 Y 范围快照（drag start 时拍一次，按 top 排序）
  const snapshotRowYRanges = useRef<RowBand[]>([]);

  // 大类拖动 reorder 的独立 state（跟上面 cat drag 互斥）
  const [superDrag, setSuperDrag] = useState<SuperDragState | null>(null);
  const [superDrop, setSuperDrop] = useState<SuperDropIntent | null>(null);
  const [landedSuperId, setLandedSuperId] = useState<string | null>(null);
  const [landedCatId, setLandedCatId] = useState<string | null>(null);
  const [mutationPending, setMutationPending] = useState(false);
  const [dragError, setDragError] = useState<DragError | null>(null);
  const landedTimerRef = useRef<number | null>(null);
  const landedCatTimerRef = useRef<number | null>(null);
  const catLandingTimerRef = useRef<number | null>(null);
  /** 大类松手后 fly-box 平滑"吸"到落地位置的目标坐标。
   *  非 null 时 fly-box 走 CSS transition 从 cursor → target，over ~200ms。
   *  动画期间 superDrag / hoverSuperReorderId 都保留——保持源 superBox 处在投机重排位置
   *  （opacity 0.3 占位），fly-box 滑到与它重叠的位置时再统一清掉 + commit reorder。 */
  const [superSnap, setSuperSnap] = useState<{ left: number; top: number } | null>(null);

  // 把 categories 按 super_category_id 分组。
  // 三档：
  //   - 真大类下面（catsBySuper）：用户归入的子分类
  //   - orphan（未归入大类）：super_category_id 为 NULL / dangling 的非 builtin
  //   - builtins：builtin=1 的分类（hidden），独立一行展示，drop 关闭
  const { catsBySuper, orphans, builtins } = useMemo(() => {
    const validSuperIds = new Set(supers.map((s) => s.id));
    const map = new Map<string, Category[]>();
    const orph: Category[] = [];
    const bi: Category[] = [];
    for (const c of categories) {
      if (c.builtin) {
        bi.push(c);
        continue;
      }
      const sid = c.superCategoryId;
      if (!sid || !validSuperIds.has(sid)) {
        orph.push(c);
      } else {
        const arr = map.get(sid) ?? [];
        arr.push(c);
        map.set(sid, arr);
      }
    }
    return { catsBySuper: map, orphans: orph, builtins: bi };
  }, [categories, supers]);

  const dragCatId = drag?.catId ?? null;

  // 大类拖动时的"投机重排"：当 cursor 命中其他 super，supers 数组实时按"会落在哪"
  // 的顺序渲染。手写 FLIP（见 prevSuperTops 那段）据此把整个 superBox（左卡 + 右行）
  // 平滑滑到新位置——右侧小类跟着大类卡一起动，跟小类内拖动同款效果。
  // deps 只用 superDrag.superId（drag 期间稳定）：不能传整个 superDrag 对象，
  // cursorY 每帧变会让 useMemo 每帧返回新数组 → FLIP 死循环。
  const superDragId = superDrag?.superId ?? null;
  const effectiveSupers = useMemo(() => {
    if (!superDragId || superDrop?.kind !== "reorder" || superDrop.targetSuperId === superDragId) {
      return supers;
    }
    const fromIdx = supers.findIndex((s) => s.id === superDragId);
    const toIdx = supers.findIndex((s) => s.id === superDrop.targetSuperId);
    if (fromIdx < 0 || toIdx < 0 || fromIdx === toIdx) return supers;
    const out = [...supers];
    const [moved] = out.splice(fromIdx, 1);
    out.splice(toIdx, 0, moved);
    return out;
  }, [supers, superDragId, superDrop]);

  const validSuperIds = useMemo(() => new Set(supers.map((s) => s.id)), [supers]);
  const superKeyOfCategory = useCallback(
    (c: Category) => {
      if (c.builtin) return BUILTIN_KEY;
      const sid = c.superCategoryId;
      return sid && validSuperIds.has(sid) ? sid : ORPHAN_KEY;
    },
    [validSuperIds],
  );
  const superKeyOf = useCallback(
    (id: string) => {
      const c = categories.find((x) => x.id === id);
      return c ? superKeyOfCategory(c) : ORPHAN_KEY;
    },
    [categories, superKeyOfCategory],
  );

  const measureSuperBands = useCallback(() => {
    const base = containerTopRef.current;
    const scrollTop = scrollRef.current?.scrollTop ?? 0;
    const snap = new Map<string, SuperBand>();
    superRowRefs.current.forEach((el, key) => {
      const r = el.getBoundingClientRect();
      // superBox 的子节点固定是 [superCard, superBody]（见 superRows.tsx）
      const bodyRect = el.children[1]?.getBoundingClientRect();
      snap.set(key, {
        top: r.top - base + scrollTop,
        bottom: r.bottom - base + scrollTop,
        bodyLeft: bodyRect?.left ?? r.left,
        bodyRight: bodyRect?.right ?? r.right,
      });
    });
    snapshotYRanges.current = snap;

    const ordered = [...snap.entries()].sort((a, b) => a[1].top - b[1].top);
    const gaps: GapBand[] = [];
    for (let index = 0; index < ordered.length - 1; index += 1) {
      const [afterSuperId, current] = ordered[index];
      const [, next] = ordered[index + 1];
      if (afterSuperId === ORPHAN_KEY) continue;
      const top = current.bottom;
      const bottom = next.top;
      if (bottom < top) continue;
      gaps.push({
        afterSuperId,
        top,
        bottom,
        lineTop: (top + bottom) / 2,
      });
    }
    snapshotGapBands.current = gaps;
  }, []);

  /** 量所有非 builtin 行的位置（内容坐标），滚动期间不重测。 */
  const measureRowBands = useCallback(() => {
    const base = containerTopRef.current;
    const scrollTop = scrollRef.current?.scrollTop ?? 0;
    const bands: RowBand[] = [];
    for (const c of categories) {
      if (c.builtin) continue;
      const el = catRowRefs.current.get(c.id);
      if (!el) continue;
      const r = el.getBoundingClientRect();
      bands.push({
        id: c.id,
        superKey: superKeyOfCategory(c),
        top: r.top - base + scrollTop,
        bottom: r.bottom - base + scrollTop,
      });
    }
    bands.sort((a, b) => a.top - b.top);
    snapshotRowYRanges.current = bands;
  }, [categories, superKeyOfCategory]);

  const positionCatPreview = useCallback(() => {
    const preview = catPreviewRef.current;
    const element = catOverlayRef.current;
    if (!preview || !element) return;
    const width = `${preview.width}px`;
    if (element.style.width !== width) element.style.width = width;
    element.style.transform = `translate3d(${preview.left}px, ${preview.top}px, 0)`;
  }, []);

  const updateCatHits = useCallback(
    (clientY: number, clientX: number) => {
      const cur = catDragRef.current;
      if (!cur) return;
      const scrollTop = scrollRef.current?.scrollTop ?? 0;
      const containerTop = containerTopRef.current;
      const containerBottom = containerBottomRef.current;
      const y = clientY - containerTop + scrollTop;
      const rows = snapshotRowYRanges.current;

      let next: CategoryDropIntent | null = null;
      let lineTop = 0;
      let left = 0;
      let width = 0;
      let kind: CatPreviewGeometry["kind"] = "row";

      if (rows.length > 0 && clientY <= containerTop + DRAG_AUTO_SCROLL_EDGE) {
        // 拖到窗口上沿：全列表第一行（含「未归入」，跳过 builtin）
        const first = rows[0];
        const band = snapshotYRanges.current.get(first.superKey);
        next = {
          kind: "row",
          superKey: first.superKey,
          rowId: first.id,
          after: false,
        };
        lineTop = first.top - scrollTop + containerTop;
        left = band?.bodyLeft ?? 0;
        width = (band?.bodyRight ?? 0) - (band?.bodyLeft ?? 0);
      } else if (rows.length > 0 && clientY >= containerBottom - DRAG_AUTO_SCROLL_EDGE) {
        // 拖到窗口下沿：全列表最后一行
        const last = rows[rows.length - 1];
        const band = snapshotYRanges.current.get(last.superKey);
        next = {
          kind: "row",
          superKey: last.superKey,
          rowId: last.id,
          after: true,
        };
        lineTop = last.bottom - scrollTop + containerTop;
        left = band?.bodyLeft ?? 0;
        width = (band?.bodyRight ?? 0) - (band?.bodyLeft ?? 0);
      } else {
        const promoteAfter = pickPromoteGap(snapshotGapBands.current, y);
        if (promoteAfter) {
          const gap = snapshotGapBands.current.find((item) => item.afterSuperId === promoteAfter);
          const band = snapshotYRanges.current.get(promoteAfter);
          if (gap && band) {
            next = { kind: "promote", afterSuperId: promoteAfter };
            kind = "promote";
            lineTop = gap.lineTop - scrollTop + containerTop;
            left = band.bodyLeft;
            width = band.bodyRight - band.bodyLeft;
          }
        }

        if (!next) {
          let hitSuper: string | null = null;
          let band: SuperBand | null = null;
          for (const [key, range] of snapshotYRanges.current) {
            if (y >= range.top && y <= range.bottom) {
              hitSuper = key;
              band = range;
              break;
            }
          }
          // 落在两个大类之间的空隙：保留上一次命中，避免松手变成 no-op
          if (!hitSuper || !band) return;

          const superRows = rows.filter((row) => row.superKey === hitSuper);
          let rowId: string | null = null;
          let after = false;
          if (superRows.length > 0) {
            // 落在目标大类左侧卡片上 = 追加到该组末尾。源大类不用这条，
            // 避免在组内拖动时误把行甩到组尾。
            if (hitSuper !== cur.sourceSuperKey && clientX < band.bodyLeft) {
              const last = superRows[superRows.length - 1];
              rowId = last.id;
              after = true;
            } else {
              const drop = pickRowDrop(superRows, y);
              if (drop) {
                rowId = drop.rowId;
                after = drop.after;
              }
            }
          }

          next = { kind: "row", superKey: hitSuper, rowId, after };
          left = band.bodyLeft;
          width = band.bodyRight - band.bodyLeft;
          if (rowId) {
            const rowBand = rows.find((row) => row.id === rowId);
            if (rowBand) {
              lineTop = (after ? rowBand.bottom : rowBand.top) - scrollTop + containerTop;
            }
          } else {
            lineTop = band.top - scrollTop + containerTop;
          }
        }
      }

      const previousPreview = catPreviewRef.current;
      catPreviewRef.current = {
        kind,
        top: lineTop,
        left,
        width,
      };
      if (previousPreview && catOverlayRef.current) {
        const distance = Math.abs(lineTop - previousPreview.top);
        const shiftDuration = Math.min(280, Math.max(160, 120 + distance * 0.35));
        catOverlayRef.current.style.setProperty(
          "--drop-line-shift-ms",
          `${Math.round(shiftDuration)}ms`,
        );
      }
      positionCatPreview();

      if (!sameCategoryDrop(catDropRef.current, next)) {
        catDropRef.current = next;
        setCatDrop(next);
      }
    },
    [positionCatPreview],
  );

  const updateSuperHits = useCallback((clientY: number) => {
    const cur = superDragRef.current;
    if (!cur) return;
    const y = clientY - containerTopRef.current + (scrollRef.current?.scrollTop ?? 0);
    const clientX = cursorXRef.current;

    let hit: string | null = null;
    let band: SuperBand | null = null;
    for (const [key, range] of snapshotYRanges.current) {
      if (key === ORPHAN_KEY) continue;
      if (y >= range.top && y <= range.bottom) {
        hit = key;
        band = range;
        break;
      }
    }

    // 拖到列表最上/最下方时，落到第一个/最后一个大类。
    if (!hit) {
      let firstId: string | null = null;
      let firstTop = Infinity;
      let lastId: string | null = null;
      let lastBottom = -Infinity;
      for (const [key, range] of snapshotYRanges.current) {
        if (key === ORPHAN_KEY) continue;
        if (range.top < firstTop) {
          firstTop = range.top;
          firstId = key;
        }
        if (range.bottom > lastBottom) {
          lastBottom = range.bottom;
          lastId = key;
        }
      }
      if (firstId && y <= firstTop) hit = firstId;
      else if (lastId && y >= lastBottom) hit = lastId;
    }

    // 滚动容器上下边缘强制落到首/尾组；即使首组刚好贴边，也不能把
    // 边缘拖拽误判成“降级到首组内容区”。
    const atTopEdge = clientY <= containerTopRef.current + DRAG_AUTO_SCROLL_EDGE;
    const atBottomEdge = clientY >= containerBottomRef.current - DRAG_AUTO_SCROLL_EDGE;
    if (atTopEdge || atBottomEdge) {
      let firstId: string | null = null;
      let firstTop = Infinity;
      let lastId: string | null = null;
      let lastBottom = -Infinity;
      for (const [key, range] of snapshotYRanges.current) {
        if (key === ORPHAN_KEY) continue;
        if (range.top < firstTop) {
          firstTop = range.top;
          firstId = key;
        }
        if (range.bottom > lastBottom) {
          lastBottom = range.bottom;
          lastId = key;
        }
      }
      hit = atTopEdge ? firstId : lastId;
      band = null;
    }

    if (!hit) {
      // 从内容区移到组间空隙时不能继续沿用 demote，否则会在空隙里误降级。
      // 保留同一大类的排序目标，空隙按“原有排序”处理。
      const previous = superDropRef.current;
      if (previous?.kind === "demote") {
        const next: SuperDropIntent = {
          kind: "reorder",
          targetSuperId: previous.targetSuperId,
        };
        superDropRef.current = next;
        setSuperDrop(next);
      }
      return;
    }
    if (hit === cur.superId) {
      if (superDropRef.current !== null) {
        superDropRef.current = null;
        setSuperDrop(null);
      }
      return;
    }

    const targetBand = band ?? snapshotYRanges.current.get(hit) ?? null;
    const overTargetBody =
      band !== null &&
      hit !== ORPHAN_KEY &&
      targetBand !== null &&
      clientX >= targetBand.bodyLeft &&
      clientX <= targetBand.bodyRight;

    let next: SuperDropIntent | null;
    if (overTargetBody) {
      const targetRows = snapshotRowYRanges.current.filter((row) => row.superKey === hit);
      const drop = pickRowDrop(targetRows, y);
      next = {
        kind: "demote",
        targetSuperId: hit,
        targetRowId: drop?.rowId ?? null,
        after: drop?.after ?? false,
      };
    } else if (hit !== ORPHAN_KEY) {
      next = { kind: "reorder", targetSuperId: hit };
    } else {
      next = null;
    }

    if (!sameSuperDrop(superDropRef.current, next)) {
      superDropRef.current = next;
      setSuperDrop(next);
    }
  }, []);

  useLayoutEffect(() => {
    if (catDrop || catLanding) positionCatPreview();
  }, [catDrop, catLanding, positionCatPreview]);

  useEffect(() => {
    return () => {
      if (landedTimerRef.current !== null) {
        window.clearTimeout(landedTimerRef.current);
      }
      if (catLandingTimerRef.current !== null) {
        window.clearTimeout(catLandingTimerRef.current);
      }
      if (landedCatTimerRef.current !== null) {
        window.clearTimeout(landedCatTimerRef.current);
      }
    };
  }, []);

  const handleGripMouseDown = useCallback(
    (cat: Category, e: ReactMouseEvent<HTMLButtonElement>, rowEl: HTMLDivElement) => {
      if (e.button !== 0 || mutationPending) return; // 只接左键
      e.preventDefault();
      const rect = rowEl.getBoundingClientRect();
      const sourceSuperKey =
        cat.superCategoryId && supers.some((s) => s.id === cat.superCategoryId)
          ? cat.superCategoryId
          : ORPHAN_KEY;

      const scrollEl = findScrollContainer(rowEl);
      scrollRef.current = scrollEl;
      const scrollRect = scrollEl?.getBoundingClientRect();
      containerTopRef.current = scrollRect ? scrollRect.top : 0;
      containerBottomRef.current = scrollRect ? scrollRect.bottom : window.innerHeight;
      cursorYRef.current = e.clientY;
      cursorXRef.current = e.clientX;
      catDropRef.current = null;
      catPreviewRef.current = null;
      setCatDrop(null);
      setCatLanding(null);
      if (catLandingTimerRef.current !== null) {
        window.clearTimeout(catLandingTimerRef.current);
        catLandingTimerRef.current = null;
      }
      if (landedCatTimerRef.current !== null) {
        window.clearTimeout(landedCatTimerRef.current);
        landedCatTimerRef.current = null;
      }
      setLandedCatId(null);
      setDragError(null);
      measureSuperBands();
      measureRowBands();

      const nextDrag: DragState = {
        catId: cat.id,
        displayName: cat.name,
        color: cat.color,
        iconKey: cat.icon,
        lockedX: rect.left,
        width: rect.width,
        height: rect.height,
        sourceSuperKey,
      };
      catDragRef.current = nextDrag;
      setDrag(nextDrag);
    },
    [mutationPending, supers, measureSuperBands, measureRowBands],
  );

  const flashLandedSuper = useCallback((superId: string) => {
    if (landedTimerRef.current !== null) {
      window.clearTimeout(landedTimerRef.current);
    }
    setLandedSuperId(superId);
    landedTimerRef.current = window.setTimeout(() => {
      setLandedSuperId(null);
      landedTimerRef.current = null;
    }, 1700);
  }, []);

  const flashLandedCat = useCallback((catId: string) => {
    if (landedCatTimerRef.current !== null) {
      window.clearTimeout(landedCatTimerRef.current);
    }
    setLandedCatId(catId);
    landedCatTimerRef.current = window.setTimeout(() => {
      setLandedCatId(null);
      landedCatTimerRef.current = null;
    }, 700);
  }, []);

  // 全局 mousemove / mouseup —— drag 期间 attach，结束 detach。
  // 依赖只放 catId；cursorY / hover 都走 ref，避免每次 mousemove 都解绑重绑。
  useEffect(() => {
    if (!dragCatId) return;
    // 拖动期间整个文档 cursor 锁成 grabbing —— fly-chip 是 pointer-events:none，
    // cursor 会穿透到下面元素，不锁 body 就会变成下方元素的 cursor（pointer/text 等）
    const prevCursor = document.body.style.cursor;
    const prevCategoryDrag = document.body.dataset.categoryDrag;
    document.body.style.cursor = "grabbing";
    document.body.dataset.categoryDrag = "true";
    let frame = 0;
    let lastX = cursorXRef.current;
    let lastY = cursorYRef.current;

    const positionFly = () => {
      const cur = catDragRef.current;
      const element = catFlyRef.current;
      if (!cur || !element) return;
      const height = Math.min(cur.height, 96);
      element.style.transform = `translate3d(${cur.lockedX}px, ${lastY - height / 2}px, 0) scale(1.02)`;
    };

    const onMove = (e: MouseEvent) => {
      cursorYRef.current = e.clientY;
      cursorXRef.current = e.clientX;
      lastX = e.clientX;
      lastY = e.clientY;
      if (frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        positionFly();
        updateCatHits(lastY, lastX);
      });
    };
    const onUp = () => {
      // 鼠标最后一次移动可能还在待处理的 rAF 里；松手前重算一次，
      // 不能依赖上一帧的命中结果。
      updateCatHits(cursorYRef.current, cursorXRef.current);
      const cur = catDragRef.current;
      const drop = catDropRef.current;
      const preview = catPreviewRef.current;
      catDragRef.current = null;
      catDropRef.current = null;
      setDrag(null);
      setCatDrop(null);

      if (cur && preview && drop) {
        const height = Math.min(cur.height, 96);
        const landing: CatLanding = {
          preview: cur,
          fromX: cur.lockedX,
          fromY: lastY - height / 2,
          toX: preview.left + (preview.kind === "row" ? 8 : 0),
          toY: preview.top - height / 2,
        };
        setCatLanding(landing);
        if (catLandingTimerRef.current !== null) {
          window.clearTimeout(catLandingTimerRef.current);
        }
        catLandingTimerRef.current = window.setTimeout(() => {
          catLandingTimerRef.current = null;
          setCatLanding(null);
        }, 190);
      }

      if (!cur || !drop) return;
      const allIds = categories.map((category) => category.id);

      if (drop.kind === "promote") {
        setMutationPending(true);
        void promoteCategory(cur.catId, drop.afterSuperId, {
          name: cur.displayName,
          color: cur.color,
          icon: cur.iconKey,
        })
          .then((result) => {
            if (!result.ok) {
              setDragError({
                kind: "promote",
                rollback: result.rollback,
              });
            } else {
              flashLandedCat(cur.catId);
              flashLandedSuper(result.value.id);
            }
          })
          .finally(() => setMutationPending(false));
        return;
      }

      if (!drop.rowId) {
        if (drop.superKey === cur.sourceSuperKey) return;
        const superOrder = [...supers.map((superCategory) => superCategory.id), ORPHAN_KEY];
        const newOrder = insertBlockAtSuperPosition(
          allIds,
          [cur.catId],
          drop.superKey,
          superOrder,
          superKeyOf,
        );
        const superId = drop.superKey === ORPHAN_KEY ? null : drop.superKey;
        setMutationPending(true);
        void moveCategoryToSuper(cur.catId, superId, newOrder)
          .then(() => flashLandedCat(cur.catId))
          .finally(() => setMutationPending(false));
        return;
      }

      if (drop.rowId === cur.catId) return;
      const newOrder = moveBeforeOrAfter(allIds, cur.catId, drop.rowId, drop.after);
      setMutationPending(true);
      if (drop.superKey === cur.sourceSuperKey) {
        const orderChanged = !sameOrder(newOrder, allIds);
        void (
          orderChanged
            ? reorderCategories(newOrder).then(() => flashLandedCat(cur.catId))
            : Promise.resolve()
        ).finally(() => setMutationPending(false));
      } else {
        const superId = drop.superKey === ORPHAN_KEY ? null : drop.superKey;
        void moveCategoryToSuper(cur.catId, superId, newOrder)
          .then(() => flashLandedCat(cur.catId))
          .finally(() => setMutationPending(false));
      }
    };
    positionFly();
    document.addEventListener("mousemove", onMove, { passive: true });
    document.addEventListener("mouseup", onUp);
    return () => {
      if (frame !== 0) cancelAnimationFrame(frame);
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = prevCursor;
      if (prevCategoryDrag === undefined) delete document.body.dataset.categoryDrag;
      else document.body.dataset.categoryDrag = prevCategoryDrag;
    };
  }, [
    dragCatId,
    moveCategoryToSuper,
    reorderCategories,
    categories,
    supers,
    superKeyOf,
    updateCatHits,
    promoteCategory,
    flashLandedCat,
    flashLandedSuper,
  ]);

  useDragAutoScroll({
    active: dragCatId !== null,
    containerRef: scrollRef,
    cursorYRef,
    onScroll: () => updateCatHits(cursorYRef.current, cursorXRef.current),
  });

  const catVisual = drag ?? catLanding?.preview ?? null;
  const FlyIcon = catVisual ? resolveCategoryIcon(catVisual.iconKey) : null;
  const catDragHint =
    drag && catDrop
      ? catDrop.kind === "promote"
        ? t("categories.super.drag.promoteHint", { name: drag.displayName })
        : catDrop.superKey !== drag.sourceSuperKey
          ? t("categories.super.drag.moveHint", { name: drag.displayName })
          : null
      : null;
  const catDropForLine =
    drag && catDrop && !(catDrop.kind === "row" && catDrop.rowId === drag.catId) ? catDrop : null;
  // 用户如果先 hover 让 AppList 展开后才抓 grip，drag.height 会测到展开值（>150px），
  // 那种 fly-chip 视觉太重；clamp 到 96px collapsed 高度兜底
  const flyHeight = catVisual ? Math.min(catVisual.height, 96) : 0;

  useLayoutEffect(() => {
    const element = catFlyRef.current;
    if (!drag || !element) return;
    // 落地过渡结束后元素可能因新拖拽被复用，必须把 landing 期间的
    // transition / opacity / height 恢复成正常拖拽态。
    element.style.transition = "none";
    element.style.opacity = "1";
    element.style.transform = `translate3d(${drag.lockedX}px, ${
      cursorYRef.current - flyHeight / 2
    }px, 0) scale(1.02)`;
  }, [drag, flyHeight]);

  useLayoutEffect(() => {
    if (!catLanding || !catFlyRef.current) return;
    const element = catFlyRef.current;
    const height = Math.min(catLanding.preview.height, 96);
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    element.style.transition = "none";
    element.style.opacity = "1";
    element.style.transform = `translate3d(${catLanding.fromX}px, ${catLanding.fromY}px, 0) scale(1.02)`;
    requestAnimationFrame(() => {
      if (!catFlyRef.current) return;
      element.style.transition = reduced
        ? "opacity 80ms ease-out"
        : "transform 160ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 160ms ease-out";
      element.style.transform = `translate3d(${catLanding.toX}px, ${catLanding.toY}px, 0) scale(0.96)`;
      element.style.opacity = "0";
      element.style.height = `${height}px`;
    });
  }, [catLanding]);

  // —— 大类卡片 mousedown 启动 super drag。除 icon/name/del 按钮以外的区域触发 —— //
  // 注意：传入的是**整个 .superBox section**，不是 .superCard，
  // 这样测出来的 width 才包含右侧 CategoryRow 区域，fly-box 1:1 复刻整组。
  const handleSuperMouseDown = useCallback(
    (sup: SuperCategory, e: ReactMouseEvent<HTMLDivElement>, boxEl: HTMLElement) => {
      if (e.button !== 0 || mutationPending) return;
      // mousedown 落在按钮 / input / role=button 元素上则不当 drag（让它们自己 click / dblclick）
      // role=button 用于排除 superName span（双击改名）等"语义按钮但非 <button> tag"的节点
      const target = e.target as HTMLElement;
      if (target.closest('button, input, [role="button"]')) return;
      e.preventDefault();
      const rect = boxEl.getBoundingClientRect();
      const scrollEl = findScrollContainer(boxEl);
      scrollRef.current = scrollEl;
      containerTopRef.current = scrollEl ? scrollEl.getBoundingClientRect().top : 0;
      containerBottomRef.current = scrollEl
        ? scrollEl.getBoundingClientRect().bottom
        : window.innerHeight;
      cursorYRef.current = e.clientY;
      cursorXRef.current = e.clientX;
      superDropRef.current = null;
      setSuperDrop(null);
      setSuperSnap(null);
      setDragError(null);
      setLandedCatId(null);
      if (landedCatTimerRef.current !== null) {
        window.clearTimeout(landedCatTimerRef.current);
        landedCatTimerRef.current = null;
      }
      measureSuperBands();
      measureRowBands();

      const nextDrag: SuperDragState = {
        superId: sup.id,
        displayName: sup.name,
        color: sup.color,
        iconKey: sup.icon,
        lockedX: rect.left,
        width: rect.width,
        height: rect.height,
      };
      superDragRef.current = nextDrag;
      setSuperDrag(nextDrag);
    },
    [mutationPending, measureSuperBands, measureRowBands],
  );

  // —— super drag 全局 mousemove / mouseup —— //
  useEffect(() => {
    if (!superDragId) return;
    // 整个文档 cursor 锁成 grabbing —— 同 cat drag 那边的理由
    const prevCursor = document.body.style.cursor;
    const prevCategoryDrag = document.body.dataset.categoryDrag;
    document.body.style.cursor = "grabbing";
    document.body.dataset.categoryDrag = "true";
    let frame = 0;
    let lastY = cursorYRef.current;

    const positionFly = () => {
      const cur = superDragRef.current;
      const element = superFlyRef.current;
      if (!cur || !element) return;
      element.style.transform = `translate3d(${cur.lockedX}px, ${lastY - cur.height / 2}px, 0)`;
    };

    const onMove = (e: MouseEvent) => {
      cursorYRef.current = e.clientY;
      cursorXRef.current = e.clientX;
      lastY = e.clientY;
      if (!superDragRef.current || frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        positionFly();
        updateSuperHits(lastY);
      });
    };
    const onUp = () => {
      // 同 cat drag：先把最后一个未处理的鼠标位置落成命中结果。
      updateSuperHits(cursorYRef.current);
      const cur = superDragRef.current;
      const drop = superDropRef.current;
      superDragRef.current = null;
      superDropRef.current = null;
      // 没目标 / 落回自己 → 直接清掉，不放 snap 动画（fly-box 应"原地消失"）
      if (!cur || !drop || (drop.kind === "reorder" && drop.targetSuperId === cur.superId)) {
        setSuperDrag(null);
        setSuperDrop(null);
        setSuperSnap(null);
        return;
      }

      if (drop.kind === "demote") {
        const targetBand = snapshotYRanges.current.get(drop.targetSuperId);
        const targetRow = drop.targetRowId
          ? snapshotRowYRanges.current.find((row) => row.id === drop.targetRowId)
          : null;
        if (!targetBand) {
          setSuperDrag(null);
          setSuperDrop(null);
          setSuperSnap(null);
          return;
        }
        const scrollTop = scrollRef.current?.scrollTop ?? 0;
        const targetY = targetRow
          ? (drop.after ? targetRow.bottom : targetRow.top) - scrollTop + containerTopRef.current
          : targetBand.top - scrollTop + containerTopRef.current;
        setMutationPending(true);
        setSuperSnap({
          left: targetBand.bodyLeft,
          top: targetY - cur.height / 2,
        });
        window.setTimeout(() => {
          const allIds = categories.map((category) => category.id);
          const movingIds = categories
            .filter((category) => !category.builtin && category.superCategoryId === cur.superId)
            .map((category) => category.id);
          const superOrder = [...supers.map((superCategory) => superCategory.id), ORPHAN_KEY];
          const orderedIds = planDemoteOrder(
            allIds,
            movingIds,
            drop.targetSuperId,
            drop.targetRowId,
            drop.after,
            superOrder,
            superKeyOf,
          );
          void demoteSuper(cur.superId, drop.targetSuperId, orderedIds)
            .then((result) => {
              if (!result.ok) {
                setDragError({ kind: "demote", rollback: result.rollback });
              } else {
                flashLandedSuper(drop.targetSuperId);
              }
            })
            .finally(() => {
              setSuperDrag(null);
              setSuperSnap(null);
              setSuperDrop(null);
              setMutationPending(false);
            });
        }, 190);
        return;
      }

      // 落到别的大类：测量"源 superBox"当前 DOM 位置——因 effectiveSupers 已投机重排，
      // 它的当前 rect == 最终落地位置。set superSnap，让 fly-box CSS transition 滑过去。
      setMutationPending(true);
      const sourceEl = superRowRefs.current.get(cur.superId);
      const ids = supers.map((superCategory) => superCategory.id);
      const fromIdx = ids.indexOf(cur.superId);
      const toIdx = ids.indexOf(drop.targetSuperId);
      const commit = () => {
        if (fromIdx >= 0 && toIdx >= 0 && fromIdx !== toIdx) {
          const reordered = [...ids];
          reordered.splice(fromIdx, 1);
          reordered.splice(toIdx, 0, cur.superId);
          void reorderSupers(reordered)
            .then(() => flashLandedSuper(cur.superId))
            .finally(() => setMutationPending(false));
        } else {
          setMutationPending(false);
        }
      };
      if (!sourceEl) {
        setSuperDrag(null);
        setSuperDrop(null);
        setSuperSnap(null);
        commit();
        return;
      }
      const rect = sourceEl.getBoundingClientRect();
      setSuperSnap({ left: rect.left, top: rect.top });
      // CSS transition 跑完后再清 state + commit reorder。期间 superDrag 仍 truthy，
      // 投机重排 + opacity 0.3 占位都保留，视觉上 fly-box 平滑落地后整组无缝接管。
      window.setTimeout(() => {
        setSuperDrag(null);
        setSuperDrop(null);
        setSuperSnap(null);
        commit();
      }, 420);
    };
    positionFly();
    document.addEventListener("mousemove", onMove, { passive: true });
    document.addEventListener("mouseup", onUp);
    return () => {
      if (frame !== 0) cancelAnimationFrame(frame);
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = prevCursor;
      if (prevCategoryDrag === undefined) delete document.body.dataset.categoryDrag;
      else document.body.dataset.categoryDrag = prevCategoryDrag;
    };
  }, [
    superDragId,
    supers,
    reorderSupers,
    updateSuperHits,
    categories,
    superKeyOf,
    demoteSuper,
    flashLandedSuper,
  ]);

  useDragAutoScroll({
    active: superDragId !== null && superSnap === null,
    containerRef: scrollRef,
    cursorYRef,
    onScroll: () => updateSuperHits(cursorYRef.current),
  });

  const SuperFlyIcon = superDrag ? resolveCategoryIcon(superDrag.iconKey) : null;
  const demoteTargetName =
    superDrop?.kind === "demote"
      ? supers.find((superCategory) => superCategory.id === superDrop.targetSuperId)?.name
      : null;
  const superDragHint =
    superDrag && demoteTargetName
      ? t("categories.super.drag.demoteHint", {
          name: demoteTargetName,
          count: catsBySuper.get(superDrag.superId)?.length ?? 0,
        })
      : null;

  useLayoutEffect(() => {
    const element = superFlyRef.current;
    if (!superDrag || !element) return;
    if (superSnap) {
      element.style.transition = "transform 420ms cubic-bezier(0.2, 0.9, 0.3, 1)";
      element.style.transform = `translate3d(${superSnap.left}px, ${superSnap.top}px, 0)`;
      return;
    }
    element.style.transition = "none";
    element.style.transform = `translate3d(${superDrag.lockedX}px, ${
      cursorYRef.current - superDrag.height / 2
    }px, 0)`;
  }, [superDrag, superSnap]);

  // —— 手写 FLIP：大类整组（左卡 + 右行）reorder 时的滑动动画 ——
  // 用 useAutoAnimate 时整组从来不动 —— 大概率是 React 19 strict mode 双挂载、
  // 或 portal 子节点 + MutationObserver 时序问题。改成手写 FLIP，行为可预期 + 已验证。
  //
  // 标准 FLIP：
  //   1. layout 前：清掉前一轮残留 transform 测纯 layout（newTop）
  //   2. 跟上一轮存的 prevTop 对比，dy = prevTop - newTop（元素"曾经在"的位置）
  //   3. transform: translateY(dy) 视觉回到 dy 位置
  //   4. 双 rAF 后清 transform + 开 transition → 沿 transition 滑回 newTop
  // 落地时 superSnap !== null：跳过动画，所有元素立刻就位
  const prevSuperTops = useRef<Map<string, number>>(new Map());
  useLayoutEffect(() => {
    const skipAnim = superSnap !== null;
    const newTops = new Map<string, number>();
    superRowRefs.current.forEach((el, id) => {
      // 先清前一轮 transform / transition，测纯 layout
      el.style.transition = "none";
      el.style.transform = "";
      const newTop = el.getBoundingClientRect().top;
      newTops.set(id, newTop);
      if (skipAnim) return;
      const prevTop = prevSuperTops.current.get(id);
      if (prevTop === undefined) return;
      const dy = prevTop - newTop;
      if (Math.abs(dy) < 0.5) return;
      el.style.transform = `translateY(${dy}px)`;
      // 双 rAF：先 commit transform，再开 transition 还原
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          el.style.transition = "transform 480ms cubic-bezier(0.2, 0.9, 0.3, 1)";
          el.style.transform = "";
        });
      });
    });
    prevSuperTops.current = newTops;
  }, [effectiveSupers, superSnap]);

  return (
    <>
      {dragError && (
        <div className={styles.dragError} role="alert">
          <span className={styles.dragErrorText}>
            {t(
              dragError.rollback === "partial"
                ? "categories.super.drag.partialFailure"
                : dragError.kind === "promote"
                  ? "categories.super.drag.promoteFailed"
                  : "categories.super.drag.demoteFailed",
            )}
          </span>
          <button
            type="button"
            className={styles.dragErrorClose}
            onClick={() => setDragError(null)}
            aria-label={t("common.close")}
            title={t("common.close")}
          >
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      )}
      <div className={styles.table}>
        {effectiveSupers.map((sup) => (
          <SuperRow
            key={sup.id}
            ref={getSuperRowRef(sup.id)}
            sup={sup}
            cats={catsBySuper.get(sup.id) ?? []}
            dragging={drag !== null}
            isHot={
              catDrop?.kind === "row" &&
              catDrop.superKey === sup.id &&
              drag?.sourceSuperKey !== sup.id
            }
            draggingCatId={drag?.catId ?? null}
            justCreatedCatId={justCreatedCatId}
            landedCatId={landedCatId}
            justCreated={justCreatedSuperId === sup.id}
            isLanded={landedSuperId === sup.id}
            isDemoteTarget={superDrop?.kind === "demote" && superDrop.targetSuperId === sup.id}
            onGripMouseDown={handleGripMouseDown}
            registerRowRef={registerCatRowRef}
            onSuperMouseDown={handleSuperMouseDown}
            isSuperDragSource={superDrag?.superId === sup.id}
            isSuperDragTarget={
              superDrop?.kind === "reorder" &&
              superDrop.targetSuperId === sup.id &&
              superDrag?.superId !== sup.id
            }
          />
        ))}
        <OrphanRow
          ref={getSuperRowRef(ORPHAN_KEY)}
          cats={orphans}
          dragging={drag !== null}
          isHot={
            catDrop?.kind === "row" &&
            catDrop.superKey === ORPHAN_KEY &&
            drag?.sourceSuperKey !== ORPHAN_KEY
          }
          draggingCatId={drag?.catId ?? null}
          justCreatedCatId={justCreatedCatId}
          landedCatId={landedCatId}
          onGripMouseDown={handleGripMouseDown}
          registerRowRef={registerCatRowRef}
        />
        {/* 隐藏 builtin 独立成行——不参与拖拽 drop（不入 superRowRefs Y-band 快照）；
          hidden cat 的 grip mousedown 也被 CategoryRow 内部 return 拦掉 */}
        {builtins.length > 0 && <BuiltinRow cats={builtins} registerRowRef={registerCatRowRef} />}
      </div>

      {/* —— 拖拽落点预览：整层通过 transform 更新，内容只在命中类型变化时重渲染 —— */}
      {catDropForLine &&
        catVisual &&
        FlyIcon &&
        createPortal(
          <div ref={catOverlayRef} className={styles.dragOverlay} style={{ width: 280 }}>
            <div className={styles.dropLine} />
            {catDropForLine.kind === "promote" && (
              <div
                className={styles.promoteGhost}
                style={{ "--drag-color": catVisual.color } as CSSProperties}
              >
                <span className={styles.promoteGhostIcon}>
                  <FlyIcon size={20} strokeWidth={1.85} />
                </span>
                <span className={styles.promoteGhostName}>{catVisual.displayName}</span>
              </div>
            )}
          </div>,
          document.body,
        )}

      {/* —— Fly chip (portal)：X 锁源列、宽度跟源行；高度走 collapsed clamp（见 flyHeight）——
          关键：portal 必须放在 .table div **外面**——放在里面时，每次 drag/superDrag
          state 变化都会让 React 重新协调 .table 的 children 列表（portal 在 JSX 树里算
          .table 的子节点，虽然 DOM 上 portal 到 body）。这会干扰 useAutoAnimate 对子节点
          mutation 的检测，导致排序动画不触发。 */}
      {catVisual &&
        FlyIcon &&
        createPortal(
          <div
            ref={catFlyRef}
            className={catStyles.catFlyChip}
            style={
              {
                left: 0,
                top: 0,
                width: catVisual.width,
                height: flyHeight,
                opacity: 1,
                willChange: "transform, opacity",
                "--cat-color": catVisual.color,
              } as CSSProperties
            }
          >
            <span className={catStyles.catFlyIcon}>
              <FlyIcon size={20} strokeWidth={1.85} />
            </span>
            <span className={catStyles.catFlyName}>{catVisual.displayName}</span>
            {catDragHint && <span className={catStyles.catFlyHint}>{catDragHint}</span>}
          </div>,
          document.body,
        )}

      {/* —— Super fly-box (portal)：大类拖动时的整组预览 ——
          关键设计：**直接复用源 .superBox / .superCard / .superBody 三件套**，
          只用 inline style 覆盖定位 (position:fixed + left/top) 与"举起来"的
          drop-shadow / z-index / pointer-events / 锁宽。这样 fly-box 跟源 superBox
          1:1 同款，没有任何 .superFly* 分支 CSS 漂移空间——
          边框宽度、box-shadow、background color-mix 比例、padding、border-radius
          全部跟源完全一致。 */}
      {superDrag &&
        SuperFlyIcon &&
        createPortal(
          <div
            ref={superFlyRef}
            className={styles.superBox}
            style={
              {
                position: "fixed",
                zIndex: 9999,
                pointerEvents: "none",
                left: 0,
                top: 0,
                width: superDrag.width,
                filter:
                  "drop-shadow(0 12px 28px rgba(var(--shadow-rgb), calc(0.18 * var(--shadow-strength))))",
                willChange: "transform",
                transform: `translate3d(${superDrag.lockedX}px, ${
                  cursorYRef.current - superDrag.height / 2
                }px, 0)`,
                "--sup-color": superDrag.color,
              } as CSSProperties
            }
          >
            <div className={styles.superCard}>
              <span className={styles.superIcon}>
                <SuperFlyIcon size={24} strokeWidth={1.85} />
              </span>
              <div className={styles.superNameWrap}>
                <span className={styles.superName}>{superDrag.displayName}</span>
              </div>
            </div>
            <div className={styles.superBody}>
              {/* 用完整 CategoryRow 渲染，跟源行视觉 1:1。不传 registerRowRef
                  避免污染父的 catRowRefs map */}
              {(catsBySuper.get(superDrag.superId) ?? []).map((cat) => (
                <CategoryRow key={cat.id} category={cat} />
              ))}
            </div>
            {superDragHint && <span className={styles.superFlyHint}>{superDragHint}</span>}
          </div>,
          document.body,
        )}
    </>
  );
}
