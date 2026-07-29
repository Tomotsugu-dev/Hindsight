import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { DatabaseZap, Loader2 } from "lucide-react";
import {
  api,
  ENGINE_DOWNLOAD_EVENT,
  type EngineDownloadProgress,
  type MemoryPendingStats,
} from "../../api/hindsight";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { ocrRuntimeReady } from "../../lib/ocrRuntime";
import { logError } from "../../lib/logger";
import { useSettings } from "../../state/settings";
import styles from "./ChatPage.module.css";

interface BackfillBannerProps {
  stats: MemoryPendingStats;
  /** 重查 pending stats(轮询进度与收尾都靠它刷新父组件的 stats) */
  onRefresh: () => void;
}

type Phase = "idle" | "downloading" | "running" | "background" | "failed";

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
  const { settings, update } = useSettings();
  const [phase, setPhase] = useState<Phase>("idle");
  const [errMsg, setErrMsg] = useState("");
  // OCR 组件缺失时的下载确认弹窗与进度(MB)
  const [ocrConfirm, setOcrConfirm] = useState(false);
  const [dlMb, setDlMb] = useState(0);
  // 点过停止(防连点):停止是异步生效的(循环帧间感知,~1s),按住 disabled
  // 直到本轮 digest resolve 收尾
  const [stopping, setStopping] = useState(false);
  // 停止后的语义收尾:常驻 OCR 开着时,"停止"只掐当前批,下个 tick(~1 分钟)
  // 会自己继续——不明说用户会以为停止按钮坏了。批停下后显示过渡态
  // "本批已停止 · 常驻识别约 1 分钟后将继续 [关闭常驻 OCR]",
  // 常驻恢复运行(polling 重新为 true)或用户关掉常驻后自动消失。
  const [stoppedNotice, setStoppedNotice] = useState(false);
  const residentOn = settings?.memoryOcrResident ?? false;

  // 后端消化正在跑(常驻批/别处触发的手动批)时,即使本组件刚挂载
  // (比如用户切走再切回来),也直接显示"后台索引中"而不是带按钮的初始态
  const effective: Phase =
    phase === "idle" && stats.digestRunning ? "background" : phase;

  // 索引进行中轮询剩余数;total 归零时父组件的 stats 更新会让本组件不再渲染
  const polling = effective === "running" || effective === "background";
  useEffect(() => {
    if (!polling) return;
    const timer = setInterval(onRefresh, POLL_MS);
    return () => clearInterval(timer);
  }, [polling, onRefresh]);

  // background 态的批不归本组件持有(常驻批/别处触发),点停止后没有 promise
  // 可收尾——批停下后轮询把 digestRunning 拉回 false、离开进行态,在这里复位
  // stopping。running 态自己的 doRun finally 也会复位,这里只是统一兜底。
  // 离开进行态且刚点过停止 + 常驻开着 → 亮"本批已停止"过渡态;
  // 进行态恢复(常驻下个 tick 又开跑/用户手动再跑)→ 过渡态失效。
  useEffect(() => {
    if (polling) {
      setStoppedNotice(false);
      return;
    }
    if (!residentOn) setStoppedNotice(false); // 常驻被(从任何入口)关掉,"将继续"不再成立
    else if (stopping) setStoppedNotice(true);
    setStopping(false);
  }, [polling, stopping, residentOn]);

  if (stats.total <= 0) return null;

  /** 点「立即回填」:先确保 OCR 组件就绪,缺则弹确认(下载完自动继续回填)。 */
  const run = async () => {
    if (!(await ocrRuntimeReady())) {
      setOcrConfirm(true);
      return;
    }
    await doRun();
  };

  /** 确认下载 OCR 组件(banner 上显示进度),完成后自动开始回填。 */
  const downloadThenRun = async () => {
    setOcrConfirm(false);
    setPhase("downloading");
    setDlMb(0);
    const unlisten = await listen<EngineDownloadProgress>(
      ENGINE_DOWNLOAD_EVENT,
      (ev) => {
        if (ev.payload.stage === "runtime" && ev.payload.phase === "downloading") {
          setDlMb(Math.round(ev.payload.downloaded / 1024 / 1024));
        }
      },
    );
    try {
      await api.downloadOcrRuntime();
    } catch (e) {
      logError("chat.backfill.ocrDownload", e);
      setErrMsg(String(e));
      setPhase("failed");
      return;
    } finally {
      unlisten();
    }
    await doRun();
  };

  const doRun = async () => {
    setPhase("running");
    try {
      await api.memoryBackfill();
      // 停止按钮走 memoryDigestStop 翻标志,这里的 digest 感知后正常
      // resolve 已处理部分,落回 idle 初始态(剩余帧数还在,可再点回填)
      await api.memoryDigestNow();
      // 手动批被停止收尾时同样亮过渡态(常驻开着的话下个 tick 会接着跑)
      if (stopping && residentOn) setStoppedNotice(true);
      setPhase("idle");
      onRefresh();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已在运行")) {
        // 帧已登记,后台批会消化;转入后台态继续轮询进度
        setPhase("background");
      } else if (msg.includes("embedding runtime missing")) {
        // 文字识别运行时缺失/过旧(如 CPU→DirectML 迁移):指路而非裸报错
        setErrMsg(t("chat.backfill.runtimeMissing"));
        setPhase("failed");
      } else {
        logError("chat.backfill", e);
        setErrMsg(msg);
        setPhase("failed");
      }
    } finally {
      setStopping(false);
    }
  };

  /** 点「停止」:翻后端停止标志即返回,消化循环帧间感知(~1s)后停。
   *  running 态由 doRun 的收尾把 banner 落回初始态;background 态靠轮询
   *  看到 digestRunning=false 后离开进行态(stopping 在 polling effect 复位)。 */
  const stopRun = () => {
    setStopping(true);
    api.memoryDigestStop().catch((e) => logError("chat.backfill.stop", e));
  };

  return (
    <div className={styles.banner} role="status">
      <DatabaseZap size={14} strokeWidth={2} className={styles.bannerIcon} />
      <span className={styles.bannerText}>
        {effective === "running" &&
          t("chat.backfill.running", { count: stats.total })}
        {effective === "downloading" &&
          t("chat.backfill.downloadingOcr", { mb: dlMb })}
        {effective === "background" &&
          t("chat.backfill.alreadyRunning", { count: stats.total })}
        {effective === "failed" && t("chat.backfill.failed", { msg: errMsg })}
        {effective === "idle" &&
          (stoppedNotice
            ? t("chat.backfill.stoppedNotice")
            : t("chat.backfill.pending", { count: stats.total }))}
      </span>
      {/* 停止后的过渡态:就地给"彻底停"的逃生门,而不是让停止看起来会诈尸 */}
      {effective === "idle" && stoppedNotice && (
        <button
          type="button"
          className={styles.bannerBtn}
          onClick={() => {
            update({ memoryOcrResident: false });
            setStoppedNotice(false);
          }}
        >
          {t("chat.backfill.disableResident")}
        </button>
      )}
      {((effective === "idle" && !stoppedNotice) || effective === "failed") && (
        <button type="button" className={styles.bannerBtn} onClick={() => void run()}>
          {t("chat.backfill.action")}
        </button>
      )}
      {/* 手动批与后台批(常驻/别处触发)的当前轮都能停;常驻模式下个周期
          仍会继续消化,彻底停走 设置 → 常驻 OCR 开关 */}
      {polling && (
        <button
          type="button"
          className={styles.bannerBtn}
          onClick={stopRun}
          disabled={stopping}
        >
          {stopping ? t("chat.backfill.stopping") : t("chat.backfill.stop")}
        </button>
      )}
      {(polling || effective === "downloading") && (
        <Loader2 size={13} strokeWidth={2.25} className={styles.bannerSpin} />
      )}
      <ConfirmDialog
        open={ocrConfirm}
        title={t("chat.backfill.ocrConfirmTitle")}
        message={t("chat.backfill.ocrConfirmMessage")}
        confirmLabel={t("chat.backfill.ocrConfirmAccept")}
        variant="primary"
        onConfirm={() => void downloadThenRun()}
        onCancel={() => setOcrConfirm(false)}
      />
    </div>
  );
}
