import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";
import {
  HIGH_RISK,
  permissionKey,
  permissionTree,
  permissionsUnder,
  setChecked,
  type PermissionNode,
} from "./pluginPermissions";
import styles from "./PluginsTab.module.css";

/** The permissions a plugin asks for, as a tree that opens and closes. Each has a box, and
 * checking follows setChecked. A row with permissions under it starts closed. When closed, it
 * shows how many under it are checked, and marks high risk if any under it is high risk. */
export function PermissionTree({
  asked,
  dependsOn,
  checked,
  onChange,
  disabled = false,
  fixed = [],
}: {
  asked: string[];
  dependsOn: Record<string, string>;
  checked: string[];
  onChange: (checked: string[]) => void;
  disabled?: boolean;
  /** What comes with the install and cannot be unchecked, such as disk space. Shown last,
   * checked and grayed out */
  fixed?: string[];
}) {
  const { t } = useTranslation();
  /** The permissions that are open */
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());

  const toggleOpen = (name: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  const row = (node: PermissionNode, depth: number) => {
    const under = permissionsUnder(node);
    const isOpen = open.has(node.name);
    const hasChildren = under.length > 0;
    const risky = [node.name, ...(isOpen ? [] : under)].some((p) => HIGH_RISK.has(p));
    return (
      <li key={node.name}>
        <div className={styles.treeRow} style={{ paddingLeft: 6 + depth * 20 }}>
          {hasChildren ? (
            <button
              type="button"
              className={styles.twistie}
              aria-expanded={isOpen}
              aria-label={
                isOpen ? t("plugins.permissionTree.collapse") : t("plugins.permissionTree.expand")
              }
              onClick={() => toggleOpen(node.name)}
            >
              <ChevronRight
                size={14}
                strokeWidth={2}
                className={`${styles.chevron} ${isOpen ? styles.chevronOpen : ""}`}
              />
            </button>
          ) : (
            <span className={styles.twistie} aria-hidden />
          )}
          <label className={styles.treeLabel}>
            <input
              type="checkbox"
              checked={checked.includes(node.name)}
              disabled={disabled}
              onChange={(e) =>
                onChange(setChecked(checked, node.name, e.target.checked, asked, dependsOn))
              }
            />
            <span>
              {t(`plugins.permissions.${permissionKey(node.name)}`, { defaultValue: node.name })}
            </span>
          </label>
          {hasChildren && !isOpen ? (
            <span className={styles.treeNote}>
              {under.filter((p) => checked.includes(p)).length}/{under.length}
            </span>
          ) : null}
          {risky ? <span className={styles.risk}>{t("plugins.install.highRisk")}</span> : null}
        </div>
        {hasChildren && isOpen ? (
          <ul className={styles.treeChildren}>{node.children.map((c) => row(c, depth + 1))}</ul>
        ) : null}
      </li>
    );
  };

  return (
    <ul className={styles.tree}>
      {permissionTree(asked, dependsOn).map((node) => row(node, 0))}
      {fixed.map((label) => (
        <li key={label}>
          <div className={styles.treeRow} style={{ paddingLeft: 6 }}>
            <span className={styles.twistie} aria-hidden />
            <label className={styles.treeLabel}>
              <input type="checkbox" checked disabled readOnly />
              <span>{label}</span>
            </label>
            <span className={styles.treeNote}>{t("plugins.permissionTree.required")}</span>
          </div>
        </li>
      ))}
    </ul>
  );
}
