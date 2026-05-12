import { invoke } from "@tauri-apps/api/core";

export interface HourSegment {
  categoryId: string;
  minutes: number;
}

export interface HourSlot {
  hour: number;
  segments: HourSegment[];
}

export interface AppUsage {
  /** 显示名：组的 display_name（合并组内多个进程名） */
  process: string;
  categoryId: string;
  minutes: number;
  /** AppIcon 用来查图标的代表 process_name；合并组里取一个稳定成员名 */
  iconProcess: string;
}

export interface DaySummaryDto {
  date: string;
  segments: HourSegment[];
}

export interface DaySummary {
  date: Date;
  segments: HourSegment[];
}

export function dtoToDaySummary(dto: DaySummaryDto): DaySummary {
  const [y, m, d] = dto.date.split("-").map((s) => parseInt(s, 10));
  return {
    date: new Date(y, m - 1, d),
    segments: dto.segments,
  };
}

export interface Category {
  id: string;
  name: string;
  color: string;
  icon: string;
  builtin: boolean;
  apps: string[];
}

export interface CategoryInput {
  name: string;
  color: string;
  icon: string;
}

export interface CategoryPatch {
  name?: string;
  color?: string;
  icon?: string;
}

export interface UnclassifiedApp {
  processName: string;
  minutes: number;
  lastSeenAt: string;
}

export interface AppGroupMember {
  processName: string;
  /// 该成员近 7 天累计时长（秒），按 process_name 聚合，跨设备求和
  recentSecs: number;
  /// 该成员最后一次被采集到时所在的设备 ID（用于按设备分列）
  lastDeviceId: string | null;
}

export interface AppGroup {
  id: string;
  displayName: string;
  categoryId: string | null;
  members: AppGroupMember[];
}

export interface CaptureStatus {
  running: boolean;
  todayCount: number;
  lastCaptureAt: string | null;
  lastError: string | null;
}

export interface TimeRange {
  start: string;
  end: string;
}

/** 快速模板总结的统计单元（top app 列表行 / 分类占比行 共用）。 */
export interface QuickUsageEntry {
  /** 显示名：app 是组的 displayName；category 是 categoryId（前端用 useCategories 拼 name） */
  key: string;
  minutes: number;
  /** 占总时长比例 0..=1；分类行用，应用行也填，前端可选不渲染 */
  percent: number;
  /** 仅 top apps 用：分类 id；其它场景空串 */
  categoryId: string;
  /** 仅 top apps 用：AppIcon 查图标用的代表 process_name；其它场景空串 */
  iconProcess: string;
}

/** 一天里的时段桶：night(0–5) / morning(6–11) / afternoon(12–17) / evening(18–23)。 */
export interface QuickDayPart {
  /** "night" / "morning" / "afternoon" / "evening" */
  key: string;
  minutes: number;
  percent: number;
}

/** 周/月报里"最忙/最闲那天"的指针。 */
export interface QuickPeakDay {
  /** "YYYY-MM-DD" */
  date: string;
  minutes: number;
  /** 0..=6 周一起算（跟 chrono::Weekday::num_days_from_monday 对齐） */
  weekday: number;
}

/** 周/月报里逐日时长（按日期升序）。 */
export interface QuickDailyPoint {
  date: string;
  minutes: number;
}

/** 日报维度的快速模板数据。 */
export interface QuickDaySummary {
  /** "YYYY-MM-DD" */
  date: string;
  totalMinutes: number;
  /** 0..=24 */
  activeHours: number;
  /** 0..=23；totalMinutes=0 时为 null */
  peakHour: number | null;
  peakHourMinutes: number;
  dayParts: QuickDayPart[];
  topApps: QuickUsageEntry[];
  categories: QuickUsageEntry[];
}

/** 周报维度。 */
export interface QuickWeekSummary {
  /** 周一日期 "YYYY-MM-DD" */
  weekStart: string;
  /** 周日日期 "YYYY-MM-DD" */
  weekEnd: string;
  totalMinutes: number;
  /** 7 天里有活动的天数 */
  activeDays: number;
  /** 日均（基于 activeDays；activeDays=0 时为 0） */
  dailyAverageMinutes: number;
  peakDay: QuickPeakDay | null;
  dailySeries: QuickDailyPoint[];
  topApps: QuickUsageEntry[];
  categories: QuickUsageEntry[];
}

/** 月报维度。 */
export interface QuickMonthSummary {
  /** 月份的第一天 "YYYY-MM-DD" */
  monthStart: string;
  /** 月份的最后一天 "YYYY-MM-DD" */
  monthEnd: string;
  /** 月内总天数（28-31） */
  totalDays: number;
  totalMinutes: number;
  activeDays: number;
  dailyAverageMinutes: number;
  peakDay: QuickPeakDay | null;
  /** 月内时长最低那天；仅 activeDays >= 2 时返回 */
  quietDay: QuickPeakDay | null;
  dailySeries: QuickDailyPoint[];
  topApps: QuickUsageEntry[];
  categories: QuickUsageEntry[];
}

export interface AiSegment {
  label: string;
  /** 0..=23 */
  startHour: number;
  /** 1..=24（24 = 当日午夜结束） */
  endHour: number;
  /** 用户自定义底色，hex 格式 `#rrggbb`；空字符串 = 走 UI 自动按时段渐变 */
  color: string;
}

export interface TestAiEndpointResp {
  ok: boolean;
  models: string[];
  message: string;
}

/** 本地 llama-server binary 的安装状态。 */
export interface EngineBinaryStatus {
  /** 当前主机对应的 binary 是否已落到磁盘 */
  installed: boolean;
  /** 已安装的 PIN tag；未安装 = null */
  installedVersion: string | null;
  /** Hindsight 当前 PIN 的 llama.cpp 版本 */
  currentPin: string;
  /** 当前主机被路由到的变体 ID（"win-cuda-12.4-x64" 等） */
  platformId: string;
  /** 完整 asset 文件名 */
  assetName: string;
  /** 估算下载体积（字节）；UI 给用户提示 "约 NN MB" 用 */
  estimatedBytes: number;
}

/** llama-server 子进程运行时状态。 */
export interface EngineRuntimeStatus {
  state: "stopped" | "starting" | "running" | "error";
  /** running 时的端口；其它状态 null */
  port: number | null;
  /** error 时的可读错误（stderr 截短）；其它状态 null */
  error: string | null;
  /** running 且空闲时，距 idle watcher 自动 stop 还剩多少秒；in-flight 时为 null */
  idleSecondsRemaining: number | null;
}

/** 系统 VRAM 信息来源——前端按此切换文案。 */
export type VramSource = "discrete" | "unified";

/** 系统总显存信息。
 *  - `discrete` = NVIDIA 独立显存（nvidia-smi 报值）
 *  - `unified`  = Apple Silicon 统一内存 × 0.7（业界惯例可用比例）
 *  CPU-only 机器 / 探测失败时整个对象为 null。 */
export interface VramInfo {
  totalGb: number;
  source: VramSource;
}

/** ONNX Runtime 推理库（embedding 用）安装状态。跟 EngineBinary 是 AI 引擎的两半。 */
export interface EmbeddingRuntimeStatus {
  /** dylib 文件是否已落到磁盘 */
  installed: boolean;
  /** 已安装版本（来自版本标记文件）；未装为 null */
  installedVersion: string | null;
  /** Hindsight 当前 PIN 的 onnxruntime 版本 */
  currentPin: string;
  /** 估算下载体积（字节） */
  estimatedBytes: number;
}

/** binary + onnxruntime + runtime + 子进程保护 + 系统 VRAM 合并；getEngineStatus 返回这个 */
export interface EngineStatus extends EngineBinaryStatus {
  runtime: EngineRuntimeStatus;
  /** onnxruntime 推理库安装状态——跟 binary 是一对，任一未装都视作"AI 引擎未就绪"。 */
  embeddingRuntime: EmbeddingRuntimeStatus;
  /** 子进程保护降级原因；null = 保护正常 */
  protectionDegraded: string | null;
  /** 系统 VRAM；null = 未检测到（CPU-only 机器或 nvidia-smi 不存在） */
  systemVram: VramInfo | null;
}

/** 本地磁盘上的 GGUF 文件条目（可能是主权重，也可能是 mmproj）。 */
export interface ModelEntry {
  /** 文件名（含 .gguf 后缀） */
  filename: string;
  /** 绝对路径，可直接传给删除 / 选中等命令 */
  path: string;
  /** 字节数 */
  sizeBytes: number;
  /** 文件名包含 mmproj → 是 vision 投影，不是主模型 */
  isMmproj: boolean;
}

/** Hindsight 内置推荐 vision LLM。前端按这张表渲染推荐卡片。 */
export interface RecommendedModel {
  displayName: string;
  /** HF 仓库 ID，例如 `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF` */
  repo: string;
  mainFile: string;
  mainBytes: number;
  mmprojFile: string | null;
  mmprojBytes: number;
  /** 品牌 logo URL（HF org 头像直链）；缺失时前端 fallback 到首字母占位。 */
  logoUrl: string | null;
  /** 是否支持 vision 输入。false = 纯文本模型，step 1 toggle 会被禁用。
   *  跟"有没有 mmproj 文件"不是同一回事——由 JSON 维护者按实测显式标定。 */
  vision: boolean;
  /** 品牌标识——前端按这个分组筛选 (Qwen / Google / DeepSeek / OpenAI / Z.AI)；
   *  空串 = 该模型不参与品牌筛选。 */
  brand: string;
}

/** 下载 GGUF 时的进度事件 payload。`file` 字段标识哪个文件（main / mmproj）。 */
export interface ModelDownloadProgress {
  file: string;
  downloaded: number;
  total: number | null;
}

/** 半成品下载条目——给前端渲染"继续"按钮 + 当前已下进度。
 *  `downloadedBytes` 来自 `<file>.partial` 文件大小；`total` 没存（运行时才知道）。 */
export interface PartialDownload {
  filename: string;
  downloadedBytes: number;
}

/** 模型下载进度事件名。前端 listen 这个。 */
export const MODEL_DOWNLOAD_EVENT = "ai://model-download-progress";

/** 下载进度阶段。`downloaded` / `total` 只在 downloading 阶段有意义。 */
export type EngineDownloadPhase = "downloading" | "verifying" | "extracting" | "done";

/** 下载阶段：`engine` = llama.cpp binary、`runtime` = onnxruntime dylib。
 *  download_binary 命令把两阶段串联跑，前端按这个字段切换提示文字
 *  ("下载 AI 引擎中…" / "下载推理库中…")。 */
export type EngineDownloadStage = "engine" | "runtime";

export interface EngineDownloadProgress {
  phase: EngineDownloadPhase;
  downloaded: number;
  total: number | null;
  stage: EngineDownloadStage;
}

/** 后端 emit 进度事件用的事件名。前端 listen 它。 */
export const ENGINE_DOWNLOAD_EVENT = "ai://engine-download-progress";

/** AI 总结进度事件（Phase 1B-γ）。 */
export const SUMMARY_PROGRESS_EVENT = "ai://summary-progress";

/** 周报 precheck 返回的"某天"元数据。 */
export interface WeekPrecheckDay {
  /** "YYYY-MM-DD" */
  date: string;
  /** 按当前 prompt 语言写的星期简写（"周一" / "Mon" / "月"） */
  weekday: string;
  /** 该日是否有 daily ok 段总结 */
  hasDaily: boolean;
  /** 该日是否有活动记录（过滤掉 excluded_categories 后） */
  hasActivity: boolean;
}

/** 周报 precheck 返回 payload。 */
export interface WeekPrecheckResp {
  /** 一周 7 天，按周一到周日顺序 */
  days: WeekPrecheckDay[];
  /** 7 天里有几天有 daily ok 段总结 */
  daysWithDaily: number;
  /** 7 天里有几天 hasActivity = true 但 hasDaily = false */
  daysActivityOnly: number;
}

export type SummaryPhase =
  | "engine_starting"
  | "dedup_running"
  | "segment_started"
  | "image_described"
  | "step1_done"
  | "summarizing"
  | "segment_done"
  | "all_done"
  | "cancelled"
  | "error";

export interface SummaryProgress {
  /** "daily" / "debug"——前端两个 tab 各 listen 自己 source 的事件，避免串台 */
  source: string;
  date: string;
  phase: SummaryPhase;
  segmentIdx: number | null;
  totalSegments: number;
  imagesTotal: number | null;
  /** image_described 时该图在段内的下标（0-based） */
  imageIndex: number | null;
  /** image_described 时附该图绝对路径 */
  imagePath: string | null;
  /** image_described 时附该图的描述文本 */
  imageDescription: string | null;
  /** image_described 时附调用耗时（ms） */
  latencyMs: number | null;
  /** image_described 时附 prompt token 数 */
  promptTokens: number | null;
  /** image_described 时附 completion token 数 */
  completionTokens: number | null;
  /** segment_done 时附该段总结正文（直接是 LLM 输出 markdown），其它阶段为 null */
  content: string | null;
  /** segment_done 时落库行的状态："ok" / "skipped_no_screenshots" / "error" */
  status: SummarySegmentStatus | null;
  /** error / engine_starting 时的提示文字 */
  message: string | null;
}

/** 单次 generate 调用对 settings.ai 的局部覆盖（不写 settings 全局）。
 *  调试 tab 用：跑总结时传这个 patch，跑完不留痕。 */
export interface AiOverrides {
  excludedCategories?: string[];
  maxImagesPerSegment?: number;
  /** 段内余弦阈值去重；范围 0.70..=0.99。详见 [AiConfig.dedupThreshold]。 */
  dedupThreshold?: number;
  /** step 2 段总结的 system prompt 文本覆盖（按当前 promptLanguage 生效） */
  systemPrompt?: string;
  /** step 1 单图描述的 system prompt 文本覆盖（按当前 promptLanguage 生效） */
  imageDescribePrompt?: string;
  /** llama-server `--batch-size` / `--ubatch-size`（取一致值）。
   *  双套参数语义：旧字段是 fallback——`describe*` / `summary*` 未设时降级使用。 */
  batchSize?: number;
  /** llama-server `-np`（并行槽位数）。详见 [batchSize] 关于 fallback 语义。 */
  parallelSlots?: number;
  /** 每个 slot 的 ctx 上限（token）。详见 [batchSize] 关于 fallback 语义。 */
  ctxSize?: number;
  /** 图描述阶段的 batch；`undefined` = fallback 到 [batchSize]。 */
  describeBatchSize?: number;
  /** 图描述阶段的 `-np`；`undefined` = fallback 到 [parallelSlots]。 */
  describeParallelSlots?: number;
  /** 图描述阶段的每槽 ctx；`undefined` = fallback 到 [ctxSize]。 */
  describeCtxSize?: number;
  /** 段总结阶段的 batch；`undefined` = fallback 到 [batchSize]。 */
  summaryBatchSize?: number;
  /** 段总结阶段的 `-np`（推荐恒为 1）；`undefined` = fallback 到 [parallelSlots]。 */
  summaryParallelSlots?: number;
  /** 段总结阶段的每槽 ctx；`undefined` = fallback 到 [ctxSize]。 */
  summaryCtxSize?: number;
  /** 本次跑段总结走云端 (true) 还是本地 (false)。`undefined` = 沿用 settings.ai.externalEnabled。
   *  endpoint / model / apiKey 永远沿用 settings 全局值——这里只控制路径选择。 */
  externalEnabled?: boolean;
}

/** ai_image_descriptions 表的一行——两步生成 step 1 的产物，给调试 tab 渲染。 */
export interface ImageDescriptionRow {
  /** "daily" / "debug" — 跟段总结的 source 同义 */
  source: string;
  localDate: string;
  segmentIdx: number;
  /** 该段抽帧后的 0-based 顺序 */
  imageIndex: number;
  /** 截图绝对路径 */
  screenshotPath: string;
  /** LLM 输出的描述文本 */
  description: string;
  /** 生成时用的 active_main 文件名 */
  model: string;
  generatedAt: string;
  /** 单张图调用 LLM 的总耗时（ms）；llama-server 没返时 null */
  latencyMs: number | null;
  /** 上下文 token 数；llama-server 没返时 null */
  promptTokens: number | null;
  /** 输出 token 数；同上 */
  completionTokens: number | null;
}

/** 段总结落库状态。 */
export type SummarySegmentStatus = "ok" | "skipped_no_screenshots" | "error";

/** ai_summaries 表的一行，前端拿来渲染 SegmentSummaryCard。 */
export interface SegmentSummaryRow {
  /** "daily"（DailyTab 写读）/ "debug"（DebugTab 写读）；PK 含 source */
  source: string;
  localDate: string;
  segmentIdx: number;
  label: string;
  startHour: number;
  endHour: number;
  /** LLM 输出的 markdown 段落；status != 'ok' 时是空串 */
  content: string;
  /** 生成时用的 active_main 文件名（换模型不擦旧总结） */
  model: string;
  status: SummarySegmentStatus;
  /** status='error' 时的可读错误描述；其它状态 null */
  error: string | null;
  /** RFC3339 UTC 时间戳 */
  generatedAt: string;
}

/** AI 子系统的所有用户配置；嵌进 Settings.ai。
 *  字段镜像后端 Rust `crate::ai::config::AiConfig`（camelCase）。 */
export interface AiConfig {
  /** 外部云端 API base URL（OpenAI 兼容，不含 /chat/completions）。
   *  仅在 externalEnabled = true 时生效。本地 step 1 不用这个。 */
  endpoint: string;
  /** 外部 API 的模型 ID，如 gpt-4o-mini / deepseek-chat。
   *  仅在 externalEnabled = true 时生效。 */
  model: string;
  /** 外部 API 的 Bearer token；明文落 settings JSON。 */
  apiKey: string;
  /** 是否启用云端 API 跑 step 2 段总结。
   *  false = 全程本地；true = step 1 本地 vision，step 2 走 endpoint/model/apiKey。
   *  截图永远只在 step 1 经手，不上云。 */
  externalEnabled: boolean;
  /** Provider 预设 ID："openai" / "deepseek" / "openrouter" / "together" / "groq" / "custom"。
   *  仅控前端 Base URL / Model placeholder；后端只 sanitize。 */
  externalProvider: string;
  /** 用户对自己的简短描述，AI 总结时拼进 system prompt */
  userBrief: string;
  /** 一天的时段划分（按 startHour 排序、相邻段共边） */
  segments: AiSegment[];
  /** 不分析的 category id 列表 */
  excludedCategories: string[];
  /** 单段送 AI 的截图上限 */
  maxImagesPerSegment: number;
  /** 截图相似度去重阈值（余弦），段内贪心去重；step 1 vision LLM 之前砍冗余画面。
   *  范围 0.70..=0.99；默认 0.95（POC 验证 ~70% 去重率，肉眼无误删）。
   *  越严越保守（接近 1）；越松越激进，可能误删。 */
  dedupThreshold: number;
  /** 模型（GGUF）保存路径。
   *  空字符串 = 走默认 `<data_root>/ai/models/`；后端 settings load 启动时
   *  会把空值填成实际默认路径，所以前端拿到的总是非空字符串。 */
  modelsPath: string;
  /** 当前选中的主权重 GGUF 文件名（旧版字段；新代码读 `describeMain` / `summaryMain` 的
   *  effective 值，本字段当 fallback 用）。空 = 还没选；启动引擎会拒绝。 */
  activeMain: string;
  /** 当前选中的 mmproj GGUF 文件名（fallback 同上）。空 = 没有。 */
  activeMmproj: string;
  /** step 1（图描述）专用主权重；空 = fallback 到 `activeMain`。 */
  describeMain: string;
  /** step 1 专用 mmproj；空 = fallback 到 `activeMmproj`。 */
  describeMmproj: string;
  /** step 2（段总结）专用主权重；空 = fallback 到 `activeMain`。 */
  summaryMain: string;
  /** step 2 专用 mmproj；空 = fallback 到 `activeMmproj`。一般纯文本模型留空即可。 */
  summaryMmproj: string;
  /** AI 总结使用的提示词语言："zh" / "en" / "ja"。
   *  决定模型用哪种语言写总结，也决定 UI 编辑时显示哪一份覆盖。 */
  promptLanguage: PromptLanguage;
  /** 用户对内置 system prompt（step 2 段总结）的覆盖；按语言独立。
   *  对应字段为空 = 用内置默认。 */
  promptOverrides: PromptOverrides;
  /** 用户对内置 image describe prompt（step 1 单图描述）的覆盖；按语言独立。 */
  imageDescribeOverrides: PromptOverrides;
  /** 引擎启动级参数：`--batch-size` / `--ubatch-size` 一致值。
   *  双套参数语义：这三个旧字段（batchSize/parallelSlots/ctxSize）现在是 fallback——
   *  对应的 describe* / summary* 字段未填时降级使用。详见 describeBatchSize 等。 */
  batchSize: number | null;
  /** 引擎启动级参数：`-np` 并行槽位数。详见 [batchSize] 关于 fallback 语义。 */
  parallelSlots: number | null;
  /** 引擎启动级参数：每 slot 的 ctx 上限（token）。详见 [batchSize] 关于 fallback 语义。 */
  ctxSize: number | null;

  /** 图描述阶段（step 1，多图并行）的 batch；null = fallback 到 [batchSize]。 */
  describeBatchSize: number | null;
  /** 图描述阶段的 `-np` 并行槽数；null = fallback 到 [parallelSlots]。
   *  双套参数的关键差异点——describe 默认推荐高 slots（多图并行）。 */
  describeParallelSlots: number | null;
  /** 图描述阶段的每槽 ctx；null = fallback 到 [ctxSize]。 */
  describeCtxSize: number | null;
  /** 段总结阶段（step 2，单段串行）的 batch；null = fallback 到 [batchSize]。 */
  summaryBatchSize: number | null;
  /** 段总结阶段的 `-np`；null = fallback 到 [parallelSlots]。
   *  推荐恒为 1，给 ctx 让出预算。 */
  summaryParallelSlots: number | null;
  /** 段总结阶段的每槽 ctx；null = fallback 到 [ctxSize]。
   *  双套参数的关键差异点——summary 默认推荐高 ctx（容纳多图描述聚合）。 */
  summaryCtxSize: number | null;
}

export type PromptLanguage = "zh" | "en" | "ja";

export interface PromptOverrides {
  /** 中文 system prompt 覆盖；空 = 用内置默认 */
  systemZh: string;
  systemEn: string;
  systemJa: string;
}

export interface Settings {
  captureEnabled: boolean;
  /** 截图独立开关——关掉只停截图，窗口 / 应用切换记录继续 */
  screenshotEnabled: boolean;
  captureIntervalSeconds: number;
  screenshotPath: string;
  workHoursEnabled: boolean;
  workRanges: TimeRange[];
  autoStart: boolean;
  showWindowOnAutoStart: boolean;
  retentionDays: number;
  /** Google Cloud Console 创建的 Desktop App OAuth 凭证（Drive 同步用） */
  googleClientId: string;
  googleClientSecret: string;
  /** 浏览器过滤：浏览器地址栏 URL 包含其中任意一条（忽略大小写）即跳过截图。
   *  默认装一组常见登录页路径片段。 */
  privacyUrlKeywords: string[];
  /** 应用过滤：应用名或窗口标题包含其中任意一条（忽略大小写）即跳过截图。
   *  默认空，用户自填（如 微信、招商银行）。 */
  privacyAppKeywords: string[];
  /** 关闭按钮（窗口右上角 X）行为：true=隐藏到系统托盘，false=直接退出。 */
  minimizeToTray: boolean;
  /** 是否自动检查应用更新 */
  autoUpdateEnabled: boolean;
  /** 自动检查频率：daily / weekly / monthly / onstartup */
  autoUpdateInterval: "daily" | "weekly" | "monthly" | "onstartup";
  /** 上次检查更新的 RFC3339 时间戳；从未查过则为 null */
  lastUpdateCheckAt: string | null;
  /** 用户多久不动鼠键就算"挂机"：超过这个秒数 capture 不再延续会话，
   *  避免离开电脑后还在累计使用时长。0 = 关闭挂机检测。
   *  UI 按分钟展示，值进出后端时由调用方做秒↔分钟转换。 */
  idleThresholdSeconds: number;
  /** AI 总结相关配置（端点、模型、时段、过滤、抽帧参数）。
   *  嵌套结构而不是平铺，跟后端 Settings.ai 对齐；
   *  更新某个子字段时调用方必须 spread 旧 ai：
   *    update({ ai: { ...settings.ai, model: v } })
   *  否则 useSettings.update 的浅合并会把其他子字段擦掉，
   *  后端 #[serde(default)] 又会把缺失字段填回默认值——双重擦除。 */
  ai: AiConfig;
}

export type SettingsPatch = Partial<Settings>;

export interface StorageInfo {
  dbBytes: number;
  screenshotsBytes: number;
  dbPath: string;
  screenshotsPath: string;
}

export interface DeviceRow {
  deviceId: string;
  displayName: string;
  color: string;
  icon: string;
  os: string | null;
  lastSeenAt: string | null;
  isSelf: boolean;
}

export interface AuthState {
  signedIn: boolean;
  uid: string | null;
  email: string | null;
  /** OAuth 凭证是否齐全（决定登录按钮是否可点） */
  configured: boolean;
  /** 多账号场景下登到了不同账号；前端拿到后应提示用户重启 app 切换 DB */
  requiresRestart?: boolean;
}

export interface SyncStatus {
  running: boolean;
  lastPushedAt: string | null;
  lastPulledAt: string | null;
  lastError: string | null;
  pending: number;
  deadLetter: number;
}

export const api = {
  getDayHours: (dayOffset: number, deviceId?: string) =>
    invoke<HourSlot[]>("get_day_hours", { dayOffset, deviceId }),
  getDayApps: (dayOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_day_apps", { dayOffset, limit, deviceId }),
  getHourApps: (dayOffset: number, hour: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_hour_apps", { dayOffset, hour, limit, deviceId }),
  getWeekDays: (weekOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_week_days", { weekOffset, deviceId }),
  getWeekApps: (weekOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_week_apps", { weekOffset, limit, deviceId }),
  getMonthDays: (monthOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_month_days", { monthOffset, deviceId }),
  getMonthApps: (monthOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_month_apps", { monthOffset, limit, deviceId }),
  /** 拉某天的快速模板总结（纯 SQL 聚合，不调 LLM；瞬时返回）。
   *  跟 AI 总结互补，给没有本地大模型硬件的用户用。 */
  getQuickDaySummary: (dayOffset: number, deviceId?: string) =>
    invoke<QuickDaySummary>("get_quick_day_summary", { dayOffset, deviceId }),
  /** 拉某周的快速模板总结。weekOffset = 0 本周，-1 上周。 */
  getQuickWeekSummary: (weekOffset: number, deviceId?: string) =>
    invoke<QuickWeekSummary>("get_quick_week_summary", { weekOffset, deviceId }),
  /** 拉某月的快速模板总结。monthOffset = 0 本月，-1 上月。 */
  getQuickMonthSummary: (monthOffset: number, deviceId?: string) =>
    invoke<QuickMonthSummary>("get_quick_month_summary", { monthOffset, deviceId }),
  listCategories: () => invoke<Category[]>("list_categories"),
  createCategory: (input: CategoryInput) => invoke<Category>("create_category", { input }),
  updateCategory: (id: string, patch: CategoryPatch) =>
    invoke<void>("update_category", { id, patch }),
  deleteCategory: (id: string) => invoke<void>("delete_category", { id }),
  reorderCategories: (orderedIds: string[]) => invoke<void>("reorder_categories", { orderedIds }),
  assignApp: (processName: string, categoryId: string) =>
    invoke<void>("assign_app_to_category", { processName, categoryId }),
  unassignApp: (processName: string) => invoke<void>("unassign_app", { processName }),
  listUnclassifiedApps: (daysBack?: number) =>
    invoke<UnclassifiedApp[]>("list_unclassified_apps", { daysBack }),
  listAppGroups: () => invoke<AppGroup[]>("list_app_groups"),
  createAppGroup: (displayName: string) => invoke<string>("create_app_group", { displayName }),
  deleteAppGroup: (groupId: string) => invoke<void>("delete_app_group", { groupId }),
  mergeAppGroup: (processName: string, targetGroupId: string) =>
    invoke<void>("merge_app_group", { processName, targetGroupId }),
  unmergeAppGroup: (processName: string) => invoke<void>("unmerge_app_group", { processName }),
  renameAppGroup: (groupId: string, displayName: string) =>
    invoke<void>("rename_app_group", { groupId, displayName }),
  assignAppGroupCategory: (groupId: string, categoryId: string | null) =>
    invoke<void>("assign_app_group_category", { groupId, categoryId }),
  startCapture: () => invoke<void>("start_capture"),
  stopCapture: () => invoke<void>("stop_capture"),
  getCaptureStatus: () => invoke<CaptureStatus>("get_capture_status"),
  getAppIcon: (processName: string) => invoke<string | null>("get_app_icon", { processName }),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) => invoke<Settings>("update_settings", { patch }),
  getStorageInfo: () => invoke<StorageInfo>("get_storage_info"),
  purgeActivities: () => invoke<void>("purge_activities"),
  purgeScreenshots: () => invoke<void>("purge_screenshots"),
  openScreenshotsDir: () => invoke<void>("open_screenshots_dir"),
  getDataRoot: () => invoke<string>("get_data_root"),
  setDataRoot: (path: string) => invoke<void>("set_data_root", { path }),
  listDevices: () => invoke<DeviceRow[]>("list_devices"),
  updateSelfDevice: (name?: string, color?: string, icon?: string) =>
    invoke<DeviceRow>("update_self_device", { name, color, icon }),
  authStatus: () => invoke<AuthState>("auth_status"),
  signInWithGoogle: () => invoke<AuthState>("sign_in_with_google"),
  signOut: () => invoke<void>("sign_out"),
  restartApp: () => invoke<void>("restart_app"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncNow: () => invoke<void>("sync_now"),
  /** 测试 AI 端点连通性：GET {endpoint}/models。
   *  失败不抛 Promise reject，而是 resolve { ok: false, message }，
   *  前端只需检查 ok 字段。 */
  testAiEndpoint: (endpoint: string, apiKey?: string) =>
    invoke<TestAiEndpointResp>("test_ai_endpoint", { endpoint, apiKey }),
  getEngineStatus: () => invoke<EngineStatus>("get_engine_status"),
  /** 触发下载；进度通过 listen(ENGINE_DOWNLOAD_EVENT, ...) 拿。
   *  Promise resolve = 下载 + 校验 + 解压全部成功；reject = 任何一阶段失败。 */
  downloadBinary: () => invoke<void>("download_binary"),
  deleteBinary: () => invoke<void>("delete_binary"),
  openEngineDir: () => invoke<void>("open_engine_dir"),
  /** 拿 llama-server 子进程最近 stderr/stdout（最多 500 行）。调试 tab 用，
   *  看 GPU 加载日志。每次启动会清空 ring，所以拿到的是"本次启动以来"。 */
  getEngineLogs: () => invoke<string[]>("get_engine_logs"),
  /** 启动 llama-server 子进程；返回监听端口。
   *  Phase 1B-α 不传模型，会因为缺模型 fail；Phase 1B-β 起会真传值。 */
  startEngine: () => invoke<number>("start_engine"),
  stopEngine: () => invoke<void>("stop_engine"),
  /** 扫描 settings.ai.modelsPath 下所有 .gguf 文件（main + mmproj 平等列）。
   *  目录不存在或为空都返回 []，不抛错。 */
  listLocalModels: () => invoke<ModelEntry[]>("list_local_models"),
  /** 删除一个本地 GGUF。filename 必须是 basename（不含路径分隔符）。 */
  deleteModel: (filename: string) => invoke<void>("delete_model", { filename }),
  /** Hindsight 内置推荐表，前端拿来渲染推荐卡片。静态数据。 */
  listRecommendedModels: () => invoke<RecommendedModel[]>("list_recommended_models"),
  /** 从 HuggingFace 下载一个 GGUF 文件到 settings.ai.modelsPath。
   *  进度通过 listen(MODEL_DOWNLOAD_EVENT, ...) 拿。
   *  Promise resolve = 整个文件下载完毕；reject = 任何阶段失败（含被 cancel——
   *  错误字符串以 "download cancelled:" 前缀标识，前端可据此区分"暂停"和"真失败"）。
   *  已存在 .partial 时后端会自动发 Range request 续传，调用方无需感知。
   *
   *  `saveAs` 让落盘文件名跟 HF URL 上的文件名解耦——多个 rec 的 mmproj 在 HF 上常常
   *  同名（比如 unsloth 系列都是 mmproj-F16.gguf），前端按 `<mainStem>__<hfName>` 起独立
   *  名字落盘，避免互相覆盖。不传则跟 file 一样。progress event / cancel 都按 saveAs 索引。
   *  返回值是落盘后的完整路径。 */
  downloadModel: (repo: string, file: string, expectedBytes: number, saveAs?: string | null) =>
    invoke<string>("download_model", {
      repo,
      file,
      saveAs: saveAs ?? null,
      expectedBytes,
    }),
  /** 暂停某个进行中的下载——翻 cancel flag。`<file>.partial` 保留，下次再调
   *  downloadModel 同 file 名时会续传。文件没在下载时静默成功（idempotent）。 */
  cancelModelDownload: (file: string) => invoke<void>("cancel_model_download", { file }),
  /** 列扫所有 `<file>.partial` 半成品；用于渲染"继续"按钮 + 已下进度。
   *  目录不存在或没有 partial 时返回 `[]`。 */
  listPartialDownloads: () => invoke<PartialDownload[]>("list_partial_downloads"),
  /** 切换 / 设置当前在用的模型（旧版 API；新代码请用 setStepModel）。写 settings 后会把
   *  在跑的 server 停掉，让用户主动点"启动引擎"按新模型重起。
   *  mmprojFile 传 null 表示没有（纯文本模型）。 */
  setActiveModel: (mainFile: string, mmprojFile: string | null) =>
    invoke<void>("set_active_model", { mainFile, mmprojFile }),
  /** 单独设置 step 1（图描述）或 step 2（段总结）的模型；其它 step 不动。
   *  step 取 "describe" / "summary"；mainFile 空字符串 = 清掉该 step 的覆盖（fallback
   *  到 activeMain）。同时会 stop 在跑的 server。 */
  setStepModel: (step: "describe" | "summary", mainFile: string, mmprojFile: string | null) =>
    invoke<void>("set_step_model", { step, mainFile, mmprojFile }),
  /** 跑某天全部段总结。命令本体异步等到所有段完成才 resolve（或 cancel 后早 return）。
   *  期间通过 listen(SUMMARY_PROGRESS_EVENT, ...) 拿进度事件，前端边跑边渲染。
   *  date 格式 "YYYY-MM-DD"；deviceId 传 null = 多设备聚合；
   *  overrides 是调试 tab 用的局部参数覆盖，传 null = 走 settings.ai 全局值。 */
  generateDaySummary: (
    date: string,
    forceRefresh: boolean,
    deviceId: string | null,
    overrides: AiOverrides | null = null,
    /** "daily"（DailyTab，默认）/ "debug"（DebugTab）—— PK 级隔离两支数据 */
    source: string = "daily",
    /** true = 只跑 step 1（逐图描述），跳过 step 2（段总结）。
     *  调试 tab「仅生成图片描述」按钮用；默认 false 走完整流程。 */
    step1Only: boolean = false,
    /** true = 跳过 step 1，从 DB 读已存的图片描述跑 step 2。
     *  调试 tab「仅生成段总结」按钮用；默认 false 走完整流程。
     *  与 step1Only 互斥（前端不应同时传 true）。 */
    step2Only: boolean = false,
  ) =>
    invoke<void>("generate_day_summary", {
      date,
      forceRefresh,
      deviceId,
      overrides,
      source,
      step1Only,
      step2Only,
    }),
  /** 单段重试——只重跑指定一段，复用已在跑的 server。 */
  retrySummarySegment: (
    date: string,
    segmentIdx: number,
    deviceId: string | null,
    overrides: AiOverrides | null = null,
    source: string = "daily",
  ) =>
    invoke<void>("retry_summary_segment", {
      date,
      segmentIdx,
      deviceId,
      overrides,
      source,
    }),
  /** 设取消标记——下一段循环开头会感知到然后早 return。
   *  已经在路上的单段 LLM 请求**不会**被中断（一段 30-180s 必须跑完）。 */
  cancelDaySummary: () => invoke<void>("cancel_day_summary"),
  /** 拉某天已落库的总结。前端进页面调一次：有就直接渲染，没有就显示"开始总结"按钮。 */
  getDaySummary: (date: string, source: string = "daily") =>
    invoke<SegmentSummaryRow[]>("get_day_summary", { date, source }),
  /** 跑某周的周报。命令本体异步等到 LLM 调用完毕（含 DB 写入）才 resolve；
   *  期间通过 listen(SUMMARY_PROGRESS_EVENT, ...) 拿进度（按 source="weekly" 过滤）。
   *  weekStart 推荐传周一日期 "YYYY-MM-DD"；不是周一时后端自动对齐到当周周一。
   *
   *  allowMissingDays:
   *   - false（默认）= 严格模式：本周一天日报都没有时后端写 error 行让前端引导用户先补日报
   *   - true = 宽松模式：前端已通过 precheckWeekSummary 检测到缺失日并用确认弹框跟用户
   *     确认过——缺失天用当日 top apps 顶替进 prompt，整周缺失但有 activity 时仅基于
   *     整周 + 每日 top apps 做简化分析。 */
  generateWeekSummary: (
    weekStart: string,
    forceRefresh: boolean,
    allowMissingDays: boolean = false,
  ) =>
    invoke<void>("generate_week_summary", {
      weekStart,
      forceRefresh,
      allowMissingDays,
    }),
  /** 周报生成前预览：返回该周 7 天每天的"是否有日报 / 是否有活动"。
   *  前端点击生成时调一次，根据结果决定要不要弹"日报缺失"确认弹框。 */
  precheckWeekSummary: (weekStart: string) =>
    invoke<WeekPrecheckResp>("precheck_week_summary", { weekStart }),
  /** 拉某周已落库的周报；该周无生成时返回 null。weekStart 不是周一时自动对齐。 */
  getWeekSummary: (weekStart: string) =>
    invoke<SegmentSummaryRow | null>("get_week_summary", { weekStart }),
  /** 删除某周已生成的周报行。weekStart 不是周一时自动对齐。 */
  clearWeekSummary: (weekStart: string) => invoke<void>("clear_week_summary", { weekStart }),
  /** 拉某段所有"逐图描述"——调试 tab 渲染列表用。两步生成 step 1 的产物。 */
  getSegmentImageDescriptions: (date: string, segmentIdx: number, source: string = "daily") =>
    invoke<ImageDescriptionRow[]>("get_segment_image_descriptions", {
      date,
      segmentIdx,
      source,
    }),
  /** 拉某天所有段的"逐图描述"——调试 tab 一次性渲染整日时用。 */
  getDayImageDescriptions: (date: string, source: string = "daily") =>
    invoke<ImageDescriptionRow[]>("get_day_image_descriptions", {
      date,
      source,
    }),
  /** 清当天所有 AI 产物：段总结 + 逐图描述。调试 tab 的「删除」按钮调，
   *  给用户在不重跑的情况下手动清历史。 */
  clearDaySummary: (date: string, source: string = "daily") =>
    invoke<void>("clear_day_summary", { date, source }),
  /** 只删当天逐图描述（不动段总结）。调试 tab「逐图描述」Section 删除按钮用。 */
  clearDayImageDescriptions: (date: string, source: string = "daily") =>
    invoke<void>("clear_day_image_descriptions", { date, source }),
  /** 只删当天段总结（不动逐图描述）。调试 tab「段总结结果」Section 删除按钮用。 */
  clearDaySegmentSummaries: (date: string, source: string = "daily") =>
    invoke<void>("clear_day_segment_summaries", { date, source }),
  /** 重跑某段某张图的描述——调试 tab 行末"重跑"按钮用。
   *  不动段总结、其它图描述；期间走 SUMMARY_PROGRESS_EVENT 推一条 image_described。 */
  retrySingleImageDescription: (
    date: string,
    segmentIdx: number,
    imageIndex: number,
    overrides: AiOverrides | null = null,
    source: string = "daily",
  ) =>
    invoke<void>("retry_single_image_description", {
      date,
      segmentIdx,
      imageIndex,
      overrides,
      source,
    }),
};
