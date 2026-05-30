import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { Folder, X } from "lucide-react";
import { useSuperCategories } from "../../state/superCategories";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { displaySuperCategoryName } from "../../utils/categoryName";
import type { Category, SuperCategory } from "../../api/hindsight";
import { CategoryRow } from "./CategoryRow";
import styles from "./SuperCategoriesTable.module.css";

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
      {/* 卡片 mousedown 仅用于鼠标拖拽排序；拖拽暂无键盘等价操作（已知 a11y 缺口）。
          卡内 icon/改名等都是独立 button，故此处不把整卡当按钮。 */}
      {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions */}
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
                void update(sup.id, { icon: i });
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
                if (e.key === "Enter") void commitName();
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
export const SuperRow = ({
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

export const OrphanRow = ({
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
export function BuiltinRow({
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
