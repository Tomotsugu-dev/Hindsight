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
  /** 启动 llama-server 子进程；返回监听端口。
   *  Phase 1B-α 不传模型，会因为缺模型 fail；Phase 1B-β 起会真传值。 */
  startEngine: () => invoke<number>("start_engine"),
  stopEngine: () => invoke<void>("stop_engine"),
};
