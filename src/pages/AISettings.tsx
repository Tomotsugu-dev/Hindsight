import { useEffect, useState } from "react";
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
  Square,
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
        <h1 className={styles.title}>AI 设置</h1>
      </header>

      <div className={styles.content}>
        <Section
          title="本地 AI 引擎"
          description="Hindsight 自带 llama.cpp 推理引擎，在你机器上本地跑——截图不上传任何外部服务。"
          icon={Server}
        >
          <EngineSection />
        </Section>

        <Section
          title="模型"
          description="本地推理用的 vision LLM；GGUF 文件下载自 HuggingFace。"
          icon={Bot}
        >
          <ModelsSection />
        </Section>

        <Section
          title="个人简介"
          icon={User}
          info="AI 总结时会带上这段，帮模型更懂你的工作内容与上下文。"
        >
          {/* hover 整个 Row（含 label）或 focus textarea 时才展开 textarea。
              Row label 一直可见，避免折叠态用户看不出这块是什么。 */}
          <div className={styles.briefHover}>
            <Row label="关于你（可选）" block>
              <div className={styles.briefCell}>
                <textarea
                  className={`${styles.textarea} ${styles.briefTextarea}`}
                  value={ai.userBrief}
                  onChange={(e) => updateAi({ userBrief: e.target.value })}
                  placeholder="例：我是做后端开发的，平时主要写 Rust 和 TypeScript；周末会做点游戏。"
                  rows={6}
                />
              </div>
            </Row>
          </div>
        </Section>

        <Section
          title="提示词"
          icon={MessageSquareText}
          description="告诉模型怎么写总结。三种语言各有内置默认；改完点保存生效，点重置回默认。"
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
          title="时段划分"
          icon={Clock}
          info="AI 按段汇总；段内截图按相似度抽帧再发给模型。"
        >
          <Row label="时段" block>
            <SegmentList
              segments={ai.segments}
              onChange={(next: AiSegment[]) => updateAi({ segments: next })}
            />
          </Row>
        </Section>

        <Section title="过滤" icon={Filter}>
          <Row
            label="不分析这些分类"
            labelHint={
              "点击切换：\n" +
              "• 彩色 + 分类图标 = 参与 AI 分析\n" +
              "• 灰色空心 + 闭眼图标 = 已排除"
            }
            block
          >
            <CategoryChipMultiSelect
              selectedIds={ai.excludedCategories}
              onChange={(next) => updateAi({ excludedCategories: next })}
            />
          </Row>
        </Section>

        <Section
          title="抽帧参数"
          icon={ImageIcon}
          description="一段时间内截图很多，先按相似度去重再选送给模型，省时省 token。"
        >
          <Row
            label="相似度阈值"
            labelHint={
              "dHash 64 位汉明距离\n" +
              "• 越小越严格（同一画面才算重复）\n" +
              "• 5 通常合适\n" +
              "• 0 = 像素级一致才去重"
            }
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
            label="时间窗"
            labelHint={
              "只在窗口内的截图之间比相似度。\n" +
              "避免把不同时间段的相似画面（如同一应用上午 / 下午）误合并。"
            }
          >
            <Slider
              value={ai.hashWindowMinutes}
              onChange={(v) => updateAi({ hashWindowMinutes: v })}
              min={0}
              max={30}
              step={1}
              suffix="分钟"
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
    if (!confirm("卸载本地 AI 引擎？模型文件不受影响。")) return;
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

  const onStopEngine = async () => {
    setEngineBusy(true);
    try {
      await api.stopEngine();
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
        console.warn("test 后 stop 失败:", e);
      }
      await refresh();
      setEngineBusy(false);
    }
  };

  if (!status) {
    return <div className={styles.engineCard}>加载中…</div>;
  }

  const installed = status.installed;
  const accelLabel = humanAccelLabel(status.platformId);
  const version = installed ? status.installedVersion : status.currentPin;
  const stale =
    installed &&
    status.installedVersion !== null &&
    status.installedVersion !== status.currentPin;
  // Windows 但 CUDA 未检测到：建议先装 NVIDIA CUDA
  const noCudaWarning = status.platformId === "win-cpu-x64";

  return (
    <div className={styles.engineCard}>
      <div className={styles.engineHead}>
        <span
          className={`${styles.engineBadge} ${
            installed ? styles.engineBadgeOk : styles.engineBadgeWarn
          }`}
        >
          {installed ? "已安装" : "未安装"}
        </span>
        <span className={styles.engineMeta}>
          llama.cpp
          <span
            className={styles.engineInfoWrap}
            tabIndex={0}
            aria-label={`版本 ${version ?? "?"}${stale ? `（最新 ${status.currentPin}）` : ""}`}
          >
            <Info
              size={12}
              strokeWidth={2.2}
              className={styles.engineInfoIcon}
            />
            <span className={styles.engineInfoTip} role="tooltip">
              版本 {version ?? "?"}
              {stale ? ` · 最新 ${status.currentPin}` : ""}
            </span>
          </span>
          <span className={styles.engineMetaSep}>·</span>
          检测到 {accelLabel}
        </span>
      </div>

      {noCudaWarning ? (
        <div className={styles.engineWarning}>
          <AlertTriangle size={14} strokeWidth={2.2} />
          <div className={styles.engineWarningBody}>
            <strong>未检测到 NVIDIA CUDA。</strong>
            <span>
              {" "}
              vision LLM 在 CPU 上跑会非常慢。建议先去
              <a
                className={styles.engineWarningLink}
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  void openUrl("https://developer.nvidia.com/cuda-downloads");
                }}
              >
                {" "}NVIDIA 官网安装 CUDA Toolkit
              </a>
              （≥ 12.4），重启 Hindsight 后会自动切到 GPU 加速变体。
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
          <span>
            {busy
              ? installed
                ? "更新中…"
                : "下载中…"
              : installed
                ? stale
                  ? "更新到最新"
                  : "重新下载"
                : "下载 AI 引擎"}
          </span>
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
              ? "启动引擎（如未运行）并向本地 /v1/models 发请求验证"
              : "尚未安装"
          }
        >
          {engineBusy ? (
            <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
          ) : null}
          测试连接
        </button>
        {!busy ? (
          <span className={styles.engineSize}>
            约 {Math.round(status.estimatedBytes / 1024 / 1024)} MB
          </span>
        ) : null}
        <button
          type="button"
          className={styles.engineFolderBtn}
          onClick={() => void api.openEngineDir().catch(console.error)}
          disabled={busy || !installed}
          title={
            installed ? "在文件管理器打开安装目录" : "尚未安装"
          }
        >
          <FolderOpen size={14} strokeWidth={1.85} />
          打开
        </button>
        <button
          type="button"
          className={styles.engineUninstall}
          onClick={() => void onDelete()}
          disabled={busy || !installed}
          title={installed ? "卸载本地 AI 引擎" : "尚未安装"}
        >
          <Trash2 size={14} strokeWidth={1.85} />
          卸载
        </button>
      </div>

      {installed ? (
        <EngineRuntimeRow
          status={status}
          busy={engineBusy}
          testResult={testResult}
          onStop={onStopEngine}
        />
      ) : null}
    </div>
  );
}

function EngineProgress({ progress }: { progress: EngineDownloadProgress }) {
  const mb = (n: number) => (n / 1024 / 1024).toFixed(1);
  if (progress.phase === "downloading") {
    const pct =
      progress.total !== null && progress.total > 0
        ? (progress.downloaded / progress.total) * 100
        : null;
    return (
      <div className={styles.engineProgressWrap}>
        <div className={styles.engineProgressBar}>
          <div
            className={styles.engineProgressFill}
            style={{
              width: pct !== null ? `${pct}%` : "20%",
              animation: pct === null ? "indeterminate 1.4s infinite" : undefined,
            }}
          />
        </div>
        <div className={styles.engineProgressText}>
          {pct !== null ? `${Math.round(pct)}% · ` : ""}
          {mb(progress.downloaded)}
          {progress.total ? ` / ${mb(progress.total)}` : ""} MB
        </div>
      </div>
    );
  }
  // verifying / extracting / done 都没字节进度，给单行文字提示
  const label =
    progress.phase === "verifying"
      ? "校验中…"
      : progress.phase === "extracting"
        ? "解压中…"
        : "✓ 完成";
  return <div className={styles.engineProgressText}>{label}</div>;
}

type RtTestResult =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; models: string[] }
  | { kind: "fail"; message: string };

/**
 * 引擎运行时反馈行：状态徽章 + testResult 输出 + 「停止」按钮（仅 running 时）。
 *
 * 「测试连接」按钮已经合并到上方 engineActions（点了会自动 start 再 test），
 * 这里就不再重复"启动引擎 / 测试连接"控件，只展示结果反馈 + 提供手动停止
 * 释放 VRAM 的入口。
 */
function EngineRuntimeRow({
  status,
  busy,
  testResult,
  onStop,
}: {
  status: EngineStatus;
  busy: boolean;
  testResult: RtTestResult;
  onStop: () => void;
}) {
  const rt = status.runtime;
  const isRunning = rt.state === "running";
  const isError = rt.state === "error";

  const badge =
    rt.state === "running"
      ? { text: `已运行 · 端口：${rt.port}`, cls: styles.engineBadgeOk }
      : rt.state === "starting"
        ? { text: "启动中…", cls: styles.engineBadgeWarn }
        : rt.state === "error"
          ? { text: "出错", cls: styles.engineBadgeFail }
          : null;

  // 没在跑、没在测、没出错——这一行就空了，干脆不渲染避免多一道空白
  const hasContent =
    isRunning || testResult.kind !== "idle" || badge !== null || isError;
  if (!hasContent) return null;

  return (
    <div className={styles.engineRuntime}>
      <div className={styles.engineRuntimeRow}>
        {isRunning ? (
          <button
            type="button"
            className={styles.engineStop}
            onClick={onStop}
            disabled={busy}
            title="停止引擎释放 VRAM；下次「测试连接」会自动重启"
          >
            <Square size={14} strokeWidth={2} />
            停止
          </button>
        ) : null}

        {testResult.kind === "ok" ? (
          <span
            className={`${styles.engineRuntimeStatus} ${styles.engineRuntimeStatusOk}`}
          >
            <Check size={14} strokeWidth={2.2} />
            {testResult.models.length === 0
              ? "已连接（暂无模型加载）"
              : `已连接，${testResult.models.length} 个模型`}
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
        `卸载 ${rec.displayName}？将删除 main 权重${rec.mmprojFile ? " + mmproj 投影" : ""}两个文件，无法撤销。`,
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
                      ? "下载完成后才能折叠"
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
                  {isOpen ? "收起" : `查看更多模型 (${tail.length})`}
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
            aria-label={`HuggingFace ${rec.repo}`}
          >
            <Info
              size={12}
              strokeWidth={2.2}
              className={styles.engineInfoIcon}
            />
            <span className={styles.engineInfoTip} role="tooltip">
              HuggingFace · <code>{rec.repo}</code>
            </span>
          </span>
          <span className={styles.modelCardSize}>~{totalGB.toFixed(1)} GB</span>
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
              {busy ? "下载中…" : "下载"}
            </button>
          ) : active ? (
            <button
              type="button"
              className={styles.modelActivePill}
              onClick={() => onClear()}
              title="点击取消激活，切回'已下载'状态（会停掉在跑的 server）"
            >
              <Check size={14} strokeWidth={2} />
              使用中
            </button>
          ) : (
            <button
              type="button"
              className={styles.modelReadyBtn}
              onClick={() => onUse(rec)}
              title="点击启用此模型（会停掉在跑的 server，等手动重启加载新模型）"
            >
              <HardDrive size={14} strokeWidth={2} />
              已下载
            </button>
          )}
          {/* 卸载放在最右边，跟"本地 AI 引擎"行的卸载按钮同款。
              未装时仍渲染一个 disabled 占位，layout 不抖。 */}
          <button
            type="button"
            className={styles.engineUninstall}
            onClick={() => onUninstall(rec)}
            disabled={!installed || busy}
            title={installed ? "删除本地文件（main + mmproj）" : "尚未安装"}
          >
            <Trash2 size={14} strokeWidth={1.85} />
            卸载
          </button>
        </div>
      </div>
      {busy && activeProgress ? (
        <div className={styles.engineProgressWrap}>
          <div className={styles.engineProgressBar}>
            <div
              className={styles.engineProgressFill}
              style={{
                width: activeProgress.total
                  ? `${(activeProgress.downloaded / activeProgress.total) * 100}%`
                  : "10%",
              }}
            />
          </div>
          <div className={styles.engineProgressText}>
            {activeIsMmproj ? "vision 投影" : "主权重"} ·{" "}
            {(activeProgress.downloaded / 1024 / 1024).toFixed(1)} /
            {activeProgress.total
              ? ` ${(activeProgress.total / 1024 / 1024).toFixed(1)}`
              : " ?"}{" "}
            MB
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** 平台变体 ID → 人话加速类型标签 */
function humanAccelLabel(platformId: string): string {
  switch (platformId) {
    case "win-cuda-12.4-x64":
      return "CUDA 12.4";
    case "win-cuda-13.1-x64":
      return "CUDA 13.1";
    case "win-cpu-x64":
      return "CPU 模式";
    case "macos-arm64":
      return "Apple Silicon · Metal";
    case "macos-x64":
      return "Intel Mac";
    case "ubuntu-x64":
      return "Linux CPU";
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
      <Row label="System Prompt" block>
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
                ? "有未保存的改动"
                : hasOverride
                  ? "已使用自定义提示词"
                  : "正在使用内置默认"}
            </span>
            <button
              type="button"
              className={styles.promptResetBtn}
              onClick={handleReset}
              disabled={draft === DEFAULT_SYSTEM_PROMPTS[language]}
              title="把编辑器内容填回内置默认（要点保存才真正生效）"
            >
              <RotateCcw size={13} strokeWidth={2} />
              重置默认
            </button>
            <button
              type="button"
              className={styles.promptSaveBtn}
              onClick={handleSave}
              disabled={!isDirty}
              title="保存当前语言的覆盖"
            >
              <Save size={13} strokeWidth={2} />
              保存
            </button>
          </div>
        </div>
      </Row>
    </div>
  );
}
