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
/** 停止过渡态的慢速轮询间隔:后台批恢复运行要靠它刷新 digestRunning,
 *  否则「后台恢复后过渡态自动消失」永远等不到 */
const STOPPED_POLL_MS = 30_000;

/**
 * 未入索引提示条:有 N 张截图没进文字索引时显示,一键回填。
 * 索引进行期间每 3 秒重查剩余帧数,实时显示进度;剩余归零 banner 自动消失。
 * digest 报"已在运行"(常驻/按需的后台批持锁)按后台运行处理——帧已登记,
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
  // 停止后的语义收尾:后台识别开着时,"停止"只掐当前批——不明说用户会以为
  // 停止按钮坏了。批停下后显示过渡态 + 一键彻底关的逃生门,
  // 后台恢复运行(polling 重新为 true)或用户关掉后自动消失。
  const [stoppedNotice, setStoppedNotice] = useState(false);
  const residentOn = settings?.memoryOcrResident ?? false;
  // 与后端同口径:常驻优先。双真脏数据(如旧版本降级期间只写常驻)下实际跑的
  // 是常驻,过渡态不能说成按需档的一小时冷却。
  const autoOn = (settings?.memoryOcrAuto ?? false) && !residentOn;
  // 两档都会自己接着跑,过渡态与逃生门一视同仁;差别只在"多久之后"——
  // 常驻是下个 tick(~1 分钟),按需被后端压了一小时冷却,文案分开说。
  const backgroundOn = residentOn || autoOn;

  // 后端消化正在跑(常驻批/别处触发的手动批)时,即使本组件刚挂载
  // (比如用户切走再切回来),也直接显示"后台索引中"而不是带按钮的初始态
  const effective: Phase =
    phase === "idle" && stats.digestRunning ? "background" : phase;

  // 索引进行中轮询剩余数;total 归零时父组件的 stats 更新会让本组件不再渲染。
  // 过渡态期间降频慢查:后台批恢复运行时得有人把 digestRunning 刷回来,
  // 否则"后台恢复后过渡态自动消失"永远等不到,还会一直显示已停止的假象。
  const polling = effective === "running" || effective === "background";
  useEffect(() => {
    if (!polling && !stoppedNotice) return;
    const timer = setInterval(onRefresh, polling ? POLL_MS : STOPPED_POLL_MS);
    return () => clearInterval(timer);
  }, [polling, stoppedNotice, onRefresh]);

  // 停止收尾统一在这一处:点停止后 stopping 保持 true(doRun 不清它),
  // 等轮询把 digestRunning 拉回 false、离开进行态,在这里亮过渡态并复位。
  // 手动批与后台批共用这条路——doRun 里就地判 stopping 会读到陈旧闭包,
  // 且 resolve 时 stats 往往还没跟上,曾导致手动批停止后过渡态永远不亮。
  // 进行态恢复(后台下轮又开跑/用户手动再跑)→ 过渡态失效。
  useEffect(() => {
    if (polling) {
      setStoppedNotice(false);
      return;
    }
    // 后台识别被(从任何入口)关掉,"稍后会继续"不再成立
    if (!backgroundOn) setStoppedNotice(false);
    else if (stopping) setStoppedNotice(true);
    setStopping(false);
  }, [polling, stopping, backgroundOn]);

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
      // resolve 已处理部分,落回 idle 初始态(剩余帧数还在,可再点回填)。
      // stopping 不在这里清:收尾统一交给上面的 polling effect,它读得到
      // 最新状态(在这里判 stopping 读到的是发起时的陈旧闭包,永远 false)
      await api.memoryDigestNow();
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
    }
  };

  /** 点「停止」:翻后端停止标志即返回,消化循环帧间感知(~1s)后停。
   *  收尾(亮过渡态 + 复位 stopping)统一在 polling effect:轮询看到
   *  digestRunning 落回 false、离开进行态时处理,手动批与后台批同一条路。 */
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
            ? // 按需档被压了一小时冷却,说得出确切口径就别含糊
              t(
                autoOn
                  ? "chat.backfill.stoppedNoticeAuto"
                  : "chat.backfill.stoppedNotice",
              )
            : t("chat.backfill.pending", { count: stats.total }))}
      </span>
      {/* 停止后的过渡态:就地给"彻底停"的逃生门,而不是让停止看起来会诈尸 */}
      {effective === "idle" && stoppedNotice && (
        <button
          type="button"
          className={styles.bannerBtn}
          onClick={() => {
            // 两档一起关(与设置页 "off" 分支同一份 patch),不必分辨是哪一档
            update({ memoryOcrResident: false, memoryOcrAuto: false });
            setStoppedNotice(false);
          }}
        >
          {t("chat.backfill.disableResident")}
        </button>
      )}
      {/* 过渡态里也保留回填入口:按需档的冷却最长一小时,期间用户可能就想
          手动把积压清了(手动批不走冷却);点下去 polling 恢复,过渡态自然消失 */}
      {(effective === "idle" || effective === "failed") && (
        <button type="button" className={styles.bannerBtn} onClick={() => void run()}>
          {t("chat.backfill.action")}
        </button>
      )}
      {/* 手动批与后台批(常驻/按需/别处触发)的当前轮都能停;常驻与按需模式
          稍后仍会继续消化,彻底停走 设置 → 屏幕文字识别 → 关闭 */}
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
