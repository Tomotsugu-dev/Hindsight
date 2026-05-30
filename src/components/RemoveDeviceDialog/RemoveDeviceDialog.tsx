import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { useTranslation } from "react-i18next";
import styles from "./RemoveDeviceDialog.module.css";

interface Props {
  open: boolean;
  /** 用户点确认后回调，参数告诉调用方走哪条路径：
   *  - `true`：保留本机数据（换账号迁移场景）
   *  - `false`：本机也对称清空（默认/推荐，离职/卖机器场景） */
  onConfirm: (keepLocal: boolean) => void;
  onCancel: () => void;
}

/**
 * 「从云端移除本设备」确认弹窗。三层防误操作：
 *   1. radio 让用户**显式选**本机数据怎么办（默认勾"也一并清空"，跟旧行为兼容）
 *   2. 必须输入 keyword（i18n 提供，中文「移除」/ 英文 "REMOVE" / 日文「削除」）才能点确认
 *   3. 确认按钮 disabled 直到 keyword 完全匹配
 */
export function RemoveDeviceDialog({ open, onConfirm, onCancel }: Props) {
  const { t } = useTranslation();
  const [keepLocal, setKeepLocal] = useState(false);
  const [typed, setTyped] = useState("");
  const keyword = t("settings.data.removeDeviceDialog.typeKeyword");
  const canConfirm = typed.trim() === keyword;

  // 每次打开重置状态，避免上次的输入 / 选择残留
  useEffect(() => {
    if (open) {
      setKeepLocal(false);
      setTyped("");
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
      if (e.key === "Enter" && canConfirm) onConfirm(keepLocal);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, canConfirm, keepLocal, onCancel, onConfirm]);

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
        aria-labelledby="remove-device-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="remove-device-title" className={styles.title}>
          {t("settings.data.removeDeviceDialog.title")}
        </h2>

        <p className={styles.intro}>
          {t("settings.data.removeDeviceDialog.intro")}
        </p>

        <p className={styles.unaffected}>
          {t("settings.data.removeDeviceDialog.unaffected")}
        </p>

        <hr className={styles.divider} />

        <p className={styles.question}>
          {t("settings.data.removeDeviceDialog.localQuestion")}
        </p>

        <div className={styles.options}>
          <label
            className={`${styles.option} ${!keepLocal ? styles.optionChecked : ""}`}
          >
            <input
              type="radio"
              className={styles.optionRadio}
              checked={!keepLocal}
              onChange={() => setKeepLocal(false)}
              aria-label={t("settings.data.removeDeviceDialog.alsoClearLabel")}
            />
            <div className={styles.optionBody}>
              <div className={styles.optionLabel}>
                {t("settings.data.removeDeviceDialog.alsoClearLabel")}
              </div>
              <div className={styles.optionHint}>
                {t("settings.data.removeDeviceDialog.alsoClearHint")}
              </div>
            </div>
          </label>

          <label
            className={`${styles.option} ${keepLocal ? styles.optionChecked : ""}`}
          >
            <input
              type="radio"
              className={styles.optionRadio}
              checked={keepLocal}
              onChange={() => setKeepLocal(true)}
              aria-label={t("settings.data.removeDeviceDialog.keepLabel")}
            />
            <div className={styles.optionBody}>
              <div className={styles.optionLabel}>
                {t("settings.data.removeDeviceDialog.keepLabel")}
              </div>
              <div className={styles.optionHint}>
                {t("settings.data.removeDeviceDialog.keepHint")}
              </div>
            </div>
          </label>
        </div>

        <hr className={styles.divider} />

        <p className={styles.confirmPrompt}>
          {t("settings.data.removeDeviceDialog.typeToConfirmPrompt", {
            keyword,
          })}
        </p>
        <input
          type="text"
          className={styles.confirmInput}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={t("settings.data.removeDeviceDialog.typePlaceholder")}
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
        />

        <div className={styles.actions}>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnCancel}`}
            onClick={onCancel}
          >
            {t("settings.data.removeDeviceDialog.cancel")}
          </button>
          <button
            type="button"
            className={`${styles.btn} ${styles.btnDanger}`}
            onClick={() => onConfirm(keepLocal)}
            disabled={!canConfirm}
          >
            {t("settings.data.removeDeviceDialog.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
