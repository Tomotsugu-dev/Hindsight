import { useEffect, useLayoutEffect, useRef, useState } from "react";
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
        <SortDropdown
          value={sortBy}
          onChange={onSortChange}
          label={t("apps.filter.sortLabel")}
          options={SORT_OPTIONS.map((value) => ({
            value,
            label: t(`apps.filter.sort.${camelCase(value)}`),
          }))}
        />
      </div>
    </div>
  );
}

interface SortDropdownProps<T extends string> {
  value: T;
  onChange: (v: T) => void;
  label: string;
  options: { value: T; label: string }[];
}

/** Sorting control shared by application and website classification. */
export function SortDropdown<T extends string>({
  value,
  onChange,
  label,
  options,
}: SortDropdownProps<T>) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number; width: number } | null>(null);

  // Position the menu next to the trigger and keep it inside the viewport.
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

  // Clicking outside or pressing Escape closes the menu.
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
        <span className={styles.triggerLabel}>{label}:</span>
        <span className={styles.triggerValue}>
          {options.find((option) => option.value === value)?.label}
        </span>
        <ChevronDown size={14} strokeWidth={2} className={styles.triggerChevron} />
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            className={styles.sortMenu}
            style={
              menuPos
                ? {
                    top: menuPos.top,
                    left: menuPos.left,
                    width: menuPos.width,
                  }
                : { visibility: "hidden" }
            }
          >
            {options.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`${styles.sortItem} ${
                  option.value === value ? styles.sortItemActive : ""
                }`}
                onClick={() => {
                  onChange(option.value);
                  setOpen(false);
                }}
              >
                {option.label}
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
