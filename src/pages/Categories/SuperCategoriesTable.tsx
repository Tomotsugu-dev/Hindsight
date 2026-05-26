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
import { Folder, X } from "lucide-react";
import { useCategories } from "../../state/categories";
import { useSuperCategories } from "../../state/superCategories";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { displayCategoryName, displaySuperCategoryName } from "../../utils/categoryName";
import type { Category, SuperCategory } from "../../api/hindsight";
import { CategoryRow } from "./CategoryRow";
import styles from "./SuperCategoriesTable.module.css";
import catStyles from "./Categories.module.css";

/** orphan 行用这个 sentinel 作 super-id（避免跟真 super id 冲突） */
const ORPHAN_KEY = "__ORPHAN__";

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
  /** 当前鼠标 Y（屏幕坐标） */
  cursorY: number;
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
  /** 当前鼠标 Y */
  cursorY: number;
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
export function SuperCategoriesTable() {
  const { t } = useTranslation();
  const { categories, reorder: reorderCategories } = useCategories();
  const { supers, assignCategory, reorder: reorderSupers } = useSuperCategories();

  // super-card / orphan-card 的 DOM ref —— 拍 Y 范围快照判断 cursor 落在哪个 super 区
  const superRowRefs = useRef<Map<string, HTMLElement>>(new Map());
  const setSuperRowRef = useCallback(
    (key: string) => (el: HTMLElement | null) => {
      if (el) superRowRefs.current.set(key, el);
      else superRowRefs.current.delete(key);
    },
    [],
  );

  // 每行 CategoryRow 的 DOM ref —— 同大类内 reorder 时拍 Y 范围做行命中
  const catRowRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const registerCatRowRef = useCallback(
    (id: string, el: HTMLDivElement | null) => {
      if (el) catRowRefs.current.set(id, el);
      else catRowRefs.current.delete(id);
    },
    [],
  );

  const [drag, setDrag] = useState<DragState | null>(null);
  // 跨大类 drop 目标：cursor 落在哪个 super 上（≠ sourceSuperKey 时表示要跨）
  const [hotSuperKey, setHotSuperKey] = useState<string | null>(null);
  // 同大类内 reorder 目标：cursor 落在源大类内哪一行上（用作 insertion target）
  const [hoverRowId, setHoverRowId] = useState<string | null>(null);
  // super 级 Y 范围快照
  const snapshotYRanges = useRef<Map<string, { top: number; bottom: number }>>(
    new Map(),
  );
  // 源大类内每行 Y 范围快照（仅在 drag start 时按 source super 的 cats 拍一次）
  const snapshotRowYRanges = useRef<Map<string, { top: number; bottom: number }>>(
    new Map(),
  );

  // 大类拖动 reorder 的独立 state（跟上面 cat drag 互斥）
  const [superDrag, setSuperDrag] = useState<SuperDragState | null>(null);
  /** 大类拖动期间，cursor Y 命中哪个 super 作 insertion target（≠ sourceSuper 时表示要换位） */
  const [hoverSuperReorderId, setHoverSuperReorderId] = useState<string | null>(
    null,
  );
  /** 大类松手后 fly-box 平滑"吸"到落地位置的目标坐标。
   *  非 null 时 fly-box 走 CSS transition 从 cursor → target，over ~200ms。
   *  动画期间 superDrag / hoverSuperReorderId 都保留——保持源 superBox 处在投机重排位置
   *  （opacity 0.3 占位），fly-box 滑到与它重叠的位置时再统一清掉 + commit reorder。 */
  const [superSnap, setSuperSnap] = useState<{ left: number; top: number } | null>(
    null,
  );

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

  // 同大类内 reorder 中的"投机重排"——只动源大类内行的顺序，FLIP 据此播 row 滑动。
  // 注意 deps：只能依赖 drag 的 catId / sourceSuperKey（拖动期间不变），不能依赖整个
  // drag 对象——后者每次 mousemove 都因 cursorY 更新而变身份，会导致 useMemo 每帧
  // 返回新引用 → FLIP useLayoutEffect 每帧重置 transform → 动画从来没机会播完。
  const dragCatId = drag?.catId ?? null;
  const dragSourceSuperKey = drag?.sourceSuperKey ?? null;
  const { effectiveCatsBySuper, effectiveOrphans } = useMemo(() => {
    if (!dragCatId || !hoverRowId || hoverRowId === dragCatId) {
      return { effectiveCatsBySuper: catsBySuper, effectiveOrphans: orphans };
    }
    const reorder = (arr: Category[]) => {
      const fromIdx = arr.findIndex((c) => c.id === dragCatId);
      const toIdx = arr.findIndex((c) => c.id === hoverRowId);
      if (fromIdx < 0 || toIdx < 0 || fromIdx === toIdx) return arr;
      const copy = [...arr];
      const [moved] = copy.splice(fromIdx, 1);
      copy.splice(toIdx, 0, moved);
      return copy;
    };
    if (dragSourceSuperKey === ORPHAN_KEY) {
      return { effectiveCatsBySuper: catsBySuper, effectiveOrphans: reorder(orphans) };
    }
    if (!dragSourceSuperKey) {
      return { effectiveCatsBySuper: catsBySuper, effectiveOrphans: orphans };
    }
    const sourceCats = catsBySuper.get(dragSourceSuperKey);
    if (!sourceCats) {
      return { effectiveCatsBySuper: catsBySuper, effectiveOrphans: orphans };
    }
    const nextMap = new Map(catsBySuper);
    nextMap.set(dragSourceSuperKey, reorder(sourceCats));
    return { effectiveCatsBySuper: nextMap, effectiveOrphans: orphans };
  }, [catsBySuper, orphans, dragCatId, dragSourceSuperKey, hoverRowId]);

  // 大类拖动时的"投机重排"：当 cursor 命中其他 super，supers 数组实时按"会落在哪"
  // 的顺序渲染。手写 FLIP（见 prevSuperTops 那段）据此把整个 superBox（左卡 + 右行）
  // 平滑滑到新位置——右侧小类跟着大类卡一起动，跟小类内拖动同款效果。
  // deps 只用 superDrag.superId（drag 期间稳定），跟 effectiveCatsBySuper 同理：
  // 不能传整个 superDrag 对象，cursorY 每帧变会让 useMemo 每帧返回新数组 → FLIP 死循环。
  const superDragId = superDrag?.superId ?? null;
  const effectiveSupers = useMemo(() => {
    if (!superDragId || !hoverSuperReorderId || hoverSuperReorderId === superDragId) {
      return supers;
    }
    const fromIdx = supers.findIndex((s) => s.id === superDragId);
    const toIdx = supers.findIndex((s) => s.id === hoverSuperReorderId);
    if (fromIdx < 0 || toIdx < 0 || fromIdx === toIdx) return supers;
    const out = [...supers];
    const [moved] = out.splice(fromIdx, 1);
    out.splice(toIdx, 0, moved);
    return out;
  }, [supers, superDragId, hoverSuperReorderId]);

  const handleGripMouseDown = useCallback(
    (cat: Category, e: ReactMouseEvent<HTMLButtonElement>, rowEl: HTMLDivElement) => {
      if (e.button !== 0) return; // 只接左键
      e.preventDefault();
      const rect = rowEl.getBoundingClientRect();
      const sourceSuperKey =
        cat.superCategoryId && supers.some((s) => s.id === cat.superCategoryId)
          ? cat.superCategoryId
          : ORPHAN_KEY;

      // —— super-level 快照（决定 cursor 落在哪个大类）——
      const snap = new Map<string, { top: number; bottom: number }>();
      superRowRefs.current.forEach((el, key) => {
        const r = el.getBoundingClientRect();
        snap.set(key, { top: r.top, bottom: r.bottom });
      });
      snapshotYRanges.current = snap;

      // —— row-level 快照（仅源大类内的行，用于同大类内 reorder 命中）——
      const sourceCats =
        sourceSuperKey === ORPHAN_KEY
          ? categories.filter(
              (c) =>
                !c.builtin &&
                (!c.superCategoryId ||
                  !supers.some((s) => s.id === c.superCategoryId)),
            )
          : categories.filter((c) => c.superCategoryId === sourceSuperKey);
      const rowSnap = new Map<string, { top: number; bottom: number }>();
      for (const sc of sourceCats) {
        const el = catRowRefs.current.get(sc.id);
        if (!el) continue;
        const r = el.getBoundingClientRect();
        rowSnap.set(sc.id, { top: r.top, bottom: r.bottom });
      }
      snapshotRowYRanges.current = rowSnap;

      setDrag({
        catId: cat.id,
        displayName: cat.name,
        color: cat.color,
        iconKey: cat.icon,
        lockedX: rect.left,
        width: rect.width,
        height: rect.height,
        sourceSuperKey,
        cursorY: e.clientY,
      });
    },
    [supers, categories],
  );

  // 全局 mousemove / mouseup —— drag 期间 attach，结束 detach
  useEffect(() => {
    if (!drag) return;
    // 拖动期间整个文档 cursor 锁成 grabbing —— fly-chip 是 pointer-events:none，
    // cursor 会穿透到下面元素，不锁 body 就会变成下方元素的 cursor（pointer/text 等）
    const prevCursor = document.body.style.cursor;
    document.body.style.cursor = "grabbing";
    const onMove = (e: MouseEvent) => {
      setDrag((d) => (d ? { ...d, cursorY: e.clientY } : null));
      // super-level Y-band 命中（决定 cursor 落在哪个大类）
      let hitSuper: string | null = null;
      for (const [key, r] of snapshotYRanges.current) {
        if (e.clientY >= r.top && e.clientY <= r.bottom) {
          hitSuper = key;
          break;
        }
      }
      // 在源大类内 → reorder 模式：找 cursor 落在哪行
      if (hitSuper === drag.sourceSuperKey) {
        let hitRow: string | null = null;
        for (const [id, r] of snapshotRowYRanges.current) {
          if (e.clientY >= r.top && e.clientY <= r.bottom) {
            hitRow = id;
            break;
          }
        }
        setHoverRowId(hitRow);
        setHotSuperKey(null);
      } else {
        // 在别的大类（或没命中任何 super）→ cross-super 模式
        setHoverRowId(null);
        setHotSuperKey(hitSuper);
      }
    };
    const onUp = () => {
      const cur = drag;
      const targetSuper = hotSuperKey;
      const targetRow = hoverRowId;
      setDrag(null);
      setHotSuperKey(null);
      setHoverRowId(null);
      if (!cur) return;

      // 同大类内 reorder：把 targetRow 当 insertion target
      if (targetRow && targetRow !== cur.catId) {
        const allIds = categories.map((c) => c.id);
        const fromIdx = allIds.indexOf(cur.catId);
        const toIdx = allIds.indexOf(targetRow);
        if (fromIdx >= 0 && toIdx >= 0 && fromIdx !== toIdx) {
          const reordered = [...allIds];
          reordered.splice(fromIdx, 1);
          reordered.splice(toIdx, 0, cur.catId);
          void reorderCategories(reordered);
        }
        return;
      }

      // 跨大类归属变更
      if (!targetSuper) return;
      if (targetSuper === cur.sourceSuperKey) return; // 落回源 = no-op
      const newSuperId = targetSuper === ORPHAN_KEY ? null : targetSuper;
      void assignCategory(cur.catId, newSuperId);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = prevCursor;
    };
  }, [drag, hotSuperKey, hoverRowId, assignCategory, reorderCategories, categories]);

  const FlyIcon = drag ? resolveCategoryIcon(drag.iconKey) : null;
  // 用户如果先 hover 让 AppList 展开后才抓 grip，drag.height 会测到展开值（>150px），
  // 那种 fly-chip 视觉太重；clamp 到 96px collapsed 高度兜底
  const flyHeight = drag ? Math.min(drag.height, 96) : 0;

  // —— 大类卡片 mousedown 启动 super drag。除 icon/name/del 按钮以外的区域触发 —— //
  // 注意：传入的是**整个 .superBox section**，不是 .superCard，
  // 这样测出来的 width 才包含右侧 CategoryRow 区域，fly-box 1:1 复刻整组。
  const handleSuperMouseDown = useCallback(
    (
      sup: SuperCategory,
      e: ReactMouseEvent<HTMLDivElement>,
      boxEl: HTMLElement,
    ) => {
      if (e.button !== 0) return;
      // mousedown 落在按钮 / input / role=button 元素上则不当 drag（让它们自己 click / dblclick）
      // role=button 用于排除 superName span（双击改名）等"语义按钮但非 <button> tag"的节点
      const target = e.target as HTMLElement;
      if (target.closest('button, input, [role="button"]')) return;
      e.preventDefault();
      const rect = boxEl.getBoundingClientRect();
      // 拍 super 级 Y 范围快照（用 superRowRefs 已经在 cat-drag 路径下注册好的）
      const snap = new Map<string, { top: number; bottom: number }>();
      superRowRefs.current.forEach((el, key) => {
        const r = el.getBoundingClientRect();
        snap.set(key, { top: r.top, bottom: r.bottom });
      });
      snapshotYRanges.current = snap;
      setSuperDrag({
        superId: sup.id,
        displayName: sup.name,
        color: sup.color,
        iconKey: sup.icon,
        lockedX: rect.left,
        width: rect.width,
        height: rect.height,
        cursorY: e.clientY,
      });
    },
    [],
  );

  // —— super drag 全局 mousemove / mouseup —— //
  useEffect(() => {
    if (!superDrag) return;
    // 整个文档 cursor 锁成 grabbing —— 同 cat drag 那边的理由
    const prevCursor = document.body.style.cursor;
    document.body.style.cursor = "grabbing";
    const onMove = (e: MouseEvent) => {
      setSuperDrag((d) => (d ? { ...d, cursorY: e.clientY } : null));
      // Y-band 命中其他 super 卡片：用相同的 snapshotYRanges，但跳过 ORPHAN_KEY
      // （orphan / builtin 行不参与 super 排序）
      let hit: string | null = null;
      for (const [key, r] of snapshotYRanges.current) {
        if (key === ORPHAN_KEY) continue;
        if (e.clientY >= r.top && e.clientY <= r.bottom) {
          hit = key;
          break;
        }
      }
      setHoverSuperReorderId(hit);
    };
    const onUp = () => {
      const cur = superDrag;
      const target = hoverSuperReorderId;
      // 没目标 / 落回自己 → 直接清掉，不放 snap 动画（fly-box 应"原地消失"）
      if (!cur || !target || target === cur.superId) {
        setSuperDrag(null);
        setHoverSuperReorderId(null);
        setSuperSnap(null);
        return;
      }
      // 落到别的大类：测量"源 superBox"当前 DOM 位置——因 effectiveSupers 已投机重排，
      // 它的当前 rect == 最终落地位置。set superSnap，让 fly-box CSS transition 滑过去。
      const sourceEl = superRowRefs.current.get(cur.superId);
      const ids = supers.map((s) => s.id);
      const fromIdx = ids.indexOf(cur.superId);
      const toIdx = ids.indexOf(target);
      const commit = () => {
        if (fromIdx >= 0 && toIdx >= 0 && fromIdx !== toIdx) {
          const reordered = [...ids];
          reordered.splice(fromIdx, 1);
          reordered.splice(toIdx, 0, cur.superId);
          void reorderSupers(reordered);
        }
      };
      if (!sourceEl) {
        // 拿不到 DOM 兜底：跳过 snap 直接 commit
        setSuperDrag(null);
        setHoverSuperReorderId(null);
        setSuperSnap(null);
        commit();
        return;
      }
      const rect = sourceEl.getBoundingClientRect();
      setSuperSnap({ left: rect.left, top: rect.top });
      // CSS transition 跑完后再清 state + commit reorder。期间 superDrag 仍 truthy，
      // 投机重排 + opacity 0.3 占位都保留，视觉上 fly-box 平滑落地后整组无缝接管。
      // 420ms 与下面 transition 时长保持一致
      setTimeout(() => {
        setSuperDrag(null);
        setHoverSuperReorderId(null);
        setSuperSnap(null);
        commit();
      }, 420);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = prevCursor;
    };
  }, [superDrag, hoverSuperReorderId, supers, reorderSupers]);

  const SuperFlyIcon = superDrag ? resolveCategoryIcon(superDrag.iconKey) : null;

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
    <div className={styles.table}>
      {effectiveSupers.map((sup) => (
        <SuperRow
          key={sup.id}
          ref={setSuperRowRef(sup.id)}
          sup={sup}
          cats={effectiveCatsBySuper.get(sup.id) ?? []}
          dragging={drag !== null}
          isHot={hotSuperKey === sup.id && drag?.sourceSuperKey !== sup.id}
          draggingCatId={drag?.catId ?? null}
          onGripMouseDown={handleGripMouseDown}
          registerRowRef={registerCatRowRef}
          onSuperMouseDown={handleSuperMouseDown}
          isSuperDragSource={superDrag?.superId === sup.id}
          isSuperDragTarget={
            hoverSuperReorderId === sup.id && superDrag?.superId !== sup.id
          }
        />
      ))}
      <OrphanRow
        ref={setSuperRowRef(ORPHAN_KEY)}
        cats={effectiveOrphans}
        dragging={drag !== null}
        isHot={hotSuperKey === ORPHAN_KEY && drag?.sourceSuperKey !== ORPHAN_KEY}
        draggingCatId={drag?.catId ?? null}
        onGripMouseDown={handleGripMouseDown}
        registerRowRef={registerCatRowRef}
      />
      {/* 隐藏 builtin 独立成行——不参与拖拽 drop（不入 superRowRefs Y-band 快照）；
          hidden cat 的 grip mousedown 也被 CategoryRow 内部 return 拦掉 */}
      {builtins.length > 0 && (
        <BuiltinRow cats={builtins} registerRowRef={registerCatRowRef} />
      )}
    </div>

      {/* —— Fly chip (portal)：X 锁源列、宽度跟源行；高度走 collapsed clamp（见 flyHeight）——
          关键：portal 必须放在 .table div **外面**——放在里面时，每次 drag/superDrag
          state 变化都会让 React 重新协调 .table 的 children 列表（portal 在 JSX 树里算
          .table 的子节点，虽然 DOM 上 portal 到 body）。这会干扰 useAutoAnimate 对子节点
          mutation 的检测，导致排序动画不触发。 */}
      {drag &&
        FlyIcon &&
        createPortal(
          <div
            className={catStyles.catFlyChip}
            style={
              {
                left: drag.lockedX,
                top: drag.cursorY - flyHeight / 2,
                width: drag.width,
                height: flyHeight,
                "--cat-color": drag.color,
              } as CSSProperties
            }
          >
            <span className={catStyles.catFlyIcon}>
              <FlyIcon size={20} strokeWidth={1.85} />
            </span>
            <span className={catStyles.catFlyName}>
              {displayCategoryName({ id: drag.catId, name: drag.displayName }, t)}
            </span>
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
            className={styles.superBox}
            style={
              {
                position: "fixed",
                zIndex: 9999,
                pointerEvents: "none",
                // snap 阶段：left/top 切到落地位置 + 加 transition；正常拖动期间用 cursor
                left: superSnap ? superSnap.left : superDrag.lockedX,
                top: superSnap
                  ? superSnap.top
                  : superDrag.cursorY - superDrag.height / 2,
                width: superDrag.width,
                filter: "drop-shadow(0 12px 28px rgba(20, 20, 40, 0.18))",
                transition: superSnap
                  ? "left 420ms cubic-bezier(0.2, 0.9, 0.3, 1), top 420ms cubic-bezier(0.2, 0.9, 0.3, 1)"
                  : undefined,
                "--sup-color": superDrag.color,
              } as CSSProperties
            }
          >
            <div className={styles.superCard}>
              <span className={styles.superIcon}>
                <SuperFlyIcon size={24} strokeWidth={1.85} />
              </span>
              <div className={styles.superNameWrap}>
                <span className={styles.superName}>
                  {displaySuperCategoryName(
                    { id: superDrag.superId, name: superDrag.displayName },
                    t,
                  )}
                </span>
              </div>
            </div>
            <div className={styles.superBody}>
              {/* 用完整 CategoryRow 渲染，跟源行视觉 1:1。不传 registerRowRef
                  避免污染父的 catRowRefs map */}
              {(catsBySuper.get(superDrag.superId) ?? []).map((cat) => (
                <CategoryRow key={cat.id} category={cat} />
              ))}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

interface SuperRowProps {
  sup: SuperCategory;
  cats: Category[];
  dragging: boolean;
  isHot: boolean;
  draggingCatId: string | null;
  onGripMouseDown: (
    cat: Category,
    e: ReactMouseEvent<HTMLButtonElement>,
    rowEl: HTMLDivElement,
  ) => void;
  registerRowRef: (id: string, el: HTMLDivElement | null) => void;
  /** 大类卡片 mousedown 启动 super-drag。父控制 drag state。
   *  传整个 .superBox section 进去（用于测 fly-box 的左 X + 整宽），不是 .superCard。 */
  onSuperMouseDown: (
    sup: SuperCategory,
    e: ReactMouseEvent<HTMLDivElement>,
    boxEl: HTMLElement,
  ) => void;
  /** super-drag 期间：是否本卡正在被拖（淡化占位） */
  isSuperDragSource: boolean;
  /** super-drag 期间：是否本卡是其它 super 的命中目标（dropHot） */
  isSuperDragTarget: boolean;
}

function SuperRow_({
  sup,
  cats,
  dragging,
  isHot,
  draggingCatId,
  onGripMouseDown,
  registerRowRef,
  onSuperMouseDown,
  isSuperDragSource,
  isSuperDragTarget,
  rowRef,
}: SuperRowProps & { rowRef: (el: HTMLElement | null) => void }) {
  const { t } = useTranslation();
  const { update, remove } = useSuperCategories();
  const Icon = resolveCategoryIcon(sup.icon);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [editingName, setEditingName] = useState(false);
  const [draftName, setDraftName] = useState(sup.name);
  const inputRef = useRef<HTMLInputElement>(null);
  // 本地 ref → super card DOM；onMouseDown 时把 rect 给父
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (editingName) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editingName]);
  useEffect(() => {
    if (!editingName) setDraftName(sup.name);
  }, [sup.name, editingName]);

  const commitName = async () => {
    const trimmed = draftName.trim();
    if (trimmed && trimmed !== sup.name) {
      await update(sup.id, { name: trimmed });
    } else {
      setDraftName(sup.name);
    }
    setEditingName(false);
  };
  const cancelName = () => {
    setDraftName(sup.name);
    setEditingName(false);
  };

  const cardClass = [
    styles.superCard,
    dragging ? styles.dropTarget : "",
    isHot ? styles.dropHot : "",
    isSuperDragTarget ? styles.superCardDropHot : "",
  ]
    .filter(Boolean)
    .join(" ");

  // 整个大类组（左卡 + 右行）作为一个单位淡化，保持视觉一致性
  const boxClass = [
    styles.superBox,
    isSuperDragSource ? styles.superBoxSource : "",
  ]
    .filter(Boolean)
    .join(" ");

  const handleCardMouseDown = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (!cardRef.current) return;
    // 取 section（superBox），不是 card 本身——父要用整个 superBox 的 rect 测 fly-box 宽
    const boxEl = cardRef.current.parentElement;
    if (!boxEl) return;
    onSuperMouseDown(sup, e, boxEl);
  };

  return (
    <section
      ref={rowRef}
      className={boxClass}
      style={{ "--sup-color": sup.color } as CSSProperties}
    >
      <div ref={cardRef} className={cardClass} onMouseDown={handleCardMouseDown}>
        <div className={styles.iconPickerAnchor}>
          <button
            type="button"
            className={styles.superIcon}
            onClick={() => setPickerOpen((v) => !v)}
            aria-label={t("categories.list.iconBtnAria")}
            title={t("categories.list.iconBtnTitle")}
          >
            <Icon size={24} strokeWidth={1.85} />
          </button>
          {pickerOpen && (
            <AppearancePicker
              color={sup.color}
              icon={sup.icon}
              onColorChange={(c) => update(sup.id, { color: c })}
              onIconChange={(i) => {
                update(sup.id, { icon: i });
                setPickerOpen(false);
              }}
              onDismiss={() => setPickerOpen(false)}
            />
          )}
        </div>

        <div className={styles.superNameWrap}>
          {editingName ? (
            <input
              ref={inputRef}
              className={styles.superName}
              value={draftName}
              maxLength={16}
              onChange={(e) => setDraftName(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") cancelName();
              }}
            />
          ) : (
            <span
              className={styles.superName}
              role="button"
              tabIndex={0}
              onDoubleClick={() => setEditingName(true)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  setEditingName(true);
                }
              }}
            >
              {displaySuperCategoryName(sup, t)}
            </span>
          )}
        </div>

        <button
          type="button"
          className={styles.superClose}
          onClick={() => setConfirmOpen(true)}
          aria-label={t("categories.super.deleteAria")}
          title={t("categories.super.deleteAria")}
        >
          <X size={12} strokeWidth={2.4} />
        </button>
      </div>

      <SuperBody
        cats={cats}
        emptyHint={t("categories.super.emptyHint")}
        draggingCatId={draggingCatId}
        onGripMouseDown={onGripMouseDown}
        registerRowRef={registerRowRef}
      />

      <ConfirmDialog
        open={confirmOpen}
        title={t("categories.super.deleteDialog.title")}
        message={t("categories.super.deleteDialog.message", {
          name: displaySuperCategoryName(sup, t),
        })}
        confirmLabel={t("categories.deleteDialog.confirm")}
        variant="danger"
        onConfirm={async () => {
          setConfirmOpen(false);
          await remove(sup.id);
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </section>
  );
}

/** SuperRow 用 forwardRef 把 section DOM 引用暴露给父；父用它做 Y-band 快照 */
const SuperRow = ({
  ref,
  ...props
}: SuperRowProps & { ref: (el: HTMLElement | null) => void }) => (
  <SuperRow_ rowRef={ref} {...props} />
);

interface OrphanRowProps {
  cats: Category[];
  dragging: boolean;
  isHot: boolean;
  draggingCatId: string | null;
  onGripMouseDown: (
    cat: Category,
    e: ReactMouseEvent<HTMLButtonElement>,
    rowEl: HTMLDivElement,
  ) => void;
  registerRowRef: (id: string, el: HTMLDivElement | null) => void;
}

function OrphanRow_({
  cats,
  dragging,
  isHot,
  draggingCatId,
  onGripMouseDown,
  registerRowRef,
  rowRef,
}: OrphanRowProps & { rowRef: (el: HTMLElement | null) => void }) {
  const { t } = useTranslation();
  const cardClass = [
    styles.superCard,
    dragging ? styles.dropTarget : "",
    isHot ? styles.dropHot : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section ref={rowRef} className={`${styles.superBox} ${styles.orphan}`}>
      <div className={cardClass}>
        <span className={styles.superIcon} aria-hidden>
          <Folder size={24} strokeWidth={1.85} />
        </span>
        <div className={styles.superNameWrap}>
          <span className={styles.superName}>
            {t("categories.super.orphanLabel")}
          </span>
        </div>
      </div>
      <SuperBody
        cats={cats}
        emptyHint={t("categories.super.orphanEmptyHint")}
        draggingCatId={draggingCatId}
        onGripMouseDown={onGripMouseDown}
        registerRowRef={registerRowRef}
      />
    </section>
  );
}

const OrphanRow = ({
  ref,
  ...props
}: OrphanRowProps & { ref: (el: HTMLElement | null) => void }) => (
  <OrphanRow_ rowRef={ref} {...props} />
);

/** 「隐藏」builtin 行：极简渲染，**不展示左侧大类 label 列**——直接一个无 header 的卡片
 *  装 hidden CategoryRow。视觉上跟大类卡同款边框 / 阴影，但少了 grid 左列。
 *
 *  - 不入 superRowRefs → 拖动时不是 drop target
 *  - hidden cat 的 grip 在 CategoryRow 内部 builtin 拦截，也无法被拖出
 */
function BuiltinRow({
  cats,
  registerRowRef,
}: {
  cats: Category[];
  registerRowRef: (id: string, el: HTMLDivElement | null) => void;
}) {
  return (
    <section className={styles.builtinBox}>
      {cats.map((cat) => (
        <CategoryRow
          key={cat.id}
          category={cat}
          registerRowRef={registerRowRef}
        />
      ))}
    </section>
  );
}

/** SuperRow / OrphanRow 共享的 body 渲染：列出 CategoryRow + 空态提示 + 手写 FLIP
 *  让同大类内 reorder 时行平滑滑到新位置。手写 FLIP 见 SuperCategoriesTable 内 prevSuperTops
 *  那段注释。 */
function SuperBody({
  cats,
  emptyHint,
  draggingCatId,
  onGripMouseDown,
  registerRowRef,
}: {
  cats: Category[];
  emptyHint: string;
  draggingCatId: string | null;
  onGripMouseDown: (
    cat: Category,
    e: ReactMouseEvent<HTMLButtonElement>,
    rowEl: HTMLDivElement,
  ) => void;
  registerRowRef: (id: string, el: HTMLDivElement | null) => void;
}) {
  // 本 body 内的行 DOM ref（独立于父的 catRowRefs——FLIP 只关心本 body 内重排）
  const localRowRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const prevTops = useRef<Map<string, number>>(new Map());
  const setLocalRowRef = useCallback(
    (id: string, el: HTMLDivElement | null) => {
      if (el) localRowRefs.current.set(id, el);
      else localRowRefs.current.delete(id);
      // 同时也通知父（catRowRefs 用于跨大类 drop 判定）
      registerRowRef(id, el);
    },
    [registerRowRef],
  );
  useLayoutEffect(() => {
    const newTops = new Map<string, number>();
    localRowRefs.current.forEach((el, id) => {
      el.style.transition = "none";
      el.style.transform = "";
      const newTop = el.getBoundingClientRect().top;
      newTops.set(id, newTop);
      const prevTop = prevTops.current.get(id);
      if (prevTop === undefined) return;
      const dy = prevTop - newTop;
      if (Math.abs(dy) < 0.5) return;
      el.style.transform = `translateY(${dy}px)`;
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          el.style.transition = "transform 380ms cubic-bezier(0.2, 0.9, 0.3, 1)";
          el.style.transform = "";
        });
      });
    });
    prevTops.current = newTops;
  }, [cats]);
  if (cats.length === 0) {
    return (
      <div className={styles.superBody}>
        <div className={styles.emptyHint}>{emptyHint}</div>
      </div>
    );
  }
  return (
    <div className={styles.superBody}>
      {cats.map((cat) => (
        <CategoryRow
          key={cat.id}
          category={cat}
          isDragging={draggingCatId === cat.id}
          onGripMouseDown={onGripMouseDown}
          registerRowRef={setLocalRowRef}
        />
      ))}
    </div>
  );
}
