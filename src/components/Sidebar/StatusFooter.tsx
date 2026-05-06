import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRightLeft, Coffee, Globe, Pause } from "lucide-react";
import { useSettings } from "../../state/settings";
import { useLocale, type Locale } from "../../i18n/useLocale";
import styles from "./StatusFooter.module.css";

type CaptureStatus = "ok" | "idle" | "error";

interface StatusFooterProps {
  captureStatus?: CaptureStatus;
  onToggleCapture?: () => void;
}

// 采集状态文案 -> i18n key 映射
const CAPTURE_TEXT_KEY: Record<CaptureStatus, string> = {
  ok: "sidebar.capture.ok",
  idle: "sidebar.capture.idle",
  error: "sidebar.capture.error",
};

// 三个语言 option 的元信息（label 用各自语言的母语写法，避免再走 t()）
// 顺序也是循环切换的顺序：点击 trigger → 跳到 next（zh-CN → en → ja → zh-CN）
const LOCALE_OPTIONS: { value: Locale; label: string }[] = [
  { value: "zh-CN", label: "简体中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
];

function parseHM(s: string): number {
  const [h, m] = s.split(":").map((p) => parseInt(p, 10));
  if (Number.isNaN(h)) return 0;
  return h + (Number.isNaN(m) ? 0 : m / 60);
}

export function StatusFooter({
  captureStatus = "ok",
  onToggleCapture,
}: StatusFooterProps) {
  const { settings } = useSettings();
  const { t } = useTranslation();
  const [tick, setTick] = useState(0);
  const [locale, setLocale] = useLocale();

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

  // 当前与下一种语言（cycle 顺序按 LOCALE_OPTIONS 数组）
  const currentIdx = Math.max(
    0,
    LOCALE_OPTIONS.findIndex((o) => o.value === locale),
  );
  const currentOption = LOCALE_OPTIONS[currentIdx];
  const nextOption =
    LOCALE_OPTIONS[(currentIdx + 1) % LOCALE_OPTIONS.length];

  return (
    <div className={styles.footer}>
      <button
        className={`${styles.row} ${styles.captureRow} ${
          !inWorkHours ? styles.captureRowResting : ""
        }`}
        type="button"
        onClick={onToggleCapture}
        aria-label={t("sidebar.capture.toggleAria")}
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
            <span className={styles.text}>{t(CAPTURE_TEXT_KEY[captureStatus])}</span>
          </span>

          {/* 工作时间外：休息态 */}
          <span className={`${styles.face} ${styles.faceResting}`}>
            <Coffee size={12} strokeWidth={2} className={styles.restIcon} />
            <span className={styles.text}>{t("sidebar.capture.resting")}</span>
          </span>

          {/* hover 态 */}
          <span className={`${styles.face} ${styles.faceHover}`}>
            <Pause size={12} strokeWidth={2} className={styles.pauseIcon} />
            <span className={styles.text}>{t("sidebar.capture.stop")}</span>
          </span>
        </span>
      </button>

      {/* 语言切换：点击循环到下一种语言（zh-CN → en → ja → zh-CN）；
          hover 时上下 swap 显示目标语言名 */}
      <button
        className={`${styles.row} ${styles.langRow}`}
        type="button"
        onClick={() => setLocale(nextOption.value)}
        title={`Switch to ${nextOption.label}`}
      >
        <span className={styles.swap} aria-hidden>
          {/* 当前语言态 */}
          <span className={`${styles.face} ${styles.faceDefault}`}>
            <Globe size={14} strokeWidth={1.75} className={styles.cloud} />
            <span className={styles.text}>{currentOption.label}</span>
          </span>

          {/* hover 态：目标语言（cycle 中下一种） */}
          <span className={`${styles.face} ${styles.langFaceTarget}`}>
            <ArrowRightLeft size={14} strokeWidth={1.75} className={styles.cloud} />
            <span className={styles.text}>{nextOption.label}</span>
          </span>
        </span>
      </button>
    </div>
  );
}
