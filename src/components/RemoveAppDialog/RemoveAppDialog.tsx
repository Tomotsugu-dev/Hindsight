import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { EyeOff, Trash2 } from "lucide-react";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import styles from "./RemoveAppDialog.module.css";

interface RemoveAppDialogProps {
  open: boolean;
  /** 要移除的应用显示名，写进标题 */
  appName: string;
  onHide: () => void;
  onDelete: () => void;
  onCancel: () => void;
}

/**
 * 从应用列表移除一个应用时的选择弹窗。
 *
 * 为什么不是简单的「确认删除」：这里的两条路后果完全不同，用户不知情就会选错——
 *   - 隐藏：归入内建的 `hidden` 分类。不再计入任何统计与 AI 总结，但历史数据
 *     一条不少，随时能从分类页拖回来；
 *   - 删除：真删——活动记录、截图文件、OCR 文字索引一并清掉。**不可逆**，而且
 *     这个程序下次运行时还会重新出现在列表里（除非勾上「不再记录」）。
 * 所以默认推荐隐藏，删除放次要位置，并把差异写在正文里。
 *
 * **刻意不做「不再提示」**：记住「删除」意味着以后每次点行尾按钮都直接不可逆
 * 地删数据，省下的一次确认远不值这个风险。每次都问。
 */
export function RemoveAppDialog({
  open,
  appName,
  onHide,
  onDelete,
  onCancel,
}: RemoveAppDialogProps) {
  const { t } = useTranslation();

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

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
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
            <EyeOff size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.hideName")}</p>
              <p className={styles.optionDesc}>{t("apps.remove.hideDesc")}</p>
            </div>
          </li>
          <li className={styles.option}>
            <Trash2 size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.deleteName")}</p>
              <p className={styles.optionDesc}>{t("apps.remove.deleteDesc")}</p>
            </div>
          </li>
        </ul>

        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnCancel}`}
            onClick={onCancel}
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
            className={`${styles.btn} ${styles.btnConfirm}`}
            onClick={onHide}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("apps.remove.hideAction")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
