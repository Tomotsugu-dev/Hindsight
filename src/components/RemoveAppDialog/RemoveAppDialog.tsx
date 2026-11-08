import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { EyeOff, Lock, Trash2 } from "lucide-react";
import { api, type DataCount } from "../../api/hindsight";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { logError } from "../../lib/logger";
import { describeCount } from "../../utils/privateCategory";
import styles from "./RemoveAppDialog.module.css";

interface RemoveAppDialogProps {
  open: boolean;
  /** 要移除的应用显示名，写进标题 */
  appName: string;
  /** 这个应用的分组，统计会删多少用 */
  groupId: string;
  onDelete: () => void;
  onMakePrivate: () => void;
  onExclude: () => void;
  onCancel: () => void;
}

/**
 * 从应用列表移除一个应用时的选择弹窗。三条路后果不同，说明写在正文里，不推荐哪一条：
 *   - 删除：删掉已有的数据，不可逆。这个程序下次运行时还会出现在列表里；
 *   - 设为隐私：删掉已有的数据，以后也不记录。不可逆；
 *   - 不计入统计：归入内置的 `hidden` 分类，照常记录，只是不算进统计。
 * 前两条都删数据，按钮是红的；打开时焦点在「取消」上。
 *
 * **刻意不做「不再提示」**：记住「删除」意味着以后每次点行尾按钮都直接不可逆
 * 地删数据，省下的一次确认远不值这个风险。每次都问。
 */
export function RemoveAppDialog({
  open,
  appName,
  groupId,
  onDelete,
  onMakePrivate,
  onExclude,
  onCancel,
}: RemoveAppDialogProps) {
  const { t } = useTranslation();
  // null：还在统计，或者统计出错
  const [count, setCount] = useState<DataCount | null>(null);

  useEffect(() => {
    if (!open) return;
    setCount(null);
    let live = true;
    void api.countTargetData({ kind: "appGroup", groupId }).then(
      (c) => {
        if (live) setCount(c);
      },
      (e) => logError("removeApp.count", e),
    );
    return () => {
      live = false;
    };
  }, [open, groupId]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  if (!open) return null;
  const data = describeCount(t, count);

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        data-part="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="remove-app-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="remove-app-title" className={styles.title}>
          {t("apps.remove.title", { name: appName })}
        </h2>

        <ul className={styles.options}>
          <li className={styles.option}>
            <Trash2 size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.deleteName")}</p>
              <p className={styles.optionDesc}>{t("apps.remove.deleteDesc", { data })}</p>
            </div>
          </li>
          <li className={styles.option}>
            <Lock size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.privateName")}</p>
              <p className={styles.optionDesc}>{t("apps.remove.privateDesc", { data })}</p>
            </div>
          </li>
          <li className={styles.option}>
            <EyeOff size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.excludeName")}</p>
              <p className={styles.optionDesc}>{t("apps.remove.excludeDesc")}</p>
            </div>
          </li>
        </ul>

        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnCancel}`}
            onClick={onCancel}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnDanger}`}
            onClick={onDelete}
          >
            {t("apps.remove.deleteAction")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnDanger}`}
            onClick={onMakePrivate}
          >
            {t("apps.remove.privateAction")}
          </button>
          <button type="button" className={`${styles.btn} ${styles.btnCancel}`} onClick={onExclude}>
            {t("apps.remove.excludeAction")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
