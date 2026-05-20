import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import type { Category } from "../../api/hindsight";
import { displayCategoryName } from "../../utils/categoryName";
import styles from "./AppsFilterBar.module.css";

interface Props {
  categories: Category[];
  selectedCategoryIds: string[];
  unassignedOnly: boolean;
  onToggleCategory: (id: string) => void;
  onToggleUnassigned: () => void;
  onReset: () => void;
}

/**
 * 分类筛选 dropdown：trigger button + popover panel。
 *
 * trigger 文案逻辑：
 *   - 默认（无任何筛选条件）→ "全部"
 *   - 仅 unassignedOnly → "未分类"
 *   - selectedCategoryIds 非空 → 数字 badge "{N}"
 *
 * popover 内 chips：
 *   - 「全部」「未分类」是 pseudo-chip，灰色，跟下面按 category.color 染色的真分类 chip 区分
 *   - 真分类 chip 点击 toggle，hook 内部会清掉 unassignedOnly（互斥）
 *   - 「未分类」点击会进入排他单选模式（清空 selectedCategoryIds）
 *
 * 视觉路数参考 [`AssignDropdown`](../Categories/parts.tsx) + [`AppearancePicker`](../../components/AppearancePicker/AppearancePicker.tsx)：
 *   - portal 挂 body
 *   - 外击 / Esc 关
 *   - 触底自动 flip 到 trigger 上方
 */
export function CategoryFilterDropdown({
  categories,
  selectedCategoryIds,
  unassignedOnly,
  onToggleCategory,
  onToggleUnassigned,
  onReset,
}: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [panelPos, setPanelPos] = useState<{ top: number; left: number } | null>(
    null,
  );

  // 触发器文案 + 激活态指示
  const isActive = selectedCategoryIds.length > 0 || unassignedOnly;
  let triggerLabel: string;
  if (unassignedOnly) {
    triggerLabel = t("apps.filter.categoryUnassigned");
  } else if (selectedCategoryIds.length === 0) {
    triggerLabel = t("apps.filter.categoryAll");
  } else {
    triggerLabel = String(selectedCategoryIds.length);
  }

  // 定位：测量 trigger 矩形 + 估算 panel 高度，触底翻到上方
  useLayoutEffect(() => {
    if (!open || !triggerRef.current) return;
    const tr = triggerRef.current.getBoundingClientRect();
    const panelH = panelRef.current?.offsetHeight ?? 200; // 没渲染前用估值兜底
    const margin = 8;
    let top = tr.bottom + 6;
    if (top + panelH + margin > window.innerHeight) {
      top = tr.top - panelH - 6;
    }
    let left = tr.left;
    // 右边溢出兜底（panel 宽 ~280，trigger 左对齐）
    const panelW = panelRef.current?.offsetWidth ?? 280;
    if (left + panelW + margin > window.innerWidth) {
      left = window.innerWidth - panelW - margin;
    }
    setPanelPos({ top, left });
  }, [open]);

  // 外击 + Esc 关闭
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (panelRef.current?.contains(target)) return;
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

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${isActive ? styles.triggerActive : ""}`}
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="true"
        title={t("apps.filter.categoryTrigger")}
      >
        <span className={styles.triggerLabel}>{t("apps.filter.categoryTrigger")}:</span>
        <span className={styles.triggerValue}>{triggerLabel}</span>
        <ChevronDown size={14} strokeWidth={2} className={styles.triggerChevron} />
      </button>

      {open &&
        createPortal(
          <div
            ref={panelRef}
            className={styles.panel}
            style={
              panelPos
                ? ({ top: panelPos.top, left: panelPos.left } as CSSProperties)
                : { visibility: "hidden" } // 第一帧未测量好之前先藏，避免闪
            }
          >
            <div className={styles.panelRow}>
              <button
                type="button"
                className={`${styles.pseudoChip} ${
                  !isActive ? styles.pseudoChipActive : ""
                }`}
                onClick={onReset}
                aria-label={t("apps.filter.categoryAllAria")}
              >
                {t("apps.filter.categoryAll")}
              </button>
              <button
                type="button"
                className={`${styles.pseudoChip} ${
                  unassignedOnly ? styles.pseudoChipActive : ""
                }`}
                onClick={onToggleUnassigned}
              >
                {t("apps.filter.categoryUnassigned")}
              </button>
            </div>

            <div className={styles.panelDivider} aria-hidden />

            <div className={styles.panelRow}>
              {categories.map((c) => {
                const selected = selectedCategoryIds.includes(c.id);
                return (
                  <button
                    key={c.id}
                    type="button"
                    className={`${styles.catChip} ${
                      selected ? styles.catChipSelected : ""
                    }`}
                    style={
                      {
                        // 激活态用分类色填充，非激活态仅作为边框微提示
                        "--chip-color": c.color,
                      } as CSSProperties
                    }
                    onClick={() => onToggleCategory(c.id)}
                  >
                    {displayCategoryName(c, t)}
                  </button>
                );
              })}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
