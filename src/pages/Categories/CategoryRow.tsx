import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { GripVertical, Pencil, Trash2 } from "lucide-react";
import { useCategories } from "../../state/categories";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { displayCategoryName } from "../../utils/categoryName";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import type { Category } from "../../api/hindsight";
import { AppList } from "./parts";
import styles from "./Categories.module.css";

interface Props {
  category: Category;
  /** SuperCategoriesTable 注入：grip mousedown 时调到父，让父开启自渲染 drag pipeline。
   *  传 row 的 DOM 引用过去方便父测量源行的 rect / 锁 X / 锁尺寸。 */
  onGripMouseDown?: (
    cat: Category,
    e: ReactMouseEvent<HTMLButtonElement>,
    rowEl: HTMLDivElement,
  ) => void;
  /** 当前是否在被拖（父知道 dragId 时传 true）。被拖期间行半透明占位。 */
  isDragging?: boolean;
  /** SuperCategoriesTable 把 catId → DOM 收集到全局 Map，用来 drag 开始时快照各 row Y 范围
   *  做"同大类内 reorder hover 命中" */
  registerRowRef?: (id: string, el: HTMLDivElement | null) => void;
}

/**
 * 分类行：保留原 ListTab.tsx 里 CategoryRow 的全部视觉与交互——grip 把手、icon
 * 点开 AppearancePicker、双击行名改名、hover 展开"绑定的应用" chip 列表、行尾的
 * 编辑✎ / 删除🗑 按钮、删除确认对话框——全部一字不动。
 *
 * v28 新增：整行 HTML5 draggable。dragstart 通知父组件 SuperCategoriesTable 谁正在
 * 被拖，父用这信号把所有大类 label 标 dropTarget；drop 到 label 上 = 归入该大类。
 * builtin（hidden）分类不可拖。
 *
 * 老版 FLIP 行内拖拽重排序已**移除**（v1 不支持同大类内排序）；grip 现在仅作
 * 视觉提示"此行可拖"，不绑定 mousedown handler——HTML5 DnD 自动覆盖整个行的
 * 拖拽语义。
 */
export function CategoryRow({
  category,
  onGripMouseDown,
  isDragging,
  registerRowRef,
}: Props) {
  const { t } = useTranslation();
  const { update, remove } = useCategories();
  const [editingName, setEditingName] = useState(false);
  const [draftName, setDraftName] = useState(category.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  const Icon = resolveCategoryIcon(category.icon);

  // 把 row DOM 注册到父（SuperCategoriesTable）的 Map，让父在 drag 开始时拍 Y 范围快照
  useEffect(() => {
    if (!registerRowRef) return;
    const el = rowRef.current;
    registerRowRef(category.id, el);
    return () => registerRowRef(category.id, null);
  }, [category.id, registerRowRef]);

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

  const handleGripMouseDown = (e: ReactMouseEvent<HTMLButtonElement>) => {
    if (category.builtin) return;
    if (!rowRef.current) return;
    onGripMouseDown?.(category, e, rowRef.current);
  };

  const styleVar = { "--cat-color": category.color } as CSSProperties;

  return (
    <div
      ref={rowRef}
      className={[
        styles.catRow,
        isDragging ? styles.catRowSource : "",
      ]
        .filter(Boolean)
        .join(" ")}
      style={styleVar}
    >
      <button
        type="button"
        className={styles.catDragHandle}
        onMouseDown={handleGripMouseDown}
        aria-label={t("categories.list.dragHandleAria")}
        title={t("categories.list.dragHandleAria")}
        // builtin 行不可拖：mousedown handler 内部 return
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
          // 防止 button mousedown 误启动 HTML5 drag
          onMouseDown={(e) => e.stopPropagation()}
          draggable={false}
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
              onMouseDown={(e) => e.stopPropagation()}
              draggable={false}
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
          onMouseDown={(e) => e.stopPropagation()}
          draggable={false}
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
            onMouseDown={(e) => e.stopPropagation()}
            draggable={false}
          >
            <Trash2 size={14} strokeWidth={1.85} />
          </button>
        )}
      </div>

      {!category.builtin && (
        <ConfirmDialog
          open={confirmOpen}
          title={t("categories.deleteDialog.title")}
          message={t("categories.deleteDialog.message", {
            name: displayCategoryName(category, t),
          })}
          confirmLabel={t("categories.deleteDialog.confirm")}
          variant="danger"
          onConfirm={onConfirmDelete}
          onCancel={() => setConfirmOpen(false)}
        />
      )}
    </div>
  );
}
