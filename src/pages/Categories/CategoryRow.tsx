import {
  memo,
  useCallback,
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
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { IconButton } from "../../components/Button/IconButton";
import type { Category } from "../../api/hindsight";
import { EXCLUDED_CATEGORY_ID, PRIVATE_CATEGORY_ID } from "../../utils/privateCategory";
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
  /** 刚被用户新建出来的那一行：挂着陆动画 + accent 描边，并直接进改名态。
   *  没有这个反馈的话，新建按钮在视口外建出一行，用户看不到任何变化（issue #19）。 */
  justCreated?: boolean;
  /** 刚完成拖拽落地：只做短暂描边，不进入改名态。 */
  isLanded?: boolean;
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
 * builtin（hidden、private）分类不可拖。
 *
 * 老版 FLIP 行内拖拽重排序已**移除**（v1 不支持同大类内排序）；grip 现在仅作
 * 视觉提示"此行可拖"，不绑定 mousedown handler——HTML5 DnD 自动覆盖整个行的
 * 拖拽语义。
 */
export const CategoryRow = memo(function CategoryRow({
  category,
  onGripMouseDown,
  isDragging,
  justCreated,
  isLanded,
  registerRowRef,
}: Props) {
  const { t } = useTranslation();
  const { update, remove } = useCategories();
  // 新建行一挂载就进改名态：默认名是「新分类」，直接打字即可覆盖。
  // 用 useState 的惰性初值而不是 effect —— 新分类是全新 mount（key = 新 id），
  // 初值就够；后续 justCreated 转 false 也不该把用户正在编辑的输入框收掉。
  const [editingName, setEditingName] = useState(() => justCreated === true);
  const [draftName, setDraftName] = useState(category.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  const Icon = resolveCategoryIcon(category.icon);

  // 用 callback ref 在 layout effect 之前完成注册，确保首次 reorder 的 FLIP
  // 已经拿到旧位置。
  const setRowRef = useCallback(
    (el: HTMLDivElement | null) => {
      rowRef.current = el;
      registerRowRef?.(category.id, el);
    },
    [category.id, registerRowRef],
  );

  useEffect(() => {
    if (editingName) {
      // preventScroll：新建行的 focus 发生在父组件平滑滚动之前（子 effect 先跑），
      // 不拦的话浏览器会先瞬移到该行，把随后的 smooth scroll 变成一次生硬跳变
      inputRef.current?.focus({ preventScroll: true });
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
  // 两个内置分类的名字说不清它们做什么，名字后面跟一句说明
  const description =
    category.id === PRIVATE_CATEGORY_ID
      ? t("categories.private.description")
      : category.id === EXCLUDED_CATEGORY_ID
        ? t("categories.excluded.description")
        : null;

  return (
    <div
      ref={setRowRef}
      className={[
        styles.catRow,
        isDragging ? styles.catRowSource : "",
        justCreated ? styles.catRowJustCreated : "",
        isLanded ? styles.catRowLanded : "",
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
              void update(category.id, { icon: i });
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
                if (e.key === "Enter") void commitName();
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
              {category.name}
            </span>
          )}
          {description && <span className={styles.catDesc}>{description}</span>}
        </div>

        <div className={styles.appListClip}>
          <AppList category={category} />
        </div>
      </div>

      <div className={styles.catActions}>
        <IconButton
          icon={Pencil}
          label={t("categories.list.renameAria")}
          variant="soft"
          tone="accent"
          quiet
          onClick={() => setEditingName(true)}
          onMouseDown={(e) => e.stopPropagation()}
          draggable={false}
        />
        {/* 内置分类（如 v27 的 hidden）与 'other'（未分类时长的隐式归属，reports SQL
            COALESCE 到它）不允许删除：按钮不渲染，dialog 也不挂。
            后端 categories::delete 有二次防御 */}
        {!category.builtin && category.id !== "other" && (
          <IconButton
            icon={Trash2}
            label={t("categories.list.deleteAria")}
            variant="soft"
            tone="danger"
            quiet
            onClick={() => setConfirmOpen(true)}
            onMouseDown={(e) => e.stopPropagation()}
            draggable={false}
          />
        )}
      </div>

      {!category.builtin && category.id !== "other" && (
        <ConfirmDialog
          open={confirmOpen}
          title={t("categories.deleteDialog.title")}
          message={t("categories.deleteDialog.message", {
            name: category.name,
          })}
          confirmLabel={t("categories.deleteDialog.confirm")}
          variant="danger"
          onConfirm={onConfirmDelete}
          onCancel={() => setConfirmOpen(false)}
        />
      )}
    </div>
  );
});
