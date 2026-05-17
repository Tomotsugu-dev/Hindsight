import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
// 复用 RemoveDeviceDialog 同款样式（backdrop / dialog / btn / input），保持 modal 视觉一致。
import styles from "../RemoveDeviceDialog/RemoveDeviceDialog.module.css";

interface Props {
  open: boolean;
  /** 被移除的远端设备显示名，让用户看清楚到底删哪台 */
  deviceName: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 「从云端移除远端设备」确认弹窗。
 *
 * 跟 [`RemoveDeviceDialog`]（移除本设备）的差异：
 * - 没有 "keep local" radio：远端设备的本地数据就是云端同步过来的，移除时一并清掉，
 *   没有"留本机"语义
 * - 必须输入 keyword 才能确认（防误删）
 * - 显示被移除的设备名让用户最后确认对象
 */
export function ForgetRemoteDeviceDialog({ open, deviceName, onConfirm, onCancel }: Props) {
  const { t } = useTranslation();
  const [typed, setTyped] = useState("");
  const keyword = t("settings.data.removeDeviceDialog.typeKeyword");
  const canConfirm = typed.trim() === keyword;

  // 每次打开重置 typed
  useEffect(() => {
    if (open) setTyped("");
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
      if (e.key === "Enter" && canConfirm) onConfirm();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, canConfirm, onCancel, onConfirm]);

  if (!open) return null;

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        className={styles.dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="forget-remote-device-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="forget-remote-device-title" className={styles.title}>
          {t("devices.forgetDialog.title", { name: deviceName })}
        </h2>

        <p className={styles.intro}>{t("devices.forgetDialog.intro")}</p>

        <p className={styles.unaffected}>{t("devices.forgetDialog.unaffected")}</p>

        <p className={styles.confirmPrompt}>
          {t("settings.data.removeDeviceDialog.typeToConfirmPrompt", { keyword })}
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
            onClick={onConfirm}
            disabled={!canConfirm}
          >
            {t("devices.forgetDialog.confirm")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
