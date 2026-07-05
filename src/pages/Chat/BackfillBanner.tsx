import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { DatabaseZap, Loader2 } from "lucide-react";
import { api, type MemoryPendingStats } from "../../api/hindsight";
import { logError } from "../../lib/logger";
import styles from "./ChatPage.module.css";

interface BackfillBannerProps {
  stats: MemoryPendingStats;
  /** 重查 pending stats(轮询进度与收尾都靠它刷新父组件的 stats) */
  onRefresh: () => void;
}

type Phase = "idle" | "running" | "background" | "failed";

/** 索引进行中(手动触发或后台批)时的进度轮询间隔 */
const POLL_MS = 3000;

/**
 * 未入索引提示条:有 N 张截图没进文字索引时显示,一键回填。
 * 索引进行期间每 3 秒重查剩余帧数,实时显示进度;剩余归零 banner 自动消失。
 * digest 报"已在运行"(常驻 OCR 定时批持锁)按后台运行处理——帧已登记,
 * 后台批会消化,同样轮询进度。
 */
export default function BackfillBanner({ stats, onRefresh }: BackfillBannerProps) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>("idle");
  const [errMsg, setErrMsg] = useState("");

  // 索引进行中轮询剩余数;total 归零时父组件的 stats 更新会让本组件不再渲染
  const polling = phase === "running" || phase === "background";
  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(onRefresh, POLL_MS);
    return () => clearInterval(timer);
  }, [polling, onRefresh]);

  if (stats.total <= 0) return null;

  const run = async () => {
    setPhase("running");
    try {
      await api.memoryBackfill();
      await api.memoryDigestNow();
      setPhase("idle");
      onRefresh();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已在运行")) {
        // 帧已登记,后台批会消化;转入后台态继续轮询进度
        setPhase("background");
      } else {
        logError("chat.backfill", e);
        setErrMsg(msg);
        setPhase("failed");
      }
    }
  };

  return (
    <div className={styles.banner} role="status">
      <DatabaseZap size={14} strokeWidth={2} className={styles.bannerIcon} />
      <span className={styles.bannerText}>
        {phase === "running" && t("chat.backfill.running", { count: stats.total })}
        {phase === "background" &&
          t("chat.backfill.alreadyRunning", { count: stats.total })}
        {phase === "failed" && t("chat.backfill.failed", { msg: errMsg })}
        {phase === "idle" && t("chat.backfill.pending", { count: stats.total })}
      </span>
      {(phase === "idle" || phase === "failed") && (
        <button type="button" className={styles.bannerBtn} onClick={() => void run()}>
          {t("chat.backfill.action")}
        </button>
      )}
      {polling && (
        <Loader2 size={13} strokeWidth={2.25} className={styles.bannerSpin} />
      )}
    </div>
  );
}
