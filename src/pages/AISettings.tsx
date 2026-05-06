import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Bot,
  Check,
  ChevronDown,
  Clock,
  Download,
  Filter,
  FolderOpen,
  HardDrive,
  Image as ImageIcon,
  Info,
  Loader2,
  MessageSquareText,
  RotateCcw,
  Save,
  Server,
  Trash2,
  User,
  XCircle,
} from "lucide-react";
import { Section } from "./Settings/components/Section";
import { Row } from "./Settings/components/Row";
import { Slider } from "./Settings/components/Slider";
import { SegmentList } from "./Settings/components/SegmentList";
import { CategoryChipMultiSelect } from "./Settings/components/CategoryChipMultiSelect";
import {
  api,
  ENGINE_DOWNLOAD_EVENT,
  MODEL_DOWNLOAD_EVENT,
  type AiConfig,
  type AiSegment,
  type EngineDownloadProgress,
  type EngineStatus,
  type ModelDownloadProgress,
  type ModelEntry,
  type PromptLanguage,
  type PromptOverrides,
  type RecommendedModel,
} from "../api/hindsight";
import { DEFAULT_SYSTEM_PROMPTS, overrideKey } from "../lib/aiPrompts";
import { useSettings } from "../state/settings";
import styles from "./AISettings.module.css";

export default function AISettings() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  if (!settings) return null;

  const ai = settings.ai;

  /**
   * 所有 ai 子字段更新都必须走这个 wrapper。
   *
   * 原因：[useSettings.update](../state/settings.tsx) 内部用浅合并
   * `setSettings(prev => ({ ...prev, ...patch }))`。如果直接调
   * `update({ ai: { endpoint: v } })`，settings.ai 整个会被替换成
   * `{ endpoint: v }`，model / segments / 等其他子字段全没了；
   * 后端收到这个 patch 后，#[serde(default)] 会把缺字段填默认值，
   * 把用户已经存好的其他字段彻底擦除。
   *
   * 所以这里 spread 旧 ai 一次，保证发出去的 patch 总是完整 AiConfig。
   */
  const updateAi = (patch: Partial<AiConfig>) => {
    update({ ai: { ...ai, ...patch } });
  };

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("aiSettings.title")}</h1>
      </header>

      <div className={styles.content}>
        <Section
          title={t("aiSettings.engine.sectionTitle")}
          description={t("aiSettings.engine.sectionDesc")}
          icon={Server}
        >
          <EngineSection />
        </Section>

        <Section
          title={t("aiSettings.models.sectionTitle")}
          description={t("aiSettings.models.sectionDesc")}
          icon={Bot}
        >
          <ModelsSection />
        </Section>

        <Section
          title={t("aiSettings.brief.sectionTitle")}
          icon={User}
          info={t("aiSettings.brief.sectionInfo")}
        >
          {/* hover 整个 Row（含 label）或 focus textarea 时才展开 textarea。
              Row label 一直可见，避免折叠态用户看不出这块是什么。 */}
          <div className={styles.briefHover}>
            <Row label={t("aiSettings.brief.rowLabel")} block>
              <div className={styles.briefCell}>
                <textarea
                  className={`${styles.textarea} ${styles.briefTextarea}`}
                  value={ai.userBrief}
                  onChange={(e) => updateAi({ userBrief: e.target.value })}
                  placeholder={t("aiSettings.brief.placeholder")}
                  rows={6}
                />
              </div>
            </Row>
          </div>
        </Section>

        <Section
          title={t("aiSettings.prompt.sectionTitle")}
          icon={MessageSquareText}
          description={t("aiSettings.prompt.sectionDesc")}
        >
          <PromptSection
            language={ai.promptLanguage}
            overrides={ai.promptOverrides}
            onSaveOverride={(lang, text) =>
              updateAi({
                promptOverrides: {
                  ...ai.promptOverrides,
                  [overrideKey(lang)]: text,
                },
              })
            }
          />
        </Section>

        <Section
          title={t("aiSettings.segments.sectionTitle")}
          icon={Clock}
          info={t("aiSettings.segments.sectionInfo")}
        >
          <Row label={t("aiSettings.segments.rowLabel")} block>
            <SegmentList
              segments={ai.segments}
              onChange={(next: AiSegment[]) => updateAi({ segments: next })}
            />
          </Row>
        </Section>

        <Section title={t("aiSettings.filter.sectionTitle")} icon={Filter}>
          <Row
            label={t("aiSettings.filter.rowLabel")}
            labelHint={t("aiSettings.filter.rowHint")}
            block
          >
            <CategoryChipMultiSelect
              selectedIds={ai.excludedCategories}
              onChange={(next) => updateAi({ excludedCategories: next })}
            />
          </Row>
        </Section>

        <Section
          title={t("aiSettings.frame.sectionTitle")}
          icon={ImageIcon}
          description={t("aiSettings.frame.sectionDesc")}
        >
          <Row
            label={t("aiSettings.frame.thresholdLabel")}
            labelHint={t("aiSettings.frame.thresholdHint")}
          >
            <Slider
              value={ai.hashThreshold}
              onChange={(v) => updateAi({ hashThreshold: v })}
              min={0}
              max={32}
              step={1}
            />
          </Row>
          <Row
            label={t("aiSettings.frame.windowLabel")}
            labelHint={t("aiSettings.frame.windowHint")}
          >
            <Slider
              value={ai.hashWindowMinutes}
              onChange={(v) => updateAi({ hashWindowMinutes: v })}
              min={0}
              max={30}
              step={1}
              suffix={t("aiSettings.frame.windowSuffix")}
            />
          </Row>
        </Section>
      </div>
    </div>
  );
}

/**
 * 本地 AI 引擎安装状态卡片（Phase 1B-α）。
 *
 * - mount 时拉一次 [`api.getEngineStatus`]，下载 / 删除后再拉一次
 * - 进度通过 listen [`ENGINE_DOWNLOAD_EVENT`] 实时更新
 * - 下载中显示可视进度条 + 百分比 + MB 数
 *
 * 内部直接渲染一个卡片 div，不再用 Row 套——这块信息天然是一组，
 * 拆 3 个 Row 看着像调试 dump。
 */
function EngineSection() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [progress, setProgress] = useState<EngineDownloadProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 启动 / 停止 / 测试 三个动作的 in-flight 状态
  const [engineBusy, setEngineBusy] = useState(false);
  // 测试连接结果状态
  type TestResult =
    | { kind: "idle" }
    | { kind: "running" }
    | { kind: "ok"; models: string[] }
    | { kind: "fail"; message: string };
  const [testResult, setTestResult] = useState<TestResult>({ kind: "idle" });

  const refresh = async () => {
    try {
      setStatus(await api.getEngineStatus());
    } catch (e) {
      console.error("getEngineStatus 失败:", e);
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

  const onDelete = async () => {
    if (!confirm(t("aiSettings.engine.uninstallConfirm"))) return;
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
        console.warn("test 后 stop 失败:", e);
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
          <span
            className={styles.engineInfoWrap}
            tabIndex={0}
            aria-label={
              stale
                ? t("aiSettings.engine.versionStaleAria", {
                    version: versionDisplay,
                    latest: status.currentPin,
                  })
                : t("aiSettings.engine.versionAria", { version: versionDisplay })
            }
          >
            <Info
              size={12}
              strokeWidth={2.2}
              className={styles.engineInfoIcon}
            />
            <span className={styles.engineInfoTip} role="tooltip">
              {t("aiSettings.engine.versionLabel", { version: versionDisplay })}
              {stale
                ? t("aiSettings.engine.versionStaleLabel", {
                    latest: status.currentPin,
                  })
                : ""}
            </span>
          </span>
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
              <a
                className={styles.engineWarningLink}
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  void openUrl("https://developer.nvidia.com/cuda-downloads");
                }}
              >
                {t("aiSettings.engine.noCuda.linkText")}
              </a>
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
        {/* 「测试连接」合并按钮：未启动 → 先 start_engine → 再 test_ai_endpoint。
            放「重新下载」右边；testResult 状态展示在下方 EngineRuntimeRow 区域。 */}
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
          onClick={() => void api.openEngineDir().catch(console.error)}
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
          onClick={() => void onDelete()}
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

      {installed ? (
        <EngineRuntimeRow status={status} testResult={testResult} />
      ) : null}
    </div>
  );
}

function EngineProgress({ progress }: { progress: EngineDownloadProgress }) {
  const { t } = useTranslation();
  // 取整 + 单调递增显示：消除小数频繁跳动，并守住「数字只能涨不能退」。
  // 用 ref 不触发额外 render；新值 ≤ 当前 max 就保持显示老值。
  const maxMbRef = useRef(0);
  const currentMb = Math.round(progress.downloaded / 1024 / 1024);
  if (currentMb > maxMbRef.current) maxMbRef.current = currentMb;
  // phase 切到 verifying 后下次又回 downloading 时（第二个 zip 开始）重置：
  // 这里不重置——combined 累计应该贯穿两个文件。
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

type RtTestResult =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; models: string[] }
  | { kind: "fail"; message: string };

/**
 * 引擎运行时反馈行：状态徽章 + testResult 输出。
 *
 * 「测试连接」按钮已经合并到上方 engineActions（点了会 start → test → stop），
 * 「停止」入口也移除——所有路径都自带停止：
 *   - 测试连接：自带 stop
 *   - AI 总结跑完：可在总结页用「停止」按钮中断
 *   - 应用退出：钩子会 kill 子进程
 */
function EngineRuntimeRow({
  status,
  testResult,
}: {
  status: EngineStatus;
  testResult: RtTestResult;
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

  // 没在测、没出错、状态条又能直接看 → 不渲染额外行，避免多一道空白
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

        {/* 状态 badge 推到行右端：margin-left: auto 吸到末尾 */}
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

/**
 * 模型管理 Section（Phase 1B-β）。
 *
 * 顶部展示 Hindsight 内置推荐卡片（HF 一键下载）；下面是用户已下载的本地
 * .gguf 文件清单 + 删除入口。
 *
 * "已安装"判定：推荐里的 main + （如有）mmproj 文件名都能在本地清单里
 * 找到。失败的下载留下半成品（.partial），ai/models.rs 里 download
 * 函数失败时已经清理过，所以本地清单看到的都是完整文件。
 */
function ModelsSection() {
  const { t } = useTranslation();
  // settings 用来读 activeMain：决定哪个推荐 / 本地文件在"当前使用"态。
  // reload 是因为 set_active_model 是旁路命令（不走 update_settings 通道），
  // 写完 settings 后前端 SettingsContext 不会自动 refetch，必须手动 reload。
  const { settings, reload } = useSettings();
  const activeMain = settings?.ai.activeMain ?? "";

  const [recommended, setRecommended] = useState<RecommendedModel[]>([]);
  const [local, setLocal] = useState<ModelEntry[]>([]);
  const [progress, setProgress] = useState<
    Record<string, ModelDownloadProgress>
  >({});
  const [busyFiles, setBusyFiles] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  /** "查看更多模型" 是否点开过——展开后看到完整推荐列表 */
  const [showAllRecs, setShowAllRecs] = useState(false);

  const refresh = async () => {
    try {
      const [rec, loc] = await Promise.all([
        api.listRecommendedModels(),
        api.listLocalModels(),
      ]);
      setRecommended(rec);
      setLocal(loc);
    } catch (e) {
      console.error("ModelsSection refresh:", e);
    }
  };

  useEffect(() => {
    void refresh();
    const p = listen<ModelDownloadProgress>(MODEL_DOWNLOAD_EVENT, (ev) => {
      setProgress((prev) => ({ ...prev, [ev.payload.file]: ev.payload }));
    });
    return () => {
      void p.then((unlisten) => unlisten());
    };
  }, []);

  const localFilenames = new Set(local.map((m) => m.filename));
  const isInstalled = (rec: RecommendedModel): boolean => {
    if (!localFilenames.has(rec.mainFile)) return false;
    if (rec.mmprojFile && !localFilenames.has(rec.mmprojFile)) return false;
    return true;
  };

  const onDownloadRecommended = async (rec: RecommendedModel) => {
    const files: { name: string; bytes: number }[] = [
      { name: rec.mainFile, bytes: rec.mainBytes },
    ];
    if (rec.mmprojFile) {
      files.push({ name: rec.mmprojFile, bytes: rec.mmprojBytes });
    }
    setError(null);
    setBusyFiles((prev) => {
      const next = new Set(prev);
      files.forEach((f) => next.add(f.name));
      return next;
    });
    try {
      // 串行下：一个文件下完再开下一个，省网络竞争 + 进度展示更清晰
      for (const f of files) {
        await api.downloadModel(rec.repo, f.name, f.bytes);
        setProgress((prev) => {
          const next = { ...prev };
          delete next[f.name];
          return next;
        });
      }
      // 全部文件下完 → 把这个推荐自动标为当前使用，省一次"使用"点击
      await api.setActiveModel(rec.mainFile, rec.mmprojFile ?? null);
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusyFiles((prev) => {
        const next = new Set(prev);
        files.forEach((f) => next.delete(f.name));
        return next;
      });
    }
  };

  const onUseRecommended = async (rec: RecommendedModel) => {
    setError(null);
    try {
      await api.setActiveModel(rec.mainFile, rec.mmprojFile ?? null);
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /** 取消当前激活——把 activeMain / activeMmproj 清空，同时停在跑的 server。
   *  状态从"使用中"回到"已下载"。 */
  const onClearActive = async () => {
    setError(null);
    try {
      await api.setActiveModel("", null);
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  const onUninstallRecommended = async (rec: RecommendedModel) => {
    if (
      !confirm(
        t("aiSettings.models.uninstallConfirm", {
          name: rec.displayName,
          extra: rec.mmprojFile
            ? t("aiSettings.models.uninstallConfirmExtra")
            : "",
        }),
      )
    ) {
      return;
    }
    setError(null);
    try {
      await api.deleteModel(rec.mainFile);
      if (rec.mmprojFile) {
        await api.deleteModel(rec.mmprojFile);
      }
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };


  return (
    <div className={styles.modelsSection}>
      {error ? <div className={styles.engineError}>{error}</div> : null}

      <div className={styles.modelList}>
        {(() => {
          // 默认只露 top 2，其它藏在"查看更多"按钮后面。
          // 下载中的 tail 卡片必须可见——用户从展开列表点了下载然后又折叠
          // 会看不到进度。"已安装"不强制：用户可以在下方"已下载"列表里看见。
          const VISIBLE_DEFAULT = 2;
          const head = recommended.slice(0, VISIBLE_DEFAULT);
          const tail = recommended.slice(VISIBLE_DEFAULT);
          const tailHasBusy = tail.some((rec) => {
            const mainBusy = busyFiles.has(rec.mainFile);
            const mmprojBusy =
              !!rec.mmprojFile && busyFiles.has(rec.mmprojFile);
            return mainBusy || mmprojBusy;
          });
          const isOpen = showAllRecs || tailHasBusy;

          return (
            <>
              {head.map((rec) => (
                <RecommendedCard
                  key={rec.mainFile}
                  rec={rec}
                  installed={isInstalled(rec)}
                  active={activeMain === rec.mainFile}
                  busyFiles={busyFiles}
                  progress={progress}
                  onDownload={onDownloadRecommended}
                  onUse={onUseRecommended}
                  onClear={onClearActive}
                  onUninstall={onUninstallRecommended}
                />
              ))}

              {/* tail 用 grid-template-rows 0fr↔1fr 的 trick 做高度自适应动画 */}
              {tail.length > 0 ? (
                <div
                  className={`${styles.modelTailWrap} ${
                    isOpen ? styles.modelTailWrapOpen : ""
                  }`}
                  // 动画完成前 tail 内容仍可见（不能 display:none），
                  // 但 collapsed 时把内容隐藏对辅助技术更友好
                  aria-hidden={!isOpen}
                >
                  <div className={styles.modelTailInner}>
                    {tail.map((rec) => (
                      <RecommendedCard
                        key={rec.mainFile}
                        rec={rec}
                        installed={isInstalled(rec)}
                        active={activeMain === rec.mainFile}
                        busyFiles={busyFiles}
                        progress={progress}
                        onDownload={onDownloadRecommended}
                        onUse={onUseRecommended}
                        onClear={onClearActive}
                        onUninstall={onUninstallRecommended}
                      />
                    ))}
                  </div>
                </div>
              ) : null}

              {tail.length > 0 ? (
                <button
                  type="button"
                  className={styles.modelExpandBtn}
                  onClick={() => setShowAllRecs(!showAllRecs)}
                  disabled={tailHasBusy && !showAllRecs}
                  title={
                    tailHasBusy && !showAllRecs
                      ? t("aiSettings.models.expand.busyTooltip")
                      : undefined
                  }
                >
                  <ChevronDown
                    size={14}
                    strokeWidth={2}
                    className={`${styles.modelExpandChevron} ${
                      isOpen ? styles.modelExpandChevronOpen : ""
                    }`}
                  />
                  {isOpen
                    ? t("aiSettings.models.expand.collapse")
                    : t("aiSettings.models.expand.more", {
                        count: tail.length,
                      })}
                </button>
              ) : null}
            </>
          );
        })()}
      </div>

    </div>
  );
}

/**
 * 推荐模型卡片——单行紧凑：左边名字 + 大小 + ⓘ tooltip，右贴齐下载按钮。
 *
 * blurb（"平衡选择..."）+ HuggingFace repo 路径都进 ⓘ 气泡，平时不占地方。
 * 下载中卡片下方追加进度条占满宽。
 */
function RecommendedCard({
  rec,
  installed,
  active,
  busyFiles,
  progress,
  onDownload,
  onUse,
  onClear,
  onUninstall,
}: {
  rec: RecommendedModel;
  installed: boolean;
  /** 是否是当前 settings.ai.activeMain 选中的模型 */
  active: boolean;
  busyFiles: Set<string>;
  progress: Record<string, ModelDownloadProgress>;
  onDownload: (rec: RecommendedModel) => void;
  /** 已下载但未启用时点"已下载"按钮 → 设为 active（变成"使用中"） */
  onUse: (rec: RecommendedModel) => void;
  /** 当前在"使用中"时点击 → 取消激活，变回"已下载" */
  onClear: () => void;
  /** 卸载按钮——删除 main + mmproj 两个本地文件 */
  onUninstall: (rec: RecommendedModel) => void;
}) {
  const { t } = useTranslation();
  const totalGB = (rec.mainBytes + rec.mmprojBytes) / 1024 / 1024 / 1024;
  const mainBusy = busyFiles.has(rec.mainFile);
  const mmprojBusy = !!rec.mmprojFile && busyFiles.has(rec.mmprojFile);
  const busy = mainBusy || mmprojBusy;
  const activeFile = mainBusy
    ? rec.mainFile
    : mmprojBusy
      ? rec.mmprojFile
      : null;
  const activeProgress = activeFile ? progress[activeFile] : null;
  const activeIsMmproj = activeFile === rec.mmprojFile;

  return (
    <div className={styles.modelCard}>
      <div className={styles.modelCardRow}>
        <div className={styles.modelCardLeft}>
          <span className={styles.modelCardName}>{rec.displayName}</span>
          <span
            className={styles.engineInfoWrap}
            tabIndex={0}
            aria-label={t("aiSettings.models.card.hfTooltipAria", {
              repo: rec.repo,
            })}
          >
            <Info
              size={12}
              strokeWidth={2.2}
              className={styles.engineInfoIcon}
            />
            <span className={styles.engineInfoTip} role="tooltip">
              {t("aiSettings.models.card.hfTooltipPrefix")}
              <code>{rec.repo}</code>
            </span>
          </span>
          <span className={styles.modelCardSize}>
            {t("aiSettings.models.card.approxSize", {
              size: totalGB.toFixed(1),
            })}
          </span>
        </div>
        <div className={styles.modelCardRight}>
          {!installed ? (
            <button
              type="button"
              className={styles.testBtn}
              onClick={() => onDownload(rec)}
              disabled={busy}
            >
              {busy ? (
                <Loader2
                  size={14}
                  strokeWidth={2}
                  className={styles.testSpin}
                />
              ) : (
                <Download size={14} strokeWidth={2} />
              )}
              {busy
                ? t("aiSettings.models.card.downloading")
                : t("aiSettings.models.card.download")}
            </button>
          ) : active ? (
            <button
              type="button"
              className={styles.modelActivePill}
              onClick={() => onClear()}
              title={t("aiSettings.models.card.inUseTooltip")}
            >
              <Check size={14} strokeWidth={2} />
              {t("aiSettings.models.card.inUse")}
            </button>
          ) : (
            <button
              type="button"
              className={styles.modelReadyBtn}
              onClick={() => onUse(rec)}
              title={t("aiSettings.models.card.readyTooltip")}
            >
              <HardDrive size={14} strokeWidth={2} />
              {t("aiSettings.models.card.ready")}
            </button>
          )}
          {/* 卸载放在最右边，跟"本地 AI 引擎"行的卸载按钮同款。
              未装时仍渲染一个 disabled 占位，layout 不抖。 */}
          <button
            type="button"
            className={styles.engineUninstall}
            onClick={() => onUninstall(rec)}
            disabled={!installed || busy}
            title={
              installed
                ? t("aiSettings.models.card.uninstallTooltipInstalled")
                : t("aiSettings.models.card.uninstallTooltipNotInstalled")
            }
          >
            <Trash2 size={14} strokeWidth={1.85} />
            {t("aiSettings.engine.actions.uninstall")}
          </button>
        </div>
      </div>
      {busy && activeProgress ? (
        <div className={styles.engineProgressWrap}>
          <div className={styles.engineProgressBar}>
            {/* 跟引擎下载条同款：固定 20% 宽 + indeterminate 动画来回滑动；
                百分比不靠谱（后端 main + mmproj 切换时 downloaded/total 会跳），
                文字显示已下 MB 让用户看到在涨。 */}
            <div
              className={`${styles.engineProgressFill} ${styles.engineProgressFillIndeterminate}`}
            />
          </div>
          <div className={styles.engineProgressText}>
            {activeIsMmproj
              ? t("aiSettings.models.card.progressMmproj")
              : t("aiSettings.models.card.progressMain")}{" "}
            ·{" "}
            {(activeProgress.downloaded / 1024 / 1024).toFixed(1)} /
            {activeProgress.total
              ? ` ${(activeProgress.total / 1024 / 1024).toFixed(1)}`
              : ` ${t("aiSettings.models.card.progressUnknownTotal")}`}{" "}
            {t("aiSettings.models.card.progressUnit")}
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** 平台变体 ID → 人话加速类型标签 */
function humanAccelLabel(
  platformId: string,
  t: (key: string) => string,
): string {
  switch (platformId) {
    case "win-cuda-12.4-x64":
      return t("aiSettings.engine.accel.cuda12");
    case "win-cuda-13.1-x64":
      return t("aiSettings.engine.accel.cuda13");
    case "win-cpu-x64":
      return t("aiSettings.engine.accel.winCpu");
    case "macos-arm64":
      return t("aiSettings.engine.accel.macArm");
    case "macos-x64":
      return t("aiSettings.engine.accel.macIntel");
    case "ubuntu-x64":
      return t("aiSettings.engine.accel.linuxCpu");
    default:
      return platformId;
  }
}

/**
 * AI 提示词编辑器（Phase 1B-γ+）。
 *
 * 三种语言各独立维护一份覆盖：用户切语言时不会丢之前在别的语言写的覆盖。
 * 编辑器有"未保存改动"指示——避免用户切语言 / 关页时无声丢失改动。
 *
 * 数据流：
 *   props.overrides[langKey] 非空 → 走覆盖；否则展示内置默认（DEFAULT_SYSTEM_PROMPTS）
 *   保存 → onSaveOverride(lang, text)；text="" 等价"删除覆盖"
 *   重置 → 把 textarea 填回内置默认（不主动保存——给用户审一眼再决定要不要落库）
 */
function PromptSection({
  language,
  overrides,
  onSaveOverride,
}: {
  /** 当前生效的语言；跟随应用全局 i18n 走（目前固定 zh，i18n 接入后会自动切换）。 */
  language: PromptLanguage;
  overrides: PromptOverrides;
  onSaveOverride: (lang: PromptLanguage, text: string) => void;
}) {
  const { t } = useTranslation();
  const persistedFor = (lang: PromptLanguage): string => {
    const ov = overrides[overrideKey(lang)];
    return ov.trim().length > 0 ? ov : DEFAULT_SYSTEM_PROMPTS[lang];
  };

  // textarea 草稿：language 变（i18n 切换）时同步重置成新语言的持久值
  const [draft, setDraft] = useState<string>(() => persistedFor(language));
  useEffect(() => {
    setDraft(persistedFor(language));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  const persisted = persistedFor(language);
  const isDirty = draft !== persisted;
  const hasOverride = overrides[overrideKey(language)].trim().length > 0;

  const handleReset = () => {
    setDraft(DEFAULT_SYSTEM_PROMPTS[language]);
  };

  const handleSave = () => {
    // draft 跟内置默认完全一致 → 存空字符串等价"删除覆盖"
    const text = draft.trim() === DEFAULT_SYSTEM_PROMPTS[language].trim() ? "" : draft;
    onSaveOverride(language, text);
  };

  return (
    <div className={styles.promptWrap}>
      <Row label={t("aiSettings.prompt.rowLabel")} block>
        {/* Row.control 默认是 row flex；用 promptStack 改成 column，
            让 textarea 和按钮行各占一行而不是挤在同一行 */}
        <div className={styles.promptStack}>
          <textarea
            className={styles.promptTextarea}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            rows={14}
            spellCheck={false}
          />
          <div className={styles.promptActions}>
            <span className={styles.promptHint}>
              {isDirty
                ? t("aiSettings.prompt.hint.dirty")
                : hasOverride
                  ? t("aiSettings.prompt.hint.custom")
                  : t("aiSettings.prompt.hint.default")}
            </span>
            <button
              type="button"
              className={styles.promptResetBtn}
              onClick={handleReset}
              disabled={draft === DEFAULT_SYSTEM_PROMPTS[language]}
              title={t("aiSettings.prompt.actions.resetTooltip")}
            >
              <RotateCcw size={13} strokeWidth={2} />
              {t("aiSettings.prompt.actions.reset")}
            </button>
            <button
              type="button"
              className={styles.promptSaveBtn}
              onClick={handleSave}
              disabled={!isDirty}
              title={t("aiSettings.prompt.actions.saveTooltip")}
            >
              <Save size={13} strokeWidth={2} />
              {t("aiSettings.prompt.actions.save")}
            </button>
          </div>
        </div>
      </Row>
    </div>
  );
}
