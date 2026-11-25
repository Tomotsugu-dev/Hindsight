import { MoreHorizontal, type LucideIcon } from "lucide-react";
import { IconButton } from "../Button/IconButton";
import { usePicker } from "../../hooks/usePicker";
import styles from "./RowMenu.module.css";

export interface RowMenuItem {
  label: string;
  icon: LucideIcon;
  onClick: () => void;
  /** Red, below a line. For actions that delete something. */
  danger?: boolean;
}

interface RowMenuProps {
  /** What the menu is for, such as "More actions for Qwen3 8B". Screen readers read it. */
  label: string;
  items: RowMenuItem[];
}

/**
 * Show the actions a row uses less often behind a "⋯" button, such as Open folder and Delete.
 *
 * The menu opens below the button, aligned to its right edge. Items that delete something go last,
 * below a line. The menu carries `data-part="menu"` for themes.
 */
export function RowMenu({ label, items }: RowMenuProps) {
  const { open, wrapRef, toggle, close } = usePicker();
  const safe = items.filter((i) => !i.danger);
  const danger = items.filter((i) => i.danger);

  const render = (item: RowMenuItem) => {
    const Icon = item.icon;
    return (
      <button
        key={item.label}
        type="button"
        role="menuitem"
        className={`${styles.item} ${item.danger ? styles.danger : ""}`}
        onClick={() => {
          close();
          item.onClick();
        }}
      >
        <Icon className={styles.icon} aria-hidden />
        <span className={styles.itemLabel}>{item.label}</span>
      </button>
    );
  };

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <IconButton
        icon={MoreHorizontal}
        label={label}
        size="sm"
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
      />
      {open ? (
        <div className={styles.menu} role="menu" aria-label={label} data-part="menu">
          {safe.map(render)}
          {safe.length > 0 && danger.length > 0 ? <hr className={styles.divider} /> : null}
          {danger.map(render)}
        </div>
      ) : null}
    </div>
  );
}
