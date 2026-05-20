import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronDown, Search } from "lucide-react";
import type { Category } from "../../api/hindsight";
import { CategoryFilterDropdown } from "./CategoryFilterDropdown";
import type { AppsSortBy } from "./useAppsFilter";
import styles from "./AppsFilterBar.module.css";

const SORT_OPTIONS: AppsSortBy[] = [
  "default",
  "duration_desc",
  "duration_asc",
  "name_asc",
  "name_desc",
];

interface Props {
  search: string;
  onSearchChange: (v: string) => void;
  categories: Category[];
  selectedCategoryIds: string[];
  unassignedOnly: boolean;
  onToggleCategory: (id: string) => void;
  onToggleUnassigned: () => void;
  onResetCategories: () => void;
  sortBy: AppsSortBy;
  onSortChange: (v: AppsSortBy) => void;
}

/** `/apps` 页头部的单行工具栏：搜索 + 分类 dropdown + 排序 dropdown。 */
export function AppsFilterBar({
  search,
  onSearchChange,
  categories,
  selectedCategoryIds,
  unassignedOnly,
  onToggleCategory,
  onToggleUnassigned,
  onResetCategories,
  sortBy,
  onSortChange,
}: Props) {
  const { t } = useTranslation();

  return (
    <div className={styles.bar}>
      <div className={styles.searchWrap}>
        <Search size={14} strokeWidth={2} className={styles.searchIcon} aria-hidden />
        <input
          className={styles.searchInput}
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={t("apps.filter.searchPlaceholder")}
          spellCheck={false}
        />
      </div>

      <div className={styles.rightGroup}>
        <CategoryFilterDropdown
          categories={categories}
          selectedCategoryIds={selectedCategoryIds}
          unassignedOnly={unassignedOnly}
          onToggleCategory={onToggleCategory}
          onToggleUnassigned={onToggleUnassigned}
          onReset={onResetCategories}
        />
        <SortDropdown value={sortBy} onChange={onSortChange} />
      </div>
    </div>
  );
}

interface SortDropdownProps {
  value: AppsSortBy;
  onChange: (v: AppsSortBy) => void;
}

function SortDropdown({ value, onChange }: SortDropdownProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number; width: number } | null>(
    null,
  );

  const labelOf = (v: AppsSortBy): string => t(`apps.filter.sort.${camelCase(v)}`);

  // 定位 + 把菜单宽度对齐 trigger 自身宽度
  useLayoutEffect(() => {
    if (!open || !triggerRef.current) return;
    const tr = triggerRef.current.getBoundingClientRect();
    const menuH = menuRef.current?.offsetHeight ?? 180;
    const margin = 8;
    let top = tr.bottom + 6;
    if (top + menuH + margin > window.innerHeight) {
      top = tr.top - menuH - 6;
    }
    let left = tr.left;
    const menuW = tr.width;
    if (left + menuW + margin > window.innerWidth) {
      left = window.innerWidth - menuW - margin;
    }
    setMenuPos({ top, left, width: menuW });
  }, [open]);

  // 外击 + Esc
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const isDefault = value === "default";

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${!isDefault ? styles.triggerActive : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="true"
      >
        <span className={styles.triggerLabel}>{t("apps.filter.sortLabel")}:</span>
        <span className={styles.triggerValue}>{labelOf(value)}</span>
        <ChevronDown size={14} strokeWidth={2} className={styles.triggerChevron} />
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            className={styles.sortMenu}
            style={
              menuPos
                ? ({
                    top: menuPos.top,
                    left: menuPos.left,
                    width: menuPos.width,
                  } as CSSProperties)
                : { visibility: "hidden" }
            }
            role="menu"
          >
            {SORT_OPTIONS.map((opt) => (
              <button
                key={opt}
                type="button"
                className={`${styles.sortItem} ${
                  opt === value ? styles.sortItemActive : ""
                }`}
                onClick={() => {
                  onChange(opt);
                  setOpen(false);
                }}
                role="menuitem"
              >
                {labelOf(opt)}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}

/** "duration_desc" → "durationDesc" — 把 snake_case 映射到 i18n key 里的 camelCase。 */
function camelCase(s: string): string {
  return s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
}
