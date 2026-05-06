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
  Info,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { useCategories } from "../../state/categories";
import type { Category } from "../../api/hindsight";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { displayCategoryName } from "../../utils/categoryName";
import { AppList, DEFAULT_PALETTE } from "./parts";
import { PairingSection } from "./PairingSection";
import styles from "./Categories.module.css";

const DEFAULT_NEW_ICON = "Tag";

export default function CategoriesPage() {
  const { t } = useTranslation();
  const { categories, loading, create, reorder } = useCategories();
  const [creating, setCreating] = useState(false);

  const handleCreated = async (input: { name: string; color: string; icon: string }) => {
    await create(input);
    setCreating(false);
  };

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <h1 className={styles.title}>{t("categories.title")}</h1>
          <p className={styles.meta}>{t("categories.intro")}</p>
        </div>
        <button
          type="button"
          className={styles.createBtn}
          onClick={() => setCreating(true)}
          disabled={creating}
        >
          <Plus size={14} strokeWidth={2.25} />
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

      <header className={styles.header} style={{ marginTop: 8 }}>
        <div className={styles.headerText}>
          <h2 className={styles.title} style={{ fontSize: 18 }}>
            {t("categories.pairing.sectionTitle")}
            <span
              className={styles.infoTip}
              tabIndex={0}
              aria-label={t("categories.pairing.infoTipAria")}
            >
              <Info size={14} strokeWidth={2.25} />
              <span className={styles.infoTipBody} role="tooltip">
                {t("categories.pairing.infoTipBody")}
              </span>
            </span>
          </h2>
          <p className={styles.meta}>
            {t("categories.pairing.instructionPrefix")}
            <strong className={styles.metaEmph}>
              {t("categories.pairing.instructionEmph")}
            </strong>
            {t("categories.pairing.instructionSuffix")}
            <span className={styles.metaUnassigned}>
              {t("categories.pairing.unassignedHint")}
            </span>
          </p>
        </div>
      </header>

      <section className={styles.card}>
        <PairingSection />
      </section>
    </div>
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
              onDoubleClick={() => setEditingName(true)}
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
        <button
          type="button"
          className={`${styles.actionBtn} ${styles.actionBtnDanger}`}
          onClick={() => setConfirmOpen(true)}
          aria-label={t("categories.list.deleteAria")}
          title={t("categories.list.deleteAria")}
        >
          <Trash2 size={14} strokeWidth={1.85} />
        </button>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        title={t("categories.deleteDialog.title")}
        message={t("categories.deleteDialog.message", { name: displayCategoryName(category, t) })}
        confirmLabel={t("categories.deleteDialog.confirm")}
        variant="danger"
        onConfirm={onConfirmDelete}
        onCancel={() => setConfirmOpen(false)}
      />
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
  const [color, setColor] = useState(DEFAULT_PALETTE[0]);
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
    if (!drag || !hoverId || drag.id === hoverId) return categories;
    const ids = categories.map((c) => c.id);
    const fromIdx = ids.indexOf(drag.id);
    const toIdx = ids.indexOf(hoverId);
    if (fromIdx < 0 || toIdx < 0 || fromIdx === toIdx) return categories;
    const reordered = [...categories];
    const [moved] = reordered.splice(fromIdx, 1);
    reordered.splice(toIdx, 0, moved);
    return reordered;
  }, [categories, drag, hoverId]);

  // FLIP 动画：displayedCategories 顺序一变（drag 时 hover 切换 / drop 后真正 reorder），
  // 让每行从旧位置滑到新位置 —— 拖动时实时让位、drop 后落地都是同一动画机制。
  //   1. 拿渲染前每行的 top（上次 useLayoutEffect 末尾存的），对比当前 top
  //   2. delta = old - new；瞬间用 transform: translateY(delta) 把元素放回旧位置
  //   3. 双 rAF 后 transform=0 + transition → 滑回新位置
  const prevTops = useRef<Map<string, number>>(new Map());
  useLayoutEffect(() => {
    rowRefs.current.forEach((el, id) => {
      const prev = prevTops.current.get(id);
      const next = el.getBoundingClientRect().top;
      if (prev !== undefined && Math.abs(prev - next) > 0.5) {
        const dy = prev - next;
        el.style.transition = "none";
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
      }
    });
    // 更新基线，下次重排时拿来对比
    const fresh = new Map<string, number>();
    rowRefs.current.forEach((el, id) => {
      fresh.set(id, el.getBoundingClientRect().top);
    });
    prevTops.current = fresh;
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
