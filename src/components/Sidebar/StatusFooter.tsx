import { useEffect, useState } from "react";
import { Cloud, CloudOff, Pause } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { ROUTES } from "../../config/nav";
import { api, type AuthState } from "../../api/hindsight";
import styles from "./StatusFooter.module.css";

type CaptureStatus = "ok" | "idle" | "error";

interface StatusFooterProps {
  captureStatus?: CaptureStatus;
  todayCount?: number;
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
  onToggleCapture,
}: StatusFooterProps) {
  const navigate = useNavigate();
  const [auth, setAuth] = useState<AuthState | null>(null);

  useEffect(() => {
    const fetch = () => {
      api
        .authStatus()
        .then(setAuth)
        .catch(() => {});
    };
    fetch();
    // 周期性刷新；窗口重新聚焦时也立刻刷一次（登录回到 app 后能秒变色）
    const interval = window.setInterval(fetch, 60_000);
    const onFocus = () => fetch();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  const signedIn = auth?.signedIn ?? false;
  const syncLabel = signedIn ? auth?.email ?? "已连接" : "未登录";

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

      <button
        className={styles.row}
        type="button"
        onClick={() => navigate(ROUTES.devices)}
        aria-label="管理设备与云同步"
        title="设备 / 云同步"
      >
        {signedIn ? (
          <Cloud
            size={14}
            strokeWidth={1.75}
            className={`${styles.cloud} ${styles.cloudOn}`}
          />
        ) : (
          <CloudOff
            size={14}
            strokeWidth={1.75}
            className={styles.cloud}
          />
        )}
        <span className={`${styles.text} ${signedIn ? styles.textOn : ""}`}>
          {syncLabel}
        </span>
      </button>
    </div>
  );
}
