import { useTranslation } from "react-i18next";
import { ListChecks, Trash2 } from "lucide-react";
import type { Category } from "../../api/hindsight";
import { Button } from "../../components/Button/Button";
import { AssignDropdown } from "./parts";
import filterBarStyles from "../Apps/AppsFilterBar.module.css";
import styles from "./BatchBar.module.css";

/** The filter bar button that turns select mode on, and reads "Done" while it is on. */
export function SelectModeButton({ active, onClick }: { active: boolean; onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className={`${filterBarStyles.trigger} ${active ? filterBarStyles.triggerActive : ""}`}
      aria-pressed={active}
      onClick={onClick}
    >
      <ListChecks size={14} strokeWidth={2} className={filterBarStyles.triggerIcon} aria-hidden />
      <span className={filterBarStyles.triggerValue}>
        {t(active ? "categories.batch.done" : "categories.batch.start")}
      </span>
    </button>
  );
}

interface BatchBarProps {
  /** How many rows are picked */
  count: number;
  allSelected: boolean;
  onToggleAll: () => void;
  categories: Category[];
  onAssign: (categoryId: string) => void;
  onRemove: () => void;
}

/**
 * The bar at the bottom of the window in select mode. It stays in view while the list scrolls.
 * The actions show once a row is picked.
 */
export function BatchBar({
  count,
  allSelected,
  onToggleAll,
  categories,
  onAssign,
  onRemove,
}: BatchBarProps) {
  const { t } = useTranslation();
  const label = t("categories.batch.selected", { count });
  return (
    <div className={styles.dock}>
      <div className={styles.bar} role="toolbar" aria-label={label}>
        <span className={styles.count}>{label}</span>
        <Button variant="ghost" size="sm" onClick={onToggleAll}>
          {t(allSelected ? "categories.batch.clear" : "categories.batch.selectAll")}
        </Button>
        {count > 0 && (
          <>
            <span className={styles.divider} aria-hidden />
            <AssignDropdown
              categories={categories}
              currentCategoryId={null}
              onPick={(categoryId) => {
                if (categoryId !== null) onAssign(categoryId);
              }}
            />
            <Button variant="outline" tone="danger" size="sm" icon={Trash2} onClick={onRemove}>
              {t("categories.batch.remove")}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
