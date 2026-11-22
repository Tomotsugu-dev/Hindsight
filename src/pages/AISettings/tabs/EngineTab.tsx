import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  ArrowUpFromLine,
  Cable,
  Check,
  Download,
  FolderOpen,
  Info,
  Pilcrow,
  Server,
  Trash2,
  XCircle,
} from "lucide-react";
import { Button } from "../../../components/Button/Button";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import {
  api,
  ENGINE_DOWNLOAD_EVENT,
  type EngineDownloadProgress,
  type EngineStatus,
  type ModelRef,
  type TaskConnectionTest,
} from "../../../api/hindsight";
import { findAdvanced, patchAdvanced, type AiTask } from "../../../state/reportModel";
import { useAiSettings } from "../shared/useAiSettings";
import { DownloadSourcesSection } from "../shared/DownloadSourcesSection";
import {
  ENGINE_BATCH_OPTIONS,
  ENGINE_CTX_OPTIONS,
  ENGINE_SLOTS_OPTIONS,
  type EngineBatchKey,
  type EngineCtxKey,
  type EngineSlotsKey,
  engineBatchToOption,
  engineCtxToOption,
  engineOptionToBatch,
  engineOptionToCtx,
  engineOptionToSlots,
  engineSlotsToOption,
  humanAccelLabel,
  isRecommendedApplied,
  recommendEngineParams,
} from "../shared/engineParams";
import { VramEstimateLine } from "../shared/VramEstimate";
import { localizeAiError } from "../../../lib/aiErrors";
import { logError, logWarn } from "../../../lib/logger";
import styles from "../AISettings.module.css";

type TestResult =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; items: TaskConnectionTest[] }
  | { kind: "fail"; message: string };

export default function EngineTab() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  // mount 时拉一次：systemVram + platformId 给"应用推荐"算法用。
  // 后端 OnceLock 缓存，重复调几乎零成本；不参与 EngineSection 自己的 5s 轮询
  const [hwSnapshot, setHwSnapshot] = useState<{
    systemVram: EngineStatus["systemVram"];
    platformId: string | undefined;
  }>({ systemVram: null, platformId: undefined });
  useEffect(() => {
    api
      .getEngineStatus()
      .then((s) => setHwSnapshot({ systemVram: s.systemVram, platformId: s.platformId }))
      .catch((e) => logError("engine.getStatus.hwSnapshot", e));
  }, []);

  // 找一个选中的本地模型来显示/调整高级引擎参数（优先 daily，再看 chat / weekly）
  const localTarget = useMemo<{
    task: AiTask;
    model: Extract<ModelRef, { kind: "local" }>;
  } | null>(() => {
    if (!ai) return null;
    if (ai.picks.daily?.kind === "local" && ai.picks.daily.file.trim()) {
      return { task: "daily", model: { kind: "local", file: ai.picks.daily.file.trim() } };
    }
    if (ai.picks.chat?.kind === "local" && ai.picks.chat.file.trim()) {
      return { task: "chat", model: { kind: "local", file: ai.picks.chat.file.trim() } };
    }
    if (ai.picks.weekly.kind === "local" && ai.picks.weekly.file.trim()) {
      return { task: "weekly", model: { kind: "local", file: ai.picks.weekly.file.trim() } };
    }
    return null;
  }, [ai]);

  const adv = ai && localTarget ? findAdvanced(ai, localTarget.task, localTarget.model) : undefined;
  const summaryBatch = adv?.batchSize ?? null;
  const summarySlots = adv?.parallelSlots ?? null;
  const summaryCtx = adv?.ctxSize ?? null;

  const recommendSummary = useMemo(() => {
    if (!localTarget) return null;
    return recommendEngineParams(
      hwSnapshot.systemVram,
      localTarget.model.file,
      hwSnapshot.platformId,
    );
  }, [localTarget, hwSnapshot]);

  // 聊天起引擎只用上下文；批大小和并发路数后端保存时会清掉，这里也不显示
  const ctxOnly = localTarget?.task === "chat";

  const summaryApplied =
    recommendSummary != null &&
    isRecommendedApplied(recommendSummary, {
      batchSize: ctxOnly ? recommendSummary.batchSize : summaryBatch,
      parallelSlots: ctxOnly ? recommendSummary.parallelSlots : summarySlots,
      ctxSize: summaryCtx,
    });

  const updateEngineParam = (patch: {
    batchSize?: number | null;
    parallelSlots?: number | null;
    ctxSize?: number | null;
  }) => {
    if (!ai || !localTarget) return;
    updateAi({
      advanced: patchAdvanced(ai, localTarget.task, localTarget.model, patch),
    });
  };

  const handleApplySummary = () => {
    if (!recommendSummary) return;
    updateEngineParam(
      ctxOnly
        ? { ctxSize: recommendSummary.ctxSize }
        : {
            batchSize: recommendSummary.batchSize,
            parallelSlots: recommendSummary.parallelSlots,
            ctxSize: recommendSummary.ctxSize,
          },
    );
  };

  if (!ai) return null;

  return (
    <div className={styles.content}>
      <DownloadSourcesSection />
      <Section
        title={t("aiSettings.engine.sectionTitle")}
        description={t("aiSettings.engine.sectionDesc")}
        icon={Server}
      >
        <EngineSection />
        <OcrRuntimeSection />
      </Section>

      {localTarget && (
        <Section title={t("aiSettings.summaryParams.sectionTitle")} icon={Pilcrow}>
          {!ctxOnly && (
            <>
              <Row label={t("aiSettings.engineParams.batch")}>
                <SimplePicker<EngineBatchKey>
                  value={engineBatchToOption(summaryBatch)}
                  options={ENGINE_BATCH_OPTIONS}
                  onChange={(next) => updateEngineParam({ batchSize: engineOptionToBatch(next) })}
                />
              </Row>
              <Row label={t("aiSettings.engineParams.slots")}>
                <SimplePicker<EngineSlotsKey>
                  value={engineSlotsToOption(summarySlots)}
                  options={ENGINE_SLOTS_OPTIONS}
                  onChange={(next) =>
                    updateEngineParam({ parallelSlots: engineOptionToSlots(next) })
                  }
                />
              </Row>
            </>
          )}
          <Row label={t("aiSettings.engineParams.ctx")}>
            <SimplePicker<EngineCtxKey>
              value={engineCtxToOption(summaryCtx)}
              options={ENGINE_CTX_OPTIONS}
              onChange={(next) => updateEngineParam({ ctxSize: engineOptionToCtx(next) })}
            />
          </Row>
          <VramEstimateLine
            modelName={localTarget.model.file}
            parallelSlots={summarySlots ?? 1}
            ctxSize={summaryCtx ?? 8192}
            systemVram={hwSnapshot.systemVram}
            recommended={recommendSummary}
            recommendedApplied={summaryApplied}
            onApplyRecommended={handleApplySummary}
          />
        </Section>
      )}
    </div>
  );
}

/**
 * 本地 AI 引擎安装状态卡片（Phase 1B-α）。
 *
 * - mount 时拉一次 api.getEngineStatus，下载 / 删除后再拉一次
 * - 进度通过 listen ENGINE_DOWNLOAD_EVENT 实时更新
 * - 下载中显示可视进度条 + 百分比 + MB 数
 */
function EngineSection() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineDownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 启动 / 停止 / 测试 三个动作的 in-flight 状态
  const [engineBusy, setEngineBusy] = useState(false);
  const [testResult, setTestResult] = useState<TestResult>({ kind: "idle" });
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const refresh = async () => {
    try {
      setStatus(await api.getEngineStatus());
    } catch (e) {
      logError("engine.getStatus", e);
    }
  };

  useEffect(() => {
    void refresh();
    // 只认 llama.cpp 阶段;OCR 组件(stage="runtime")的进度归 OcrRuntimeSection
    const p = listen<EngineDownloadProgress>(ENGINE_DOWNLOAD_EVENT, (ev) => {
      if (ev.payload.stage !== "runtime") setProgress(ev.payload);
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  // 引擎跑起来后，每 5s 重拉一次状态——让 idle 倒计时（idleSecondsRemaining）实时
  // 跟着减少，并在 watcher 自动 stop 后及时把 badge 切回 stopped。
  // 不跑时（stopped/error/starting）不轮询省 invoke。
  useEffect(() => {
    if (status?.runtime.state !== "running") return;
    const id = window.setInterval(() => void refresh(), 5_000);
    return () => window.clearInterval(id);
  }, [status?.runtime.state]);

  const onDownload = async () => {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      // 已安装状态下点的是「重新下载/更新」→ force 强制重下(修复损坏安装);
      // 未安装则正常下载(后端幂等跳过逻辑不会挡)
      await api.downloadBinary(installed);
      await refresh();
      // done 信号到达时 progress 已被 listen 设过；保留几百 ms 让用户看到，
      // 然后清空，避免下次操作前停留旧状态
      setTimeout(() => setProgress(null), 800);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  };

  const onDelete = () => {
    setConfirmingDelete(true);
  };

  const doDelete = async () => {
    setConfirmingDelete(false);
    setBusy(true);
    setError(null);
    try {
      await api.deleteBinary();
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  };

  /** 手动停掉引擎释放 VRAM。AI 总结跑完后用户想腾出 GPU 资源时点这里；
   *  下次跑总结 summary.rs 会 lazy spawn 自动重启。 */
  const onReleaseVram = async () => {
    setEngineBusy(true);
    try {
      await api.stopEngine();
    } catch (e) {
      logWarn("engine.releaseVram", e);
    } finally {
      await refresh();
      setEngineBusy(false);
    }
  };

  /** 测试日报、周报、聊天三条当前配置的模型连接（后端测完自动停本地引擎释放显存）。 */
  const onTestLocal = async () => {
    setEngineBusy(true);
    setTestResult({ kind: "running" });
    try {
      const items = await api.testAiConnections();
      setTestResult({ kind: "done", items });
    } catch (e) {
      setTestResult({
        kind: "fail",
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      await refresh();
      setEngineBusy(false);
    }
  };

  if (!status) {
    return <div className={styles.engineCard}>{t("aiSettings.engine.loading")}</div>;
  }

  // 本卡只管 llama.cpp;OCR 的 onnxruntime 是独立组件(下方 OcrRuntimeSection)
  const installed = status.installed;
  const accelLabel = humanAccelLabel(status.platformId, t);
  const version = installed ? status.installedVersion : status.currentPin;
  const stale =
    installed && status.installedVersion !== null && status.installedVersion !== status.currentPin;
  // Windows 但 CUDA 未检测到：建议先装 NVIDIA CUDA
  const noCudaWarning = status.platformId === "win-cpu-x64";

  // 下载按钮的当前文案：busy / 已装 / stale / 全新 四态
  const downloadBtnLabel = busy
    ? installed
      ? t("aiSettings.engine.actions.updating")
      : t("aiSettings.engine.actions.downloading")
    : installed
      ? stale
        ? t("aiSettings.engine.actions.updateToLatest")
        : t("aiSettings.engine.actions.redownload")
      : t("aiSettings.engine.actions.downloadEngine");

  const versionDisplay = version ?? t("aiSettings.engine.versionUnknown");

  return (
    <>
      <div className={styles.engineCard}>
        {/* 第 1 行：name + ⓘ + size + 状态 badge —— inline 信息条，扁平不带 box */}
        <div className={styles.engineInfoRow}>
          <span className={styles.engineName}>llama.cpp</span>
          <button
            type="button"
            className={styles.engineInfoWrap}
            aria-label={
              stale
                ? t("aiSettings.engine.versionStaleAria", {
                    version: versionDisplay,
                    latest: status.currentPin,
                  })
                : t("aiSettings.engine.versionAria", { version: versionDisplay })
            }
          >
            <Info size={12} strokeWidth={2.2} className={styles.engineInfoIcon} />
            <span className={styles.engineInfoTip} role="tooltip">
              {t("aiSettings.engine.versionLabel", { version: versionDisplay })}
              {stale
                ? t("aiSettings.engine.versionStaleLabel", {
                    latest: status.currentPin,
                  })
                : ""}
            </span>
          </button>
          <span className={styles.engineSize} style={busy ? { visibility: "hidden" } : undefined}>
            {t("aiSettings.engine.actions.approxSize", {
              size: Math.round(status.estimatedBytes / 1024 / 1024),
            })}
          </span>
          <span
            className={`${styles.engineBadge} ${
              installed ? styles.engineBadgeOk : styles.engineBadgeWarn
            }`}
          >
            {installed ? t("aiSettings.engine.installed") : t("aiSettings.engine.notInstalled")}
          </span>
        </div>

        {/* 第 2 行：检测到的加速后端，灰小字 subtitle */}
        <div className={styles.engineDetectedRow}>
          {t("aiSettings.engine.detected", { accel: accelLabel })}
        </div>

        {noCudaWarning ? (
          <div className={styles.engineWarning}>
            <AlertTriangle size={14} strokeWidth={2.2} />
            <div className={styles.engineWarningBody}>
              <strong>{t("aiSettings.engine.noCuda.headline")}</strong>
              <span>
                {t("aiSettings.engine.noCuda.prefix")}
                <button
                  type="button"
                  className={styles.engineWarningLink}
                  onClick={() => void openUrl("https://developer.nvidia.com/cuda-downloads")}
                >
                  {t("aiSettings.engine.noCuda.linkText")}
                </button>
                {t("aiSettings.engine.noCuda.suffix")}
              </span>
            </div>
          </div>
        ) : null}

        {error ? <div className={styles.engineError}>{error}</div> : null}

        {/* key=stage：EngineProgress 的 maxMbRef 是单调的，靠换 key 在 engine → runtime
          两个下载阶段之间重置，否则第二阶段的 MB 数冻结在第一阶段的最大值 */}
        {progress ? <EngineProgress key={progress.stage} progress={progress} /> : null}

        {/* 第 3 行：5 个按钮平铺一行
          左：下载 / 测试连接；右：释放显存 / 打开 / 卸载 */}
        <div className={styles.engineActionsFlat}>
          <Button
            variant="outline"
            tone="accent"
            size="lg"
            icon={Download}
            onClick={() => void onDownload()}
            loading={busy}
          >
            {downloadBtnLabel}
          </Button>
          <Button
            variant="outline"
            tone="accent"
            size="lg"
            icon={Cable}
            onClick={() => void onTestLocal()}
            loading={engineBusy}
            disabled={busy}
            title={t("aiSettings.engine.actions.testTooltipReady")}
          >
            {t("aiSettings.engine.actions.testConnection")}
          </Button>
          {/* spacer：让"释放显存 / 打开 / 卸载"靠右贴齐，跟左边主操作分组 */}
          <span className={styles.engineActionsSpacer} aria-hidden="true" />
          {/* 释放显存：永远显示。未装 / 未跑 → disabled 灰着不可点；引擎在跑时才激活。 */}
          <Button
            variant="outline"
            size="lg"
            icon={ArrowUpFromLine}
            onClick={() => void onReleaseVram()}
            disabled={!installed || status.runtime.state !== "running" || engineBusy}
            title={
              !installed
                ? t("aiSettings.engine.actions.releaseVramTooltipNotInstalled")
                : status.runtime.state !== "running"
                  ? t("aiSettings.engine.actions.releaseVramTooltipNotRunning")
                  : t("aiSettings.engine.actions.releaseVramTooltip")
            }
          >
            {t("aiSettings.engine.actions.releaseVram")}
          </Button>
          <Button
            variant="outline"
            size="lg"
            icon={FolderOpen}
            onClick={() => void api.openEngineDir().catch((e) => logError("engine.openDir", e))}
            disabled={busy || !installed}
            title={
              installed
                ? t("aiSettings.engine.actions.openFolderTooltipInstalled")
                : t("aiSettings.engine.actions.openFolderTooltipNotInstalled")
            }
          >
            {t("common.open")}
          </Button>
          <Button
            variant="outline"
            tone="danger"
            size="lg"
            icon={Trash2}
            onClick={onDelete}
            disabled={busy || !installed}
            title={
              installed
                ? t("aiSettings.engine.actions.uninstallTooltipInstalled")
                : t("aiSettings.engine.actions.uninstallTooltipNotInstalled")
            }
          >
            {t("aiSettings.engine.actions.uninstall")}
          </Button>
        </div>

        {installed || testResult.kind !== "idle" ? (
          <EngineRuntimeRow status={status} testResult={testResult} />
        ) : null}
      </div>
      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSettings.engine.uninstallConfirmTitle")}
        message={t("aiSettings.engine.uninstallConfirmMessage")}
        variant="danger"
        onConfirm={() => void doDelete()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </>
  );
}

/**
 * 文字识别(OCR)组件卡——onnxruntime 运行时,与 llama.cpp 完全独立。
 * 只服务屏幕记忆的 OCR:云端 API 用户只装它(~40MB)就能用全部功能。
 * macOS 走系统 Vision,显示"无需下载"说明,不给任何按钮。
 */
function OcrRuntimeSection() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineDownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const refresh = async () => {
    try {
      setStatus(await api.getEngineStatus());
    } catch (e) {
      logError("ocrRuntime.getStatus", e);
    }
  };

  useEffect(() => {
    void refresh();
    const p = listen<EngineDownloadProgress>(ENGINE_DOWNLOAD_EVENT, (ev) => {
      if (ev.payload.stage === "runtime") setProgress(ev.payload);
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  if (!status) return null;
  const rt = status.embeddingRuntime;
  // macOS 用系统 Vision(ANE),零下载零依赖——只放一行说明
  if (status.platformId.startsWith("macos")) {
    return (
      <div className={styles.engineCard}>
        <div className={styles.engineInfoRow}>
          <span className={`${styles.engineName} ${styles.engineNameCjk}`}>
            {t("aiSettings.engine.ocr.name")}
          </span>
          <span className={`${styles.engineBadge} ${styles.engineBadgeOk}`}>
            {t("aiSettings.engine.ocr.macNativeBadge")}
          </span>
        </div>
        <div className={styles.engineDetectedRow}>{t("aiSettings.engine.ocr.macNativeDesc")}</div>
      </div>
    );
  }

  const installed = rt.installed;
  // 有版本文件但判未装 = 旧 CPU 构建残留(DirectML 迁移),提示升级收益
  const upgradeHint = !installed && !!rt.installedVersion;

  const onDownload = async () => {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      await api.downloadOcrRuntime(installed);
      await refresh();
      setTimeout(() => setProgress(null), 800);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  };

  const doDelete = async () => {
    setConfirmingDelete(false);
    setBusy(true);
    setError(null);
    try {
      await api.deleteOcrRuntime();
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className={styles.engineCard}>
        <div className={styles.engineInfoRow}>
          <span className={`${styles.engineName} ${styles.engineNameCjk}`}>
            {t("aiSettings.engine.ocr.name")}
          </span>
          <span className={styles.engineSize} style={busy ? { visibility: "hidden" } : undefined}>
            {t("aiSettings.engine.actions.approxSize", {
              size: Math.round(rt.estimatedBytes / 1024 / 1024),
            })}
          </span>
          <span
            className={`${styles.engineBadge} ${
              installed ? styles.engineBadgeOk : styles.engineBadgeWarn
            }`}
          >
            {installed ? t("aiSettings.engine.installed") : t("aiSettings.engine.notInstalled")}
          </span>
        </div>
        <div className={styles.engineDetectedRow}>
          {upgradeHint ? t("aiSettings.engine.ocr.upgradeHint") : t("aiSettings.engine.ocr.desc")}
        </div>

        {error ? <div className={styles.engineError}>{error}</div> : null}
        {progress ? <EngineProgress key={progress.stage} progress={progress} /> : null}

        <div className={styles.engineActionsFlat}>
          <Button
            variant="outline"
            tone="accent"
            size="lg"
            icon={Download}
            onClick={() => void onDownload()}
            loading={busy}
          >
            {busy
              ? t("aiSettings.engine.actions.downloading")
              : installed
                ? t("aiSettings.engine.actions.redownload")
                : t("aiSettings.engine.ocr.download")}
          </Button>
          <span className={styles.engineActionsSpacer} aria-hidden="true" />
          <Button
            variant="outline"
            tone="danger"
            size="lg"
            icon={Trash2}
            onClick={() => setConfirmingDelete(true)}
            disabled={busy || !installed}
          >
            {t("aiSettings.engine.actions.uninstall")}
          </Button>
        </div>
      </div>
      <ConfirmDialog
        open={confirmingDelete}
        title={t("aiSettings.engine.ocr.uninstallConfirmTitle")}
        message={t("aiSettings.engine.ocr.uninstallConfirmMessage")}
        variant="danger"
        onConfirm={() => void doDelete()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </>
  );
}

function EngineProgress({ progress }: { progress: EngineDownloadProgress }) {
  const { t } = useTranslation();
  // 取整 + 单调递增显示：消除小数频繁跳动，并守住「数字只能涨不能退」。
  // 用 ref 不触发额外 render；新值 ≤ 当前 max 就保持显示老值。
  // 注意：每个 stage（engine / runtime）独立计 maxMb，切换阶段时 ref 应重置；
  // 用 stage 当 key 在父级渲染时帮我们做这件事——这里只在同 stage 内累计。
  const maxMbRef = useRef(0);
  const currentMb = Math.round(progress.downloaded / 1024 / 1024);
  if (currentMb > maxMbRef.current) maxMbRef.current = currentMb;
  // 阶段文案前缀：engine 时 "下载 AI 引擎中…"、runtime 时 "下载推理库中…"
  const stageKey =
    progress.stage === "runtime"
      ? "aiSettings.engine.progress.stageRuntime"
      : "aiSettings.engine.progress.stageEngine";
  if (progress.phase === "downloading") {
    return (
      <div className={styles.engineProgressWrap}>
        <div className={styles.engineProgressBar}>
          <div
            className={`${styles.engineProgressFill} ${styles.engineProgressFillIndeterminate}`}
          />
        </div>
        <div className={styles.engineProgressText}>
          {t(stageKey)} ·{" "}
          {t("aiSettings.engine.progress.downloading", {
            size: maxMbRef.current,
          })}
        </div>
      </div>
    );
  }
  // verifying / extracting / done 都没字节进度，给单行文字提示
  const label =
    progress.phase === "verifying"
      ? t("aiSettings.engine.progress.verifying")
      : progress.phase === "extracting"
        ? t("aiSettings.engine.progress.extracting")
        : t("aiSettings.engine.progress.done");
  return (
    <div className={styles.engineProgressText}>
      {t(stageKey)} · {label}
    </div>
  );
}

/** 把秒数格式化成"X 分 Y 秒后释放显存" / "Y 秒后释放显存"。i18n 三语共用。 */
function formatIdleCountdown(
  seconds: number,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const safe = Math.max(0, Math.floor(seconds));
  if (safe < 60) {
    return t("aiSettings.engine.runtime.idleCountdownSec", { sec: safe });
  }
  const min = Math.floor(safe / 60);
  const sec = safe % 60;
  return t("aiSettings.engine.runtime.idleCountdownMin", { min, sec });
}

/**
 * 引擎运行时反馈行：状态徽章 + testResult 输出。
 * 测试连接已合并到上方的按钮行（点了会 start → test → stop）。
 */
function EngineRuntimeRow({
  status,
  testResult,
}: {
  status: EngineStatus;
  testResult: TestResult;
}) {
  const { t } = useTranslation();
  const rt = status.runtime;
  const isError = rt.state === "error";

  const badge =
    rt.state === "running"
      ? {
          text: t("aiSettings.engine.runtime.running", { port: rt.port }),
          cls: styles.engineBadgeOk,
        }
      : rt.state === "starting"
        ? {
            text: t("aiSettings.engine.runtime.starting"),
            cls: styles.engineBadgeWarn,
          }
        : rt.state === "error"
          ? {
              text: t("aiSettings.engine.runtime.error"),
              cls: styles.engineBadgeFail,
            }
          : null;

  // idle 倒计时：仅 running + 无 in-flight 时显示。"还剩 1m 23s 自动释放显存"。
  // 后端每 5s 推一次新数字（EngineSection 的 polling），前端不做本地插值——
  // 5s 跳一次足够清楚，避免每秒 re-render 的额外开销。
  const idleHint =
    rt.state === "running" && rt.idleSecondsRemaining != null
      ? formatIdleCountdown(rt.idleSecondsRemaining, t)
      : null;

  // 没在测、没出错、状态条又能直接看 → 不渲染额外行
  const hasContent = testResult.kind !== "idle" || badge !== null || isError;
  if (!hasContent) return null;

  return (
    <div className={styles.engineRuntime}>
      <div className={styles.engineRuntimeRow}>
        {testResult.kind === "done" ? (
          <div className={styles.externalTestSteps}>
            {testResult.items.map((item) => {
              const taskLabel = t(
                item.task === "daily"
                  ? "aiSummary.tabs.daily"
                  : item.task === "weekly"
                    ? "aiSummary.tabs.week"
                    : "nav.items.chat",
              );
              const detail = item.model ? `${taskLabel}（${item.model}）` : taskLabel;
              return (
                <span
                  key={item.task}
                  className={`${styles.engineRuntimeStatus} ${
                    item.ok ? styles.engineRuntimeStatusOk : styles.engineRuntimeStatusFail
                  }`}
                >
                  {item.ok ? (
                    <Check size={14} strokeWidth={2.2} />
                  ) : (
                    <XCircle size={14} strokeWidth={2.2} />
                  )}
                  {item.ok ? detail : `${detail}：${localizeAiError(item.message)}`}
                </span>
              );
            })}
          </div>
        ) : null}
        {testResult.kind === "fail" ? (
          <span className={`${styles.engineRuntimeStatus} ${styles.engineRuntimeStatusFail}`}>
            <XCircle size={14} strokeWidth={2.2} />
            {testResult.message}
          </span>
        ) : null}

        {idleHint ? <span className={styles.engineIdleHint}>{idleHint}</span> : null}

        {badge ? (
          <span className={`${styles.engineBadge} ${badge.cls} ${styles.engineRuntimeBadgeRight}`}>
            {badge.text}
          </span>
        ) : null}
      </div>

      {isError && rt.error ? <div className={styles.engineRuntimeError}>{rt.error}</div> : null}
    </div>
  );
}
