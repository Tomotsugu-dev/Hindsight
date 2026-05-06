import { useTranslation } from "react-i18next";
import { Minus, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import styles from "./WindowControls.module.css";

const appWindow = getCurrentWindow();

export function WindowControls() {
  const { t } = useTranslation();
  const minimize = t("windowControls.minimize");
  const close = t("windowControls.close");
  return (
    <div className={styles.controls}>
      <button
        className={styles.btn}
        onClick={() => appWindow.minimize()}
        aria-label={minimize}
        title={minimize}
      >
        <Minus size={12} strokeWidth={2} />
      </button>
      <button
        className={`${styles.btn} ${styles.close}`}
        onClick={() => appWindow.close()}
        aria-label={close}
        title={close}
      >
        <X size={12} strokeWidth={2} />
      </button>
    </div>
  );
}
