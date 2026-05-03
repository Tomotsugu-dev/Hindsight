import { Cloud, Pause } from "lucide-react";
import styles from "./StatusFooter.module.css";

type CaptureStatus = "ok" | "idle" | "error";

interface StatusFooterProps {
  captureStatus?: CaptureStatus;
  todayCount?: number;
  syncLabel?: string;
  onToggleCapture?: () => void;
}

const CAPTURE_TEXT: Record<CaptureStatus, string> = {
  ok: "采集中",
  idle: "已暂停",
  error: "采集异常",
};

export function StatusFooter({
  captureStatus = "ok",
  todayCount = 0,
  syncLabel = "未登录",
  onToggleCapture,
}: StatusFooterProps) {
  return (
    <div className={styles.footer}>
      <button
        className={`${styles.row} ${styles.captureRow}`}
        type="button"
        onClick={onToggleCapture}
        aria-label="点击切换采集状态"
      >
        <span className={styles.swap} aria-hidden>
          {/* 默认态 */}
          <span className={`${styles.face} ${styles.faceDefault}`}>
            {captureStatus === "idle" ? (
              <Pause
                size={12}
                strokeWidth={2.25}
                fill="currentColor"
                className={styles.idleIcon}
                aria-hidden
              />
            ) : (
              <span
                className={`${styles.dot} ${styles[`dot_${captureStatus}`]}`}
                aria-hidden
              />
            )}
            <span className={styles.text}>
              {CAPTURE_TEXT[captureStatus]}
              <span className={styles.divider}> · </span>
              今日 {todayCount}
            </span>
          </span>

          {/* hover 态 */}
          <span className={`${styles.face} ${styles.faceHover}`}>
            <Pause size={12} strokeWidth={2} className={styles.pauseIcon} />
            <span className={styles.text}>停止采集</span>
          </span>
        </span>
      </button>

      <button className={styles.row} type="button">
        <Cloud size={14} strokeWidth={1.75} className={styles.cloud} />
        <span className={styles.text}>{syncLabel}</span>
      </button>
    </div>
  );
}
