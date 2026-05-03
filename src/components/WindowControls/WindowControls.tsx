import { Minus, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import styles from "./WindowControls.module.css";

const appWindow = getCurrentWindow();

export function WindowControls() {
  return (
    <div className={styles.controls}>
      <button
        className={styles.btn}
        onClick={() => appWindow.minimize()}
        aria-label="最小化"
        title="最小化"
      >
        <Minus size={12} strokeWidth={2} />
      </button>
      <button
        className={`${styles.btn} ${styles.close}`}
        onClick={() => appWindow.close()}
        aria-label="关闭"
        title="关闭"
      >
        <X size={12} strokeWidth={2} />
      </button>
    </div>
  );
}
