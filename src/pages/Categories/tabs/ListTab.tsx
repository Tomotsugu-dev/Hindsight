import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Check,
  GripVertical,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { useCategories } from "../../../state/categories";
import type { Category } from "../../../api/hindsight";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { AppearancePicker } from "../../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../../config/categoryIcons";
import { displayCategoryName } from "../../../utils/categoryName";
import { AppList } from "../parts";
import { CATEGORY_PALETTE } from "../../../config/categoryIcons";
import styles from "../Categories.module.css";

const DEFAULT_NEW_ICON = "Tag";

/** 应用分类 tab：分类 CRUD + 拖拽排序 + 新建按钮。
 *  原 CategoriesPage 的 Section 1 提取到这里，CategoriesPage 退化为 tabs 外壳。 */
export default function ListTab() {
  const { t } = useTranslation();
  const { categories, loading, create, reorder, refresh } = useCategories();
  // 每次切回本 tab 强制 refetch —— CategoriesProvider 全局 mount 一次后不会自动重拉，
  // 用户后台开新 app 让 capture 写 app_group_members 时 UI 无感知，导致分类卡片
  // 一直显示"暂无绑定应用"。这里 mount 触发一次，切别的 tab 再回来就能刷新。
  useEffect(() => {
    void refresh();
  }, [refresh]);
  const [creating, setCreating] = useState(false);

  const handleCreated = async (input: { name: string; color: string; icon: string }) => {
    await create(input);
    setCreating(false);
  };

  return (
    <>
      <header className={styles.header}>
        <p className={styles.meta}>{t("categories.intro")}</p>
        <button
          type="button"
          className={styles.createBtn}
          onClick={() => setCreating(true)}
          disabled={creating}
        >
          <Plus size={14} strokeWidth={2} />
          {t("categories.newCategory")}
        </button>
      </header>

      <section className={styles.card}>
        {creating && (
          <CreatingRow
            onCommit={handleCreated}
            onCancel={() => setCreating(false)}
          />
        )}
        {loading && categories.length === 0 ? (
          <div className={styles.empty}>{t("categories.loading")}</div>
        ) : (
          <DraggableCategoryList categories={categories} onReorder={reorder} />
        )}
      </section>
    </>
  );
}

interface CategoryRowProps {
  category: Category;
  /** 拖拽相关：DraggableCategoryList 注入 */
  rowRef?: (el: HTMLDivElement | null) => void;
  isDraggingThis?: boolean;
  isHotTarget?: boolean;
  isLanded?: boolean;
  onHandleMouseDown?: (e: React.MouseEvent<HTMLButtonElement>) => void;
}

function CategoryRow({
  category,
  rowRef,
  isDraggingThis,
  isHotTarget,
  isLanded,
  onHandleMouseDown,
}: CategoryRowProps) {
  const { t } = useTranslation();
  const { update, remove } = useCategories();
  const [editingName, setEditingName] = useState(false);
  const [draftName, setDraftName] = useState(category.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(category.icon);

  useEffect(() => {
    if (editingName) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editingName]);

  useEffect(() => {
    if (!editingName) setDraftName(category.name);
  }, [category.name, editingName]);

  const commitName = async () => {
    const trimmed = draftName.trim();
    if (trimmed && trimmed !== category.name) {
      await update(category.id, { name: trimmed });
    } else {
      setDraftName(category.name);
    }
    setEditingName(false);
  };

  const cancelName = () => {
    setDraftName(category.name);
    setEditingName(false);
  };

  const onConfirmDelete = async () => {
    setConfirmOpen(false);
    await remove(category.id);
  };

  const styleVar = { "--cat-color": category.color } as CSSProperties;

  return (
    <div
      ref={rowRef}
      className={[
        styles.catRow,
        isDraggingThis ? styles.catRowSource : "",
        isHotTarget ? styles.catRowHot : "",
        isLanded ? styles.catRowLanded : "",
      ]
        .filter(Boolean)
        .join(" ")}
      style={styleVar}
    >
      <button
        type="button"
        className={styles.catDragHandle}
        onMouseDown={onHandleMouseDown}
        aria-label={t("categories.list.dragHandleAria")}
        title={t("categories.list.dragHandleAria")}
      >
        <GripVertical size={14} strokeWidth={2} />
      </button>
      <div className={styles.catIconWrap}>
        <button
          type="button"
          className={styles.catIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label={t("categories.list.iconBtnAria")}
          title={t("categories.list.iconBtnTitle")}
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={category.color}
            icon={category.icon}
            onColorChange={(c) => update(category.id, { color: c })}
            onIconChange={(i) => {
              update(category.id, { icon: i });
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.catBody}>
        <div className={styles.catNameRow}>
          {editingName ? (
            <input
              ref={inputRef}
              className={styles.catNameInput}
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
              className={styles.catName}
              role="button"
              tabIndex={0}
              onDoubleClick={() => setEditingName(true)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  setEditingName(true);
                }
              }}
              title={t("categories.list.renameTooltip")}
            >
              {displayCategoryName(category, t)}
            </span>
          )}
        </div>

        <div className={styles.appListClip}>
          <AppList category={category} />
        </div>
      </div>

      <div className={styles.catActions}>
        <button
          type="button"
          className={styles.actionBtn}
          onClick={() => setEditingName(true)}
          aria-label={t("categories.list.renameAria")}
          title={t("categories.list.renameAria")}
        >
          <Pencil size={14} strokeWidth={1.85} />
        </button>
        {/* 内置分类（如 v27 的 hidden）不允许删除：按钮不渲染，dialog 也不挂。
            后端 categories::delete 有二次防御（builtin != 0 拒绝） */}
        {!category.builtin && (
          <button
            type="button"
            className={`${styles.actionBtn} ${styles.actionBtnDanger}`}
            onClick={() => setConfirmOpen(true)}
            aria-label={t("categories.list.deleteAria")}
            title={t("categories.list.deleteAria")}
          >
            <Trash2 size={14} strokeWidth={1.85} />
          </button>
        )}
      </div>

      {!category.builtin && (
        <ConfirmDialog
          open={confirmOpen}
          title={t("categories.deleteDialog.title")}
          message={t("categories.deleteDialog.message", { name: displayCategoryName(category, t) })}
          confirmLabel={t("categories.deleteDialog.confirm")}
          variant="danger"
          onConfirm={onConfirmDelete}
          onCancel={() => setConfirmOpen(false)}
        />
      )}
    </div>
  );
}

function CreatingRow({
  onCommit,
  onCancel,
}: {
  onCommit: (input: { name: string; color: string; icon: string }) => void | Promise<void>;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [color, setColor] = useState(CATEGORY_PALETTE[0]);
  const [icon, setIcon] = useState(DEFAULT_NEW_ICON);
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(icon);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const commit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      onCancel();
      return;
    }
    await onCommit({ name: trimmed, color, icon });
  };

  const styleVar = { "--cat-color": color } as CSSProperties;

  return (
    <div className={styles.catRow} style={styleVar}>
      <div className={styles.catIconWrap}>
        <button
          type="button"
          className={styles.catIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label={t("categories.list.createIconAria")}
          title={t("categories.list.createIconAria")}
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={color}
            icon={icon}
            onColorChange={setColor}
            onIconChange={(i) => {
              setIcon(i);
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.catBody}>
        <div className={styles.catNameRow}>
          <input
            ref={inputRef}
            className={styles.catNameInput}
            placeholder={t("categories.list.namePlaceholder")}
            value={name}
            maxLength={16}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") onCancel();
            }}
          />
        </div>
      </div>

      <div className={styles.catActions}>
        <button
          type="button"
          className={`${styles.actionBtn}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={commit}
          aria-label={t("categories.list.confirm")}
          title={t("categories.list.confirm")}
        >
          <Check size={14} strokeWidth={2.25} />
        </button>
        <button
          type="button"
          className={`${styles.actionBtn} ${styles.actionBtnDanger}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={onCancel}
          aria-label={t("categories.list.cancel")}
          title={t("categories.list.cancel")}
        >
          <X size={14} strokeWidth={2.25} />
        </button>
      </div>
    </div>
  );
}

/**
 * 列表外壳：管理拖拽状态、命中检测、飞行 ghost 渲染。
 * 跟 PairingSection 的拖拽实现对齐：
 *   - mousedown 在 grip handle 上启动 drag，记下源行的 boundingRect
 *   - 飞行 ghost portal 出去到 body，X 锁源列水平位置（实际整列就是行宽，X 锁住=
 *     visually 列内移动），Y 跟随鼠标
 *   - mousemove 拿 cursorY 跟每行的 getBoundingClientRect 对比做命中
 *   - 命中行加 .catRowHot：抬升 + 描边 + 阴影，物理碰撞感
 *   - mouseup：把源 id 移到目标 id 的位置，调 reorder
 */
function DraggableCategoryList({
  categories,
  onReorder,
}: {
  categories: Category[];
  onReorder: (orderedIds: string[]) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [drag, setDrag] = useState<{
    id: string;
    name: string;
    color: string;
    iconKey: string;
    /** 源行的 X 起点 / 宽度 / 高度 —— 飞行 ghost 用 */
    leftX: number;
    width: number;
    height: number;
    cursorY: number;
  } | null>(null);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [landedId, setLandedId] = useState<string | null>(null);
  // 落地后到父组件 categories 追上 onReorder 结果之间，本地用这个数组锁住"已落新位置"。
  // 没它的话 mouseup 瞬间 drag/hoverId 一清，displayedCategories 回退到 categories（旧序），
  // FLIP 会先把行动画回旧位置，等 API 完成再动画到新位置 → 视觉上"卡一下" + 双动画。
  const [optimisticOrderIds, setOptimisticOrderIds] = useState<string[] | null>(
    null,
  );
  // mouseup 时置 true，下一次 FLIP useLayoutEffect 跳过动画 + 平滑接手逻辑，
  // 直接把所有行的 transform 清掉，立刻就位。
  // 不置这个 flag 的话，源行 mid-animation 的视觉残余会被识别为"动画中途"，
  // 再播一轮 320ms 的"平滑接手" → 看起来就是"放手后又卡了 300ms"。
  const dropSnapRef = useRef(false);
  const rowRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  // 拖动开始时快照各行 top/bottom，整个 drag 都用这份静态数据做命中检测。
  // 不能在 mousemove 时读 getBoundingClientRect()：FLIP 动画期间行位置正在动，
  // 读到的是过渡中的值，会导致 hoverId 在两行之间来回跳变 → 鬼畜。
  const snapshotRects = useRef<Map<string, { top: number; bottom: number }>>(
    new Map(),
  );

  const setRowRef = (id: string) => (el: HTMLDivElement | null) => {
    if (el) rowRefs.current.set(id, el);
    else rowRefs.current.delete(id);
  };

  // 投机重排：拖动过程中，把源行从原位置移到当前 hover 行的位置，作为"会落在哪"的预演。
  // 每次 hoverId 变（鼠标越过新一行），displayedCategories 重新计算，rows 实际重排，
  // FLIP useLayoutEffect 给每行加 translateY 动画 → 实时碰撞 / 让位的视觉效果。
  // 没在拖时 displayedCategories 直接是数据本身，零改动。
  const displayedCategories = useMemo(() => {
    // 拖动中：按 hoverId 投机重排
    if (drag && hoverId && drag.id !== hoverId) {
      const ids = categories.map((c) => c.id);
      const fromIdx = ids.indexOf(drag.id);
      const toIdx = ids.indexOf(hoverId);
      if (fromIdx >= 0 && toIdx >= 0 && fromIdx !== toIdx) {
        const reordered = [...categories];
        const [moved] = reordered.splice(fromIdx, 1);
        reordered.splice(toIdx, 0, moved);
        return reordered;
      }
    }
    // 落地后等 API：用 optimistic 顺序兜底，防止回退到 categories 旧序触发 double FLIP
    if (optimisticOrderIds) {
      const map = new Map(categories.map((c) => [c.id, c]));
      const out = optimisticOrderIds
        .map((id) => map.get(id))
        .filter((c): c is Category => c !== undefined);
      // 行数对齐才用 optimistic；categories 已有增/删时回退到真值，避免错位
      if (out.length === categories.length) return out;
    }
    return categories;
  }, [categories, drag, hoverId, optimisticOrderIds]);

  // 父组件 categories 顺序跟 optimisticOrderIds 一致后，清掉 optimistic（API 落地）
  useEffect(() => {
    if (!optimisticOrderIds) return;
    const currentIds = categories.map((c) => c.id).join(",");
    if (currentIds === optimisticOrderIds.join(",")) {
      setOptimisticOrderIds(null);
    }
  }, [categories, optimisticOrderIds]);

  // FLIP 动画：displayedCategories 顺序一变（drag 时 hover 切换 / drop 后真正 reorder），
  // 让每行从旧位置滑到新位置 —— 拖动时实时让位、drop 后落地都是同一动画机制。
  //
  // 标准 FLIP 模板：
  //   1. 测渲染前位置（上次 useLayoutEffect 末尾存的 prevLayout，纯 layout 不含 transform）
  //   2. 重置正在跑的 transform / transition → 测纯 layout 位置（newLayout）
  //   3. 选 "from" 起点：
  //        a) 上一帧动画还没跑完 → 用 visualBefore（视觉当前位置）→ 平滑接手
  //        b) 否则 → 用 prevLayout（标准 FLIP "old layout → new layout"）
  //   4. transform: translateY(from - newLayout) 让元素视觉回到 from
  //   5. 双 rAF 后清 transform → 沿 transition 滑回 newLayout
  //
  // 老版本两处坑（macOS WKWebView 上更易暴露）：
  //   - 保存基线时调 getBoundingClientRect()，但此刻 transform 还在身上 →
  //     存的是"被 transform 反向推过的位置"≈ 上次的旧位置，不是真 layout
  //   - 动画中途 hover 变了，next 读到 mid-animation 视觉位置 →
  //     计算的 dy 偏小，transition: none 后视觉跳回某个错位坐标
  const prevTops = useRef<Map<string, number>>(new Map());
  useLayoutEffect(() => {
    const snap = dropSnapRef.current;
    dropSnapRef.current = false;

    const newLayouts = new Map<string, number>();
    rowRefs.current.forEach((el, id) => {
      // 清掉正在跑的动画，取纯 layout 位置（这一步 snap 与否都需要）
      const visualBefore = el.getBoundingClientRect().top;
      el.style.transition = "none";
      el.style.transform = "";
      const newLayout = el.getBoundingClientRect().top;
      newLayouts.set(id, newLayout);

      // drop 落地：直接停在 layout 上，不再播任何过渡（含"平滑接手"也不要）
      if (snap) return;

      // 选 from：动画中途 → visualBefore（平滑接手）；否则 → prevLayout（标准 FLIP）
      const prevLayout = prevTops.current.get(id);
      const midAnim = Math.abs(visualBefore - newLayout) > 0.5;
      const from = midAnim ? visualBefore : prevLayout;
      if (from === undefined || Math.abs(from - newLayout) <= 0.5) return;

      const dy = from - newLayout;
      el.style.transform = `translateY(${dy}px)`;
      // 双 rAF 保证浏览器先 commit 上面的 transform，再开 transition 还原
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          // back ease: 过冲再回弹，给"碰到一起"加一点物理弹性
          el.style.transition =
            "transform 320ms cubic-bezier(0.34, 1.56, 0.64, 1)";
          el.style.transform = "";
        });
      });
    });
    // 存纯 layout 基线（在 forEach 内已清完 transform 后测的，不含 transform 污染）
    prevTops.current = newLayouts;
  }, [displayedCategories]);

  useEffect(() => {
    if (!drag) return;
    const onMove = (e: MouseEvent) => {
      setDrag((d) => (d ? { ...d, cursorY: e.clientY } : null));
      // 用 startDrag 时快照的静态 rect 做命中，FLIP 动画期间行位置在变也不会抖。
      let hit: string | null = null;
      for (const [id, r] of snapshotRects.current) {
        if (e.clientY >= r.top && e.clientY <= r.bottom) {
          hit = id;
          break;
        }
      }
      setHoverId(hit);
    };
    const onUp = () => {
      const cur = drag;
      const target = hoverId;
      setDrag(null);
      setHoverId(null);
      if (cur && target && target !== cur.id) {
        const ids = categories.map((c) => c.id);
        const fromIdx = ids.indexOf(cur.id);
        const toIdx = ids.indexOf(target);
        if (fromIdx >= 0 && toIdx >= 0 && fromIdx !== toIdx) {
          ids.splice(fromIdx, 1);
          ids.splice(toIdx, 0, cur.id);
          // optimistic：立刻把"落到的新顺序"锁进本地 state，
          // displayedCategories 不会回退到 categories 旧序，FLIP 不会双跳。
          // 等 onReorder 异步完成、父 categories prop 追上来后由 useEffect 清掉。
          setOptimisticOrderIds(ids);
          // 告诉下一次 FLIP：drop 落地 → 跳过动画 + 平滑接手，所有行立刻就位
          dropSnapRef.current = true;
          // 标记落地行 → CSS 给它一段 squish 动画（碰撞质感）
          setLandedId(cur.id);
          window.setTimeout(() => setLandedId(null), 360);
          void onReorder(ids);
        }
      }
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
  }, [drag, hoverId, categories, onReorder]);

  const startDrag = (
    e: React.MouseEvent<HTMLButtonElement>,
    cat: Category,
  ) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const row = rowRefs.current.get(cat.id);
    if (!row) return;
    const rect = row.getBoundingClientRect();
    // 给所有行做一份位置快照，整个 drag 命中检测都用它（不读 mid-FLIP 的实时 rect）
    const snap = new Map<string, { top: number; bottom: number }>();
    rowRefs.current.forEach((el, id) => {
      const r = el.getBoundingClientRect();
      snap.set(id, { top: r.top, bottom: r.bottom });
    });
    snapshotRects.current = snap;
    setDrag({
      id: cat.id,
      name: cat.name,
      color: cat.color,
      iconKey: cat.icon,
      leftX: rect.left,
      width: rect.width,
      height: rect.height,
      cursorY: e.clientY,
    });
  };

  const FlyIcon = drag ? resolveCategoryIcon(drag.iconKey) : null;

  return (
    <>
      {displayedCategories.map((c) => (
        <CategoryRow
          key={c.id}
          category={c}
          rowRef={setRowRef(c.id)}
          isDraggingThis={drag?.id === c.id}
          isHotTarget={!!drag && hoverId === c.id && drag.id !== c.id}
          isLanded={landedId === c.id}
          onHandleMouseDown={(e) => startDrag(e, c)}
        />
      ))}

      {drag &&
        FlyIcon &&
        createPortal(
          <div
            className={styles.catFlyChip}
            style={
              {
                left: drag.leftX,
                top: drag.cursorY - drag.height / 2,
                width: drag.width,
                height: drag.height,
                "--cat-color": drag.color,
              } as CSSProperties
            }
          >
            <span className={styles.catFlyIcon}>
              <FlyIcon size={20} strokeWidth={1.85} />
            </span>
            <span className={styles.catFlyName}>
              {displayCategoryName({ id: drag.id, name: drag.name }, t)}
            </span>
          </div>,
          document.body,
        )}
    </>
  );
}
