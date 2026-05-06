import { useEffect, useRef, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Plus, X } from "lucide-react";
import { useCategories } from "../../state/categories";
import type { Category } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { displayAppName } from "../../utils/displayName";
import { displayCategoryName } from "../../utils/categoryName";
import styles from "./Categories.module.css";

export function AppList({ category }: { category: Category }) {
  const { t } = useTranslation();
  const { unassignApp, assignApp } = useCategories();
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const commit = async () => {
    const trimmed = draft.trim();
    if (trimmed) {
      await assignApp(trimmed, category.id);
    }
    setDraft("");
    setAdding(false);
  };

  const cancel = () => {
    setDraft("");
    setAdding(false);
  };

  return (
    <div className={styles.appList}>
      {category.apps.length === 0 && !adding && (
        <span className={styles.empty} style={{ padding: 0 }}>
          {t("categories.appList.empty")}
        </span>
      )}
      {category.apps.map((app) => {
        const display = displayAppName(app);
        return (
          <span key={app} className={styles.appChip}>
            <AppIcon
              processName={app}
              fallbackColor={category.color}
              size={14}
            />
            {display}
            <button
              type="button"
              className={styles.appChipRemove}
              onClick={() => unassignApp(app)}
              aria-label={t("categories.appList.removeAria", { name: display })}
              title={t("categories.appList.removeTooltip")}
            >
              <X size={10} strokeWidth={2.25} />
            </button>
          </span>
        );
      })}
      {adding ? (
        <input
          ref={inputRef}
          className={styles.appAddInput}
          placeholder={t("categories.appList.addPlaceholder")}
          value={draft}
          maxLength={64}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") cancel();
          }}
        />
      ) : (
        <button
          type="button"
          className={styles.appAddBtn}
          onClick={() => setAdding(true)}
        >
          <Plus size={11} strokeWidth={2} />
          {t("categories.appList.add")}
        </button>
      )}
    </div>
  );
}

interface MenuRect {
  top: number;
  left: number;
  width: number;
}

export function AssignDropdown({
  categories,
  currentCategoryId,
  onPick,
  allowClear = false,
}: {
  categories: Category[];
  /** 当前已选中的 category id；用于在 trigger 上显示当前分类名 + 颜色 */
  currentCategoryId?: string | null;
  /** allowClear=true 时给 null 走「取消分类」语义 */
  onPick: (categoryId: string | null) => void | Promise<void>;
  /** 是否在下拉里加一行「取消分类」（仅在已分类时有意义）*/
  allowClear?: boolean;
}) {
  const { t } = useTranslation();
  const current =
    currentCategoryId != null
      ? categories.find((c) => c.id === currentCategoryId) ?? null
      : null;
  const [open, setOpen] = useState(false);
  const [menuRect, setMenuRect] = useState<MenuRect | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (btnRef.current?.contains(target)) return;
      if (menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onScroll = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open]);

  const handleToggle = () => {
    if (open) {
      setOpen(false);
      return;
    }
    if (!btnRef.current) return;
    const rect = btnRef.current.getBoundingClientRect();
    const estimatedMenuHeight = categories.length * 28 + 8 + 4;
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    const flipUp = spaceBelow < estimatedMenuHeight && spaceAbove > spaceBelow;

    setMenuRect({
      top: flipUp ? rect.top - estimatedMenuHeight - 4 : rect.bottom + 4,
      left: rect.left,
      width: rect.width,
    });
    setOpen(true);
  };

  // trigger 按 current 是否存在切换显示形态：
  //   未分类 → "+ 指派"
  //   已分类 → "● <分类名>"（带分类色作为左侧色点）
  const triggerStyle = current
    ? ({ "--cat-color": current.color } as CSSProperties)
    : undefined;

  return (
    <div className={styles.assignWrap}>
      <button
        ref={btnRef}
        type="button"
        className={`${styles.assignBtn} ${open ? styles.assignBtnOpen : ""} ${
          current ? styles.assignBtnPicked : ""
        }`}
        style={triggerStyle}
        onClick={handleToggle}
      >
        {current ? (
          <>
            <span className={styles.assignOptionDot} aria-hidden />
            <span className={styles.assignBtnLabel}>{displayCategoryName(current, t)}</span>
          </>
        ) : (
          <>
            <Plus size={11} strokeWidth={2.25} />
            {t("categories.assign.label")}
          </>
        )}
        <ChevronDown size={11} strokeWidth={2.25} className={styles.chev} />
      </button>
      {open && menuRect && (
        <div
          ref={menuRef}
          className={styles.assignMenu}
          role="menu"
          style={{
            top: menuRect.top,
            left: menuRect.left,
            minWidth: menuRect.width,
          }}
        >
          {allowClear && current && (
            <button
              type="button"
              role="menuitem"
              className={`${styles.assignOption} ${styles.assignOptionClear}`}
              onClick={() => {
                setOpen(false);
                void onPick(null);
              }}
            >
              <span className={styles.assignOptionLabel}>
                {t("categories.assign.clear")}
              </span>
            </button>
          )}
          {categories.map((c) => {
            const style = { "--cat-color": c.color } as CSSProperties;
            const isCurrent = current?.id === c.id;
            return (
              <button
                key={c.id}
                type="button"
                role="menuitem"
                className={`${styles.assignOption} ${
                  isCurrent ? styles.assignOptionCurrent : ""
                }`}
                style={style}
                onClick={() => {
                  setOpen(false);
                  void onPick(c.id);
                }}
              >
                <span className={styles.assignOptionDot} aria-hidden />
                <span className={styles.assignOptionLabel}>{displayCategoryName(c, t)}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

