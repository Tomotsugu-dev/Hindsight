import { useEffect, useMemo, useState } from "react";
import { Cloud, CloudOff, Coffee, Pause } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { ROUTES } from "../../config/nav";
import { api, type AuthState } from "../../api/hindsight";
import { useSettings } from "../../state/settings";
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

function parseHM(s: string): number {
  const [h, m] = s.split(":").map((p) => parseInt(p, 10));
  if (Number.isNaN(h)) return 0;
  return h + (Number.isNaN(m) ? 0 : m / 60);
}

export function StatusFooter({
  captureStatus = "ok",
  todayCount = 0,
  onToggleCapture,
}: StatusFooterProps) {
  const navigate = useNavigate();
  const { settings } = useSettings();
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [tick, setTick] = useState(0);

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

  // 每分钟 tick 一次，让"是否在工作时间"重新评估
  useEffect(() => {
    const t = window.setInterval(() => setTick((n) => n + 1), 60_000);
    return () => window.clearInterval(t);
  }, []);

  // 当前是否在用户设定的工作时间段内（未启用 / 没有时段都视为"始终工作"）
  const inWorkHours = useMemo(() => {
    void tick; // 让 tick 触发重新计算
    if (!settings?.workHoursEnabled) return true;
    const ranges = settings.workRanges ?? [];
    if (ranges.length === 0) return true;
    const now = new Date();
    const h = now.getHours() + now.getMinutes() / 60;
    return ranges.some((r) => {
      const s = parseHM(r.start);
      const e = parseHM(r.end);
      return h >= s && h < e;
    });
  }, [settings, tick]);

  const signedIn = auth?.signedIn ?? false;
  const syncLabel = signedIn ? auth?.email ?? "已连接" : "未登录";

  return (
    <div className={styles.footer}>
      <button
        className={`${styles.row} ${styles.captureRow} ${
          !inWorkHours ? styles.captureRowResting : ""
        }`}
        type="button"
        onClick={onToggleCapture}
        aria-label="点击切换采集状态"
      >
        <span className={styles.swap} aria-hidden>
          {/* 默认态：工作时间内 —— 采集中 */}
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

          {/* 工作时间外：休息态 */}
          <span className={`${styles.face} ${styles.faceResting}`}>
            <Coffee size={12} strokeWidth={2} className={styles.restIcon} />
            <span className={styles.text}>
              休息中
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
