import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  Download,
  FolderOpen,
  PowerOff,
  Info,
  Loader2,
  Server,
  Trash2,
  XCircle,
} from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import {
  api,
  ENGINE_DOWNLOAD_EVENT,
  type EngineDownloadProgress,
  type EngineStatus,
} from "../../../api/hindsight";
import { useAiSettings } from "../shared/useAiSettings";
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
import { logError, logWarn } from "../../../lib/logger";
import styles from "../AISettings.module.css";

type TestResult =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; models: string[] }
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
      .then((s) =>
        setHwSnapshot({ systemVram: s.systemVram, platformId: s.platformId }),
      )
      .catch((e) => logError("engine.getStatus.hwSnapshot", e));
  }, []);

  // 双套推荐参数：图描述（slots 优先）/ 段总结（ctx 优先）。
  // 模型名各自走 step 专用字段（describeMain / summaryMain），fallback 到旧 activeMain。
  // 否则用户只设了 step-specific 模型时 activeMain 为空，recommendEngineParams 找不到
  // 模型 spec 返回 null，「应用推荐」按钮不渲染。
  const recommendDescribe = useMemo(() => {
    if (!ai) return null;
    return recommendEngineParams(
      hwSnapshot.systemVram,
      ai.describeMain || ai.activeMain,
      hwSnapshot.platformId,
      "describe",
    );
  }, [ai, hwSnapshot]);
  const recommendSummary = useMemo(() => {
    if (!ai) return null;
    return recommendEngineParams(
      hwSnapshot.systemVram,
      ai.summaryMain || ai.activeMain,
      hwSnapshot.platformId,
      "summary",
    );
  }, [ai, hwSnapshot]);

  // 「已是推荐值」判断各自针对自己那套字段——picker 显示的是 effective 值
  // （new 字段 ?? old 字段），所以比较时也用同一逻辑
  const describeApplied =
    recommendDescribe != null &&
    ai != null &&
    isRecommendedApplied(recommendDescribe, {
      batchSize: ai.describeBatchSize ?? ai.batchSize,
      parallelSlots: ai.describeParallelSlots ?? ai.parallelSlots,
      ctxSize: ai.describeCtxSize ?? ai.ctxSize,
    });
  const summaryApplied =
    recommendSummary != null &&
    ai != null &&
    isRecommendedApplied(recommendSummary, {
      batchSize: ai.summaryBatchSize ?? ai.batchSize,
      parallelSlots: ai.summaryParallelSlots ?? ai.parallelSlots,
      ctxSize: ai.summaryCtxSize ?? ai.ctxSize,
    });

  // 推荐里的 null 表示"用 llama.cpp 默认值"（batch=512 / ctx=8192）。
  // 但 effective getter 是 `describeBatchSize ?? batchSize`——describeBatchSize=null
  // 会 fallback 到旧的全局 ai.batchSize（用户之前手设的值）。结果就是按了推荐没生效。
  // 这里把 null 转显式默认值，让 effective 直接读 step-specific 字段，不再 fallback。
  const handleApplyDescribe = () => {
    if (!recommendDescribe) return;
    updateAi({
      describeBatchSize: recommendDescribe.batchSize ?? 512,
      describeParallelSlots: recommendDescribe.parallelSlots,
      describeCtxSize: recommendDescribe.ctxSize ?? 8192,
    });
  };
  const handleApplySummary = () => {
    if (!recommendSummary) return;
    updateAi({
      summaryBatchSize: recommendSummary.batchSize ?? 512,
      summaryParallelSlots: recommendSummary.parallelSlots,
      summaryCtxSize: recommendSummary.ctxSize ?? 8192,
    });
  };

  if (!ai) return null;

  // picker 显示用 effective 值（描述阶段未填的字段降级到旧全局字段）
  const describeBatch = ai.describeBatchSize ?? ai.batchSize;
  const describeSlots = ai.describeParallelSlots ?? ai.parallelSlots;
  const describeCtx = ai.describeCtxSize ?? ai.ctxSize;
  const summaryBatch = ai.summaryBatchSize ?? ai.batchSize;
  const summarySlots = ai.summaryParallelSlots ?? ai.parallelSlots;
  const summaryCtx = ai.summaryCtxSize ?? ai.ctxSize;

  return (
    <div className={styles.content}>
      <Section
        title={t("aiSettings.engine.sectionTitle")}
        description={t("aiSettings.engine.sectionDesc")}
        icon={Server}
      >
        <EngineSection />
      </Section>

      {/* 引擎参数双套——图描述（slots 优先）+ 段总结（ctx 优先）。
          两阶段串行执行：日报跑 step 1 用 describe 配置，跑完 stop+start 切到
          summary 配置跑 step 2。新字段 null 时降级到旧全局 batchSize/parallelSlots/
          ctxSize（兼容老 settings JSON），所以两个 Section picker 可能展示同一个值。 */}
      <Section
        title={t("aiSettings.describeParams.sectionTitle")}
        icon={Server}
      >
        <Row label={t("aiSettings.engineParams.batch")}>
          <SimplePicker<EngineBatchKey>
            value={engineBatchToOption(describeBatch)}
            options={ENGINE_BATCH_OPTIONS}
            onChange={(next) =>
              updateAi({ describeBatchSize: engineOptionToBatch(next) })
            }
          />
        </Row>
        <Row label={t("aiSettings.engineParams.slots")}>
          <SimplePicker<EngineSlotsKey>
            value={engineSlotsToOption(describeSlots)}
            options={ENGINE_SLOTS_OPTIONS}
            onChange={(next) =>
              updateAi({ describeParallelSlots: engineOptionToSlots(next) })
            }
          />
        </Row>
        <Row label={t("aiSettings.engineParams.ctx")}>
          <SimplePicker<EngineCtxKey>
            value={engineCtxToOption(describeCtx)}
            options={ENGINE_CTX_OPTIONS}
            onChange={(next) =>
              updateAi({ describeCtxSize: engineOptionToCtx(next) })
            }
          />
        </Row>
        <VramEstimateLine
          modelName={ai.describeMain || ai.activeMain}
          parallelSlots={describeSlots ?? 1}
          ctxSize={describeCtx ?? 8192}
          systemVram={hwSnapshot.systemVram}
          recommended={recommendDescribe}
          recommendedApplied={describeApplied}
          onApplyRecommended={handleApplyDescribe}
        />
      </Section>

      <Section
        title={t("aiSettings.summaryParams.sectionTitle")}
        icon={Server}
      >
        <Row label={t("aiSettings.engineParams.batch")}>
          <SimplePicker<EngineBatchKey>
            value={engineBatchToOption(summaryBatch)}
            options={ENGINE_BATCH_OPTIONS}
            onChange={(next) =>
              updateAi({ summaryBatchSize: engineOptionToBatch(next) })
            }
          />
        </Row>
        <Row label={t("aiSettings.engineParams.slots")}>
          <SimplePicker<EngineSlotsKey>
            value={engineSlotsToOption(summarySlots)}
            options={ENGINE_SLOTS_OPTIONS}
            onChange={(next) =>
              updateAi({ summaryParallelSlots: engineOptionToSlots(next) })
            }
          />
        </Row>
        <Row label={t("aiSettings.engineParams.ctx")}>
          <SimplePicker<EngineCtxKey>
            value={engineCtxToOption(summaryCtx)}
            options={ENGINE_CTX_OPTIONS}
            onChange={(next) =>
              updateAi({ summaryCtxSize: engineOptionToCtx(next) })
            }
          />
        </Row>
        <VramEstimateLine
          modelName={ai.summaryMain || ai.activeMain}
          parallelSlots={summarySlots ?? 1}
          ctxSize={summaryCtx ?? 8192}
          systemVram={hwSnapshot.systemVram}
          recommended={recommendSummary}
          recommendedApplied={summaryApplied}
          onApplyRecommended={handleApplySummary}
        />
      </Section>
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
    const p = listen<EngineDownloadProgress>(
      ENGINE_DOWNLOAD_EVENT,
      (ev) => setProgress(ev.payload),
    );
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
      await api.downloadBinary();
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

  /** 「测试连接」合并按钮：start_engine → test_ai_endpoint → stop_engine。
   *
   *  无论之前引擎是否在跑，测完都 stop 释放 VRAM——纯诊断流程，不留下资源占用。
   *  实际跑总结时 summary.rs 会 lazy spawn，不依赖这里启动的实例。
   *
   *  start 是异步等 90s health；中间过程 testResult.kind="running" 给 UI 反馈。 */
  const onTestLocal = async () => {
    setEngineBusy(true);
    setTestResult({ kind: "running" });
    try {
      const port = await api.startEngine();
      await refresh();
      const r = await api.testAiEndpoint(`http://127.0.0.1:${port}/v1`);
      if (r.ok) setTestResult({ kind: "ok", models: r.models });
      else setTestResult({ kind: "fail", message: r.message });
    } catch (e) {
      setTestResult({
        kind: "fail",
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      // 测完无脑 stop，释放 VRAM；stop 失败仅 log 不影响 testResult
      try {
        await api.stopEngine();
      } catch (e) {
        logWarn("engine.testStopFailed", e);
      }
      await refresh();
      setEngineBusy(false);
    }
  };

  if (!status) {
    return <div className={styles.engineCard}>{t("aiSettings.engine.loading")}</div>;
  }

  const installed = status.installed;
  const accelLabel = humanAccelLabel(status.platformId, t);
  const version = installed ? status.installedVersion : status.currentPin;
  const stale =
    installed &&
    status.installedVersion !== null &&
    status.installedVersion !== status.currentPin;
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
      <div className={styles.engineHead}>
        <span
          className={`${styles.engineBadge} ${
            installed ? styles.engineBadgeOk : styles.engineBadgeWarn
          }`}
        >
          {installed
            ? t("aiSettings.engine.installed")
            : t("aiSettings.engine.notInstalled")}
        </span>
        <span className={styles.engineMeta}>
          llama.cpp
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
          <span className={styles.engineMetaSep}>·</span>
          {t("aiSettings.engine.detected", { accel: accelLabel })}
        </span>
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
                onClick={() =>
                  void openUrl("https://developer.nvidia.com/cuda-downloads")
                }
              >
                {t("aiSettings.engine.noCuda.linkText")}
              </button>
              {t("aiSettings.engine.noCuda.suffix")}
            </span>
          </div>
        </div>
      ) : null}

      {error ? <div className={styles.engineError}>{error}</div> : null}

      {progress ? <EngineProgress progress={progress} /> : null}

      <div className={styles.engineActions}>
        <button
          type="button"
          className={styles.testBtn}
          onClick={() => void onDownload()}
          disabled={busy}
        >
          {busy ? (
            <Loader2 size={16} strokeWidth={2.2} className={styles.testSpin} />
          ) : (
            <Download size={16} strokeWidth={2.2} />
          )}
          <span>{downloadBtnLabel}</span>
        </button>
        {/* 「测试连接」合并按钮：未启动 → 先 start_engine → 再 test_ai_endpoint */}
        <button
          type="button"
          className={styles.engineTest}
          onClick={() => void onTestLocal()}
          disabled={busy || !installed || engineBusy}
          title={
            installed
              ? t("aiSettings.engine.actions.testTooltipReady")
              : t("aiSettings.engine.actions.testTooltipNotInstalled")
          }
        >
          {engineBusy ? (
            <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
          ) : null}
          {t("aiSettings.engine.actions.testConnection")}
        </button>
        {/* 引擎在跑时才显示「释放显存」：手动停掉 server，下次跑总结会 lazy spawn 自动起。
            disabled 时不显示——避免用户在引擎本来就没跑时点了一脸懵。 */}
        {status.runtime.state === "running" ? (
          <button
            type="button"
            className={styles.engineFolderBtn}
            onClick={() => void onReleaseVram()}
            disabled={engineBusy}
            title={t("aiSettings.engine.actions.releaseVramTooltip")}
          >
            <PowerOff size={14} strokeWidth={1.85} />
            {t("aiSettings.engine.actions.releaseVram")}
          </button>
        ) : null}
        {/* busy 时用 visibility:hidden 保住占位，避免后面的「打开」「卸载」按钮往左跳 */}
        <span
          className={styles.engineSize}
          style={busy ? { visibility: "hidden" } : undefined}
        >
          {t("aiSettings.engine.actions.approxSize", {
            size: Math.round(status.estimatedBytes / 1024 / 1024),
          })}
        </span>
        <button
          type="button"
          className={styles.engineFolderBtn}
          onClick={() => void api.openEngineDir().catch((e) => logError("engine.openDir", e))}
          disabled={busy || !installed}
          title={
            installed
              ? t("aiSettings.engine.actions.openFolderTooltipInstalled")
              : t("aiSettings.engine.actions.openFolderTooltipNotInstalled")
          }
        >
          <FolderOpen size={14} strokeWidth={1.85} />
          {t("common.open")}
        </button>
        <button
          type="button"
          className={styles.engineUninstall}
          onClick={onDelete}
          disabled={busy || !installed}
          title={
            installed
              ? t("aiSettings.engine.actions.uninstallTooltipInstalled")
              : t("aiSettings.engine.actions.uninstallTooltipNotInstalled")
          }
        >
          <Trash2 size={14} strokeWidth={1.85} />
          {t("aiSettings.engine.actions.uninstall")}
        </button>
      </div>

      {installed ? <EngineRuntimeRow status={status} testResult={testResult} /> : null}
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

function EngineProgress({ progress }: { progress: EngineDownloadProgress }) {
  const { t } = useTranslation();
  // 取整 + 单调递增显示：消除小数频繁跳动，并守住「数字只能涨不能退」。
  // 用 ref 不触发额外 render；新值 ≤ 当前 max 就保持显示老值。
  const maxMbRef = useRef(0);
  const currentMb = Math.round(progress.downloaded / 1024 / 1024);
  if (currentMb > maxMbRef.current) maxMbRef.current = currentMb;
  if (progress.phase === "downloading") {
    return (
      <div className={styles.engineProgressWrap}>
        <div className={styles.engineProgressBar}>
          <div
            className={`${styles.engineProgressFill} ${styles.engineProgressFillIndeterminate}`}
          />
        </div>
        <div className={styles.engineProgressText}>
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
  return <div className={styles.engineProgressText}>{label}</div>;
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
 * 测试连接已合并到上方 engineActions（点了会 start → test → stop）。
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
        {testResult.kind === "ok" ? (
          <span
            className={`${styles.engineRuntimeStatus} ${styles.engineRuntimeStatusOk}`}
          >
            <Check size={14} strokeWidth={2.2} />
            {testResult.models.length === 0
              ? t("aiSettings.engine.runtime.connectedNoModels")
              : t("aiSettings.engine.runtime.connectedWithModels", {
                  count: testResult.models.length,
                })}
          </span>
        ) : null}
        {testResult.kind === "fail" ? (
          <span
            className={`${styles.engineRuntimeStatus} ${styles.engineRuntimeStatusFail}`}
          >
            <XCircle size={14} strokeWidth={2.2} />
            {testResult.message}
          </span>
        ) : null}

        {idleHint ? (
          <span className={styles.engineIdleHint}>{idleHint}</span>
        ) : null}

        {badge ? (
          <span
            className={`${styles.engineBadge} ${badge.cls} ${styles.engineRuntimeBadgeRight}`}
          >
            {badge.text}
          </span>
        ) : null}
      </div>

      {isError && rt.error ? (
        <div className={styles.engineRuntimeError}>{rt.error}</div>
      ) : null}
    </div>
  );
}
