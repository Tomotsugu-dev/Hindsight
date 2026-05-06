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
}

/** binary + runtime 合并；getEngineStatus 返回这个 */
export interface EngineStatus extends EngineBinaryStatus {
  runtime: EngineRuntimeStatus;
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
}

/** 下载 GGUF 时的进度事件 payload。`file` 字段标识哪个文件（main / mmproj）。 */
export interface ModelDownloadProgress {
  file: string;
  downloaded: number;
  total: number | null;
}

/** 模型下载进度事件名。前端 listen 这个。 */
export const MODEL_DOWNLOAD_EVENT = "ai://model-download-progress";

/** 下载进度阶段。`downloaded` / `total` 只在 downloading 阶段有意义。 */
export type EngineDownloadPhase =
  | "downloading"
  | "verifying"
  | "extracting"
  | "done";

export interface EngineDownloadProgress {
  phase: EngineDownloadPhase;
  downloaded: number;
  total: number | null;
}

/** 后端 emit 进度事件用的事件名。前端 listen 它。 */
export const ENGINE_DOWNLOAD_EVENT = "ai://engine-download-progress";

/** AI 总结进度事件（Phase 1B-γ）。 */
export const SUMMARY_PROGRESS_EVENT = "ai://summary-progress";

export type SummaryPhase =
  | "engine_starting"
  | "segment_started"
  | "image_described"
  | "segment_done"
  | "all_done"
  | "cancelled"
  | "error";

export interface SummaryProgress {
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
  hashThreshold?: number;
  hashWindowMinutes?: number;
  /** step 2 段总结的 system prompt 文本覆盖（按当前 promptLanguage 生效） */
  systemPrompt?: string;
  /** step 1 单图描述的 system prompt 文本覆盖（按当前 promptLanguage 生效） */
  imageDescribePrompt?: string;
  /** llama-server `--batch-size` / `--ubatch-size`（取一致值）；
   *  改值会触发引擎 stop+start with overrides，调试跑完再 stop 让下次默认重启。 */
  batchSize?: number;
  /** llama-server `-np`（并行槽位数）+ 后端 step 1 image describe 并发数；
   *  两者同时生效才真正并行：-np 为 KV cache 槽预留，前端并发数决定同时发的请求。 */
  parallelSlots?: number;
}

/** ai_image_descriptions 表的一行——两步生成 step 1 的产物，给调试 tab 渲染。 */
export interface ImageDescriptionRow {
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
  /** OpenAI 兼容 base URL；本机 Ollama 默认 http://localhost:11434/v1 */
  endpoint: string;
  /** 模型 ID，例如 minicpm-v:8b */
  model: string;
  /** 可选 Bearer token；Ollama 不用填 */
  apiKey: string;
  /** 用户对自己的简短描述，AI 总结时拼进 system prompt */
  userBrief: string;
  /** 一天的时段划分（按 startHour 排序、相邻段共边） */
  segments: AiSegment[];
  /** 不分析的 category id 列表 */
  excludedCategories: string[];
  /** 单段送 AI 的截图上限 */
  maxImagesPerSegment: number;
  /** dHash 64bit 汉明距离阈值（去重） */
  hashThreshold: number;
  /** 哈希聚类时间窗（分钟）；只在窗内的截图之间比相似度 */
  hashWindowMinutes: number;
  /** 模型（GGUF）保存路径。
   *  空字符串 = 走默认 `<data_root>/ai/models/`；后端 settings load 启动时
   *  会把空值填成实际默认路径，所以前端拿到的总是非空字符串。 */
  modelsPath: string;
  /** 当前选中的主权重 GGUF 文件名（在 `modelsPath` 目录下）。
   *  空 = 还没选；启动引擎会拒绝。 */
  activeMain: string;
  /** 当前选中的 mmproj GGUF 文件名（vision 模型必带）。空 = 没有。 */
  activeMmproj: string;
  /** AI 总结使用的提示词语言："zh" / "en" / "ja"。
   *  决定模型用哪种语言写总结，也决定 UI 编辑时显示哪一份覆盖。 */
  promptLanguage: PromptLanguage;
  /** 用户对内置 system prompt（step 2 段总结）的覆盖；按语言独立。
   *  对应字段为空 = 用内置默认。 */
  promptOverrides: PromptOverrides;
  /** 用户对内置 image describe prompt（step 1 单图描述）的覆盖；按语言独立。 */
  imageDescribeOverrides: PromptOverrides;
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
  getWeekDays: (weekOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_week_days", { weekOffset, deviceId }),
  getWeekApps: (weekOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_week_apps", { weekOffset, limit, deviceId }),
  getMonthDays: (monthOffset: number, deviceId?: string) =>
    invoke<DaySummaryDto[]>("get_month_days", { monthOffset, deviceId }),
  getMonthApps: (monthOffset: number, limit?: number, deviceId?: string) =>
    invoke<AppUsage[]>("get_month_apps", { monthOffset, limit, deviceId }),
  listCategories: () => invoke<Category[]>("list_categories"),
  createCategory: (input: CategoryInput) =>
    invoke<Category>("create_category", { input }),
  updateCategory: (id: string, patch: CategoryPatch) =>
    invoke<void>("update_category", { id, patch }),
  deleteCategory: (id: string) => invoke<void>("delete_category", { id }),
  reorderCategories: (orderedIds: string[]) =>
    invoke<void>("reorder_categories", { orderedIds }),
  assignApp: (processName: string, categoryId: string) =>
    invoke<void>("assign_app_to_category", { processName, categoryId }),
  unassignApp: (processName: string) =>
    invoke<void>("unassign_app", { processName }),
  listUnclassifiedApps: (daysBack?: number) =>
    invoke<UnclassifiedApp[]>("list_unclassified_apps", { daysBack }),
  listAppGroups: () => invoke<AppGroup[]>("list_app_groups"),
  createAppGroup: (displayName: string) =>
    invoke<string>("create_app_group", { displayName }),
  deleteAppGroup: (groupId: string) =>
    invoke<void>("delete_app_group", { groupId }),
  mergeAppGroup: (processName: string, targetGroupId: string) =>
    invoke<void>("merge_app_group", { processName, targetGroupId }),
  unmergeAppGroup: (processName: string) =>
    invoke<void>("unmerge_app_group", { processName }),
  renameAppGroup: (groupId: string, displayName: string) =>
    invoke<void>("rename_app_group", { groupId, displayName }),
  assignAppGroupCategory: (groupId: string, categoryId: string | null) =>
    invoke<void>("assign_app_group_category", { groupId, categoryId }),
  startCapture: () => invoke<void>("start_capture"),
  stopCapture: () => invoke<void>("stop_capture"),
  getCaptureStatus: () => invoke<CaptureStatus>("get_capture_status"),
  getAppIcon: (processName: string) =>
    invoke<string | null>("get_app_icon", { processName }),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<Settings>("update_settings", { patch }),
  getStorageInfo: () => invoke<StorageInfo>("get_storage_info"),
  purgeActivities: () => invoke<void>("purge_activities"),
  purgeScreenshots: () => invoke<void>("purge_screenshots"),
  openScreenshotsDir: () => invoke<void>("open_screenshots_dir"),
  getDataRoot: () => invoke<string>("get_data_root"),
  setDataRoot: (path: string) => invoke<void>("set_data_root", { path }),
  listDevices: () => invoke<DeviceRow[]>("list_devices"),
  updateSelfDevice: (
    name?: string,
    color?: string,
    icon?: string,
  ) => invoke<DeviceRow>("update_self_device", { name, color, icon }),
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
  deleteModel: (filename: string) =>
    invoke<void>("delete_model", { filename }),
  /** Hindsight 内置推荐表，前端拿来渲染推荐卡片。静态数据。 */
  listRecommendedModels: () =>
    invoke<RecommendedModel[]>("list_recommended_models"),
  /** 从 HuggingFace 下载一个 GGUF 文件到 settings.ai.modelsPath。
   *  进度通过 listen(MODEL_DOWNLOAD_EVENT, ...) 拿。
   *  Promise resolve = 整个文件下载完毕；reject = 任何阶段失败。
   *  返回值是落盘后的完整路径。 */
  downloadModel: (repo: string, file: string, expectedBytes: number) =>
    invoke<string>("download_model", { repo, file, expectedBytes }),
  /** 切换 / 设置当前在用的模型。写 settings 后会把在跑的 server 停掉，
   *  让用户主动点"启动引擎"按新模型重起。
   *  mmprojFile 传 null 表示没有（纯文本模型）。 */
  setActiveModel: (mainFile: string, mmprojFile: string | null) =>
    invoke<void>("set_active_model", { mainFile, mmprojFile }),
  /** 跑某天全部段总结。命令本体异步等到所有段完成才 resolve（或 cancel 后早 return）。
   *  期间通过 listen(SUMMARY_PROGRESS_EVENT, ...) 拿进度事件，前端边跑边渲染。
   *  date 格式 "YYYY-MM-DD"；deviceId 传 null = 多设备聚合；
   *  overrides 是调试 tab 用的局部参数覆盖，传 null = 走 settings.ai 全局值。 */
  generateDaySummary: (
    date: string,
    forceRefresh: boolean,
    deviceId: string | null,
    overrides: AiOverrides | null = null,
  ) =>
    invoke<void>("generate_day_summary", {
      date,
      forceRefresh,
      deviceId,
      overrides,
    }),
  /** 单段重试——只重跑指定一段，复用已在跑的 server。 */
  retrySummarySegment: (
    date: string,
    segmentIdx: number,
    deviceId: string | null,
    overrides: AiOverrides | null = null,
  ) =>
    invoke<void>("retry_summary_segment", {
      date,
      segmentIdx,
      deviceId,
      overrides,
    }),
  /** 设取消标记——下一段循环开头会感知到然后早 return。
   *  已经在路上的单段 LLM 请求**不会**被中断（一段 30-180s 必须跑完）。 */
  cancelDaySummary: () => invoke<void>("cancel_day_summary"),
  /** 拉某天已落库的总结。前端进页面调一次：有就直接渲染，没有就显示"开始总结"按钮。 */
  getDaySummary: (date: string) =>
    invoke<SegmentSummaryRow[]>("get_day_summary", { date }),
  /** 拉某段所有"逐图描述"——调试 tab 渲染列表用。两步生成 step 1 的产物。 */
  getSegmentImageDescriptions: (date: string, segmentIdx: number) =>
    invoke<ImageDescriptionRow[]>("get_segment_image_descriptions", {
      date,
      segmentIdx,
    }),
  /** 拉某天所有段的"逐图描述"——调试 tab 一次性渲染整日时用。 */
  getDayImageDescriptions: (date: string) =>
    invoke<ImageDescriptionRow[]>("get_day_image_descriptions", { date }),
  /** 重跑某段某张图的描述——调试 tab 行末"重跑"按钮用。
   *  不动段总结、其它图描述；期间走 SUMMARY_PROGRESS_EVENT 推一条 image_described。 */
  retrySingleImageDescription: (
    date: string,
    segmentIdx: number,
    imageIndex: number,
    overrides: AiOverrides | null = null,
  ) =>
    invoke<void>("retry_single_image_description", {
      date,
      segmentIdx,
      imageIndex,
      overrides,
    }),
};
