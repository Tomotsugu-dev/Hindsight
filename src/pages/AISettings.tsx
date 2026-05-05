import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  Clock,
  Download,
  Filter,
  FolderOpen,
  Image as ImageIcon,
  Info,
  Loader2,
  Play,
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
  type AiConfig,
  type AiSegment,
  type EngineDownloadProgress,
  type EngineStatus,
} from "../api/hindsight";
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
          title="个人简介"
          icon={User}
          info="AI 总结时会带上这段，帮模型更懂你的工作内容与上下文。"
        >
          <Row label="关于你（可选）" block>
            <textarea
              className={styles.textarea}
              value={ai.userBrief}
              onChange={(e) => updateAi({ userBrief: e.target.value })}
              placeholder="例：我是做后端开发的，平时主要写 Rust 和 TypeScript；周末会做点游戏。"
              rows={6}
            />
          </Row>
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

  const onStartEngine = async () => {
    setEngineBusy(true);
    setTestResult({ kind: "idle" });
    try {
      await api.startEngine();
    } catch (e) {
      // start 失败时 supervisor 已经把状态置成 error 并写了 lastError；
      // 我们只需 refresh 把 runtime.error 拉回来给 UI
      console.warn("start_engine 失败：", e);
    } finally {
      await refresh();
      setEngineBusy(false);
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

  const onTestLocal = async () => {
    if (!status?.runtime.port) return;
    setTestResult({ kind: "running" });
    try {
      const r = await api.testAiEndpoint(
        `http://127.0.0.1:${status.runtime.port}/v1`,
      );
      if (r.ok) setTestResult({ kind: "ok", models: r.models });
      else setTestResult({ kind: "fail", message: r.message });
    } catch (e) {
      setTestResult({
        kind: "fail",
        message: e instanceof Error ? e.message : String(e),
      });
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
          onStart={onStartEngine}
          onStop={onStopEngine}
          onTest={onTestLocal}
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
 * 引擎运行时控制行（Phase 1B-α）。
 *
 * 只在 binary 已安装时渲染——未安装根本没什么可控的。
 * 三个状态对应不同 badge + 不同主操作按钮：
 *   stopped  → [▶ 启动引擎]
 *   starting → [⏳ 启动中…] disabled
 *   running  → [⬛ 停止]
 *   error    → [▶ 重试启动]，下方挂错误详情
 *
 * 旁边的"测试连接"按钮只在 running 时可点。
 */
function EngineRuntimeRow({
  status,
  busy,
  testResult,
  onStart,
  onStop,
  onTest,
}: {
  status: EngineStatus;
  busy: boolean;
  testResult: RtTestResult;
  onStart: () => void;
  onStop: () => void;
  onTest: () => void;
}) {
  const rt = status.runtime;
  const isRunning = rt.state === "running";
  const isStarting = rt.state === "starting";
  const isError = rt.state === "error";

  // stopped / starting 时隐藏 badge——按钮文字已经把信息说清楚了，再挂一个 badge 是冗余。
  // running 时 badge 带端口号（按钮没说），error 时 badge 给视觉强提示。
  const badge =
    rt.state === "running"
      ? { text: `已运行 · 端口：${rt.port}`, cls: styles.engineBadgeOk }
      : rt.state === "error"
        ? { text: "出错", cls: styles.engineBadgeFail }
        : null;

  return (
    <div className={styles.engineRuntime}>
      <div className={styles.engineRuntimeRow}>
        {isRunning ? (
          <button
            type="button"
            className={styles.engineStop}
            onClick={onStop}
            disabled={busy}
          >
            <Square size={14} strokeWidth={2} />
            停止
          </button>
        ) : (
          <button
            type="button"
            className={styles.engineStart}
            onClick={onStart}
            disabled={busy || isStarting}
          >
            {isStarting ? (
              <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
            ) : (
              <Play size={14} strokeWidth={2} />
            )}
            {isStarting ? "启动中…" : isError ? "重试启动" : "启动引擎"}
          </button>
        )}

        <button
          type="button"
          className={styles.engineTest}
          onClick={onTest}
          disabled={!isRunning || testResult.kind === "running"}
          title={isRunning ? "向本地引擎打 GET /v1/models" : "引擎未启动"}
        >
          {testResult.kind === "running" ? (
            <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
          ) : null}
          测试连接
        </button>

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
