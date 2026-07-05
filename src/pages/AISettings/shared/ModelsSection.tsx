import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { BRAND_LOGOS } from "./brandLogos";
import {
  ArrowUpDown,
  ChevronDown,
  Cloud,
  Cpu,
  Download,
  FolderInput,
  HardDrive,
  Info,
  Loader2,
  Pause,
  Play,
  Plus,
  Tag,
  Trash2,
} from "lucide-react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type ModelDownloadProgress,
  type ModelEntry,
  type RecommendedModel,
} from "../../../api/hindsight";
import {
  cancelModelDownload,
  clearModelDownloadProgress,
  downloadModelDedup,
  getInflightSnapshot,
  getPartialSnapshot,
  getProgressSnapshot,
  refreshPartials,
  subscribeModelDownloads,
} from "../../../state/modelDownloads";
import {
  SimplePicker,
  type SimplePickerOption,
} from "../../../components/SimplePicker/SimplePicker";
import { ConfirmDialog } from "../../../components/ConfirmDialog/ConfirmDialog";
import { useAiSettings } from "./useAiSettings";
import { logError } from "../../../lib/logger";
import styles from "../AISettings.module.css";

/** 能力 chip：cap 字符串 → CSS class。识别不出的 type fallback 到 `capBadgeUnknown`
 *  灰色（JSON 维护者加了新 type 但前端没补色时仍可显示，不至于布局崩）。 */
const CAP_CLASS_MAP: Record<string, string> = {
  TEXT: styles.capBadgeText,
  VISION: styles.capBadgeVision,
  FAST: styles.capBadgeFast,
  BALANCED: styles.capBadgeBalanced,
  R1: styles.capBadgeR1,
  CODE: styles.capBadgeCode,
  REASONING: styles.capBadgeReasoning,
  DEFAULT: styles.capBadgeDefault,
};

/** toolbar 筛选 / 排序的取值；前 3 个跟筛选维度一一对应。 */
type FilterCap = "all" | "vision" | "text";
type FilterStatus = "all" | "installed" | "not-installed";
type SortKey = "default" | "size-asc" | "size-desc" | "name";

/** mmproj 落盘文件名 = `<mainFile_stem>__<hfMmprojName>`。
 *  HF 上不同 rec 的 mmproj 常常同名（unsloth 系列都是 mmproj-F16.gguf），落盘必须给唯一名
 *  避免互相覆盖。HF URL 仍按原 mmprojFile 取——这只是改本地文件名。
 *  没有 mmproj 时返回空串。 */
function mmprojSaveAs(rec: RecommendedModel): string {
  if (!rec.mmprojFile) return "";
  const stem = rec.mainFile.replace(/\.gguf$/i, "");
  return `${stem}__${rec.mmprojFile}`;
}

/** 文件名切成段（去 .gguf 后缀、去开头的 mmproj 前缀），小写。用于本地文件
 *  的 main↔mmproj 自动配对。gemma-4-12B-it-Q4_K_M → [gemma,4,12b,it,q4,k,m]；
 *  mmproj-gemma-4-12B-it-BF16 → [gemma,4,12b,it,bf16]。 */
function fileSegments(name: string): string[] {
  return name
    .replace(/\.gguf$/i, "")
    .replace(/^mmproj[-_]?/i, "")
    .toLowerCase()
    .split(/[-_.]/)
    .filter(Boolean);
}

/** 两个段数组的公共前缀长度。 */
function commonPrefixLen(a: string[], b: string[]): number {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return i;
}

/** 给一个本地主模型文件，从候选 mmproj 里按文件名公共前缀自动配对一个。
 *  要求公共前缀 ≥2 段（如 gemma-4）才算命中，避免把 12B 的 mmproj 配到 31B。
 *  多个候选取公共前缀最长者。找不到返回 null（纯文本模型 / 命名不规范）。 */
function autoPairMmproj(
  mainName: string,
  mmprojs: ModelEntry[],
): ModelEntry | null {
  const ms = fileSegments(mainName);
  let best: ModelEntry | null = null;
  let bestLen = 1; // 需 ≥2 段才配对
  for (const p of mmprojs) {
    const len = commonPrefixLen(ms, fileSegments(p.filename));
    if (len > bestLen) {
      bestLen = len;
      best = p;
    }
  }
  return best;
}

/**
 * 模型管理 Section（Phase 1B-β）。
 *
 * 顶部展示 Hindsight 内置推荐卡片（HF 一键下载）；下面是用户已下载的本地
 * .gguf 文件清单 + 删除入口 + 自定义 HF 表单。被 ModelsTab 包一层 Section 直接渲染。
 */
export function ModelsSection() {
  const { t } = useTranslation();
  // settings 用来读 summaryMain：判断每张推荐卡是不是被选为段总结模型，
  // 在卡上显示对应 chip。reload 让下载完成后能拉到刚刷新的本地文件清单。
  const { settings, reload } = useAiSettings();
  // 注意：raw summary_main 可能是 SUMMARY_CLOUD_SENTINEL（用户在云端卡选了 Text）。
  // 走 fallback 链拿到的字符串可能是 sentinel 或真实文件名；本地卡的 usedForSummary
  // 比较时永远拿 sentinel 跟 rec.mainFile 比，永远不命中——所以本地卡正确显示成"未选"。
  const summaryMain = settings?.ai.summaryMain || settings?.ai.activeMain || "";
  // 云端卡用 RAW 字段判，避免 fallback 链上 activeMain 的干扰。
  const cloudIsSelectedAsSummary =
    (settings?.ai.summaryMain || "") === SUMMARY_CLOUD_SENTINEL;

  const [recommended, setRecommended] = useState<RecommendedModel[]>([]);
  const [local, setLocal] = useState<ModelEntry[]>([]);
  // 下载进度跟 inflight 都提到 module-level（state/modelDownloads.ts），切侧边栏
  // unmount 不会丢；listener 也在那里全局只订阅一次。
  const progress = useSyncExternalStore(
    subscribeModelDownloads,
    getProgressSnapshot,
    getProgressSnapshot,
  );
  const inflightFiles = useSyncExternalStore(
    subscribeModelDownloads,
    getInflightSnapshot,
    getInflightSnapshot,
  );
  const busyFiles = useMemo(() => new Set(inflightFiles), [inflightFiles]);
  // 半成品 map：file → 已下字节数。结合 inflight 一起判断"已暂停"状态：
  // partial 存在 + 不在 inflight = 等用户点继续。
  const partialMap = useSyncExternalStore(
    subscribeModelDownloads,
    getPartialSnapshot,
    getPartialSnapshot,
  );
  const [error, setError] = useState<string | null>(null);
  // 正在从本地磁盘导入（拷贝）中的文件 basename 集合——UI 据此把导入按钮切到进度态。
  // 用落盘名（= 源文件 basename）做 key，跟下载进度事件的 file 字段对齐。
  const [importingFiles, setImportingFiles] = useState<ReadonlySet<string>>(
    new Set<string>(),
  );
  // 自定义 HF 仓库下载表单展开态——默认收起，避免抢推荐卡的视觉重心
  const [showCustom, setShowCustom] = useState(false);
  const [customRepo, setCustomRepo] = useState("");
  const [customMainFile, setCustomMainFile] = useState("");
  const [customMmprojFile, setCustomMmprojFile] = useState("");

  // toolbar 状态：默认 4 项都是"全部 / 默认"，相当于不过滤、不重排
  const [filterCap, setFilterCap] = useState<FilterCap>("all");
  const [filterStatus, setFilterStatus] = useState<FilterStatus>("all");
  const [filterBrand, setFilterBrand] = useState<string>("all");
  const [sortKey, setSortKey] = useState<SortKey>("default");

  // 当前正在 onDownloadRecommended 流程里的 rec.mainFile 集合——必须按 rec 维度跟踪，
  // 不能仅靠 busyFiles：所有 unsloth 镜像的 mmproj 都叫 mmproj-F16.gguf，单看
  // busyFiles.has("mmproj-F16.gguf") 会让所有 vision 卡都判定为 busy，进度条串扰。
  const [confirmingUninstall, setConfirmingUninstall] =
    useState<RecommendedModel | null>(null);
  // 云端选择的隐私确认弹窗：选 Text = 轻警示（上传应用名/分类/活动时间线）。
  // 取消选择不弹。
  const [cloudConfirm, setCloudConfirm] = useState(false);
  const [busyRecs, setBusyRecs] = useState<ReadonlySet<string>>(
    new Set<string>(),
  );

  const refresh = async () => {
    try {
      const [rec, loc] = await Promise.all([
        api.listRecommendedModels(),
        api.listLocalModels(),
      ]);
      setRecommended(rec);
      setLocal(loc);
    } catch (e) {
      logError("models.refresh", e);
    }
  };

  useEffect(() => {
    void refresh();
    // mount 时拉一次 partial 列表——如果上次有没下完的文件，渲染"继续"按钮
    void refreshPartials();
  }, []);

  /** 暂停某文件下载——前端 invoke cancel 后 inflight promise 会以错误 reject，
   *  modelDownloads.ts 的 .finally 会自动 refreshPartials 把 pausedFiles 同步过来。 */
  const onPauseDownload = (filename: string) => {
    void cancelModelDownload(filename);
  };

  const localFilenames = new Set(local.map((m) => m.filename));
  // 已安装只看 main 文件:VLM 移除后全部推理是纯文本,mmproj 不再是使用门槛。
  // 否则用户手动导入的 main(没有改名版 mmproj)会被推荐卡吞掉又判未安装,
  // 既不在"本地模型"区出现、又无法选用——只剩个不可点的卸载按钮。
  const isInstalled = (rec: RecommendedModel): boolean =>
    localFilenames.has(rec.mainFile);

  // 推荐列表占用的落盘文件名（main + mmproj saveAs）。本地文件里不在这个集合的，
  // 就是用户自己导入/手动放进模型目录、但不在推荐列表里的模型——它们不会被任何
  // 推荐卡片承载，必须在下面的"本地模型"区单独展示，否则导入了在 UI 上看不到。
  const recFilenames = useMemo(() => {
    const s = new Set<string>();
    for (const r of recommended) {
      s.add(r.mainFile);
      if (r.mmprojFile) s.add(mmprojSaveAs(r));
    }
    return s;
  }, [recommended]);
  const orphanLocals = local.filter((m) => !recFilenames.has(m.filename));
  const orphanMmprojs = orphanLocals.filter((m) => m.isMmproj);
  const orphanMains = orphanLocals.filter((m) => !m.isMmproj);
  const pairedMmprojNames = new Set<string>();
  // 第一轮：按文件名前缀配对（每次从尚未配走的 mmproj 里挑，避免多主模型抢同一个）。
  const localPairs: { main: ModelEntry; mmproj: ModelEntry | null }[] =
    orphanMains.map((main) => {
      const avail = orphanMmprojs.filter(
        (m) => !pairedMmprojNames.has(m.filename),
      );
      const mmproj = autoPairMmproj(main.filename, avail);
      if (mmproj) pairedMmprojNames.add(mmproj.filename);
      return { main, mmproj };
    });
  // 第二轮兜底：LM Studio 等给 mmproj 用通用名（gemma-3 系列都叫 mmproj-model-f16.gguf，
  // 不含模型标识），文件名前缀配不上。此时若恰好只剩 1 个没配到 mmproj 的主模型 +
  // 1 个未被配走的 mmproj，无歧义地把它们配对。
  const stillUnpaired = localPairs.filter((p) => !p.mmproj);
  const remainMmprojs = orphanMmprojs.filter(
    (m) => !pairedMmprojNames.has(m.filename),
  );
  if (stillUnpaired.length === 1 && remainMmprojs.length === 1) {
    stillUnpaired[0].mmproj = remainMmprojs[0];
    pairedMmprojNames.add(remainMmprojs[0].filename);
  }
  // 没配到任何主模型的孤儿 mmproj：单独列一张卡，只提供删除（无法单独当模型用）。
  const unpairedMmprojs = orphanMmprojs.filter(
    (m) => !pairedMmprojNames.has(m.filename),
  );

  // 品牌选项：从当前 recommended 数据动态收集（去重 + 字典序），避免硬编码——
  // 未来 JSON 加新品牌不需要改前端代码。
  const brandOptions = useMemo<SimplePickerOption<string>[]>(() => {
    const set = new Set<string>();
    for (const r of recommended) {
      if (r.brand) set.add(r.brand);
    }
    const sorted = Array.from(set).sort((a, b) => a.localeCompare(b));
    return [
      { value: "all", label: t("aiSettings.models.toolbar.brandAll") },
      ...sorted.map((b) => ({ value: b, label: b })),
    ];
  }, [recommended, t]);

  /** 应用 filter+sort 之后的展示用列表。"default" 排序保留 JSON 作者意图（"轻→重"）。 */
  const displayed = useMemo<RecommendedModel[]>(() => {
    let list = recommended;
    if (filterCap !== "all") {
      list = list.filter((r) =>
        filterCap === "vision" ? r.vision : !r.vision,
      );
    }
    if (filterStatus !== "all") {
      list = list.filter((r) =>
        filterStatus === "installed" ? isInstalled(r) : !isInstalled(r),
      );
    }
    if (filterBrand !== "all") {
      list = list.filter((r) => r.brand === filterBrand);
    }
    if (sortKey !== "default") {
      list = [...list].sort((a, b) => {
        switch (sortKey) {
          case "size-asc":
            return a.mainBytes + a.mmprojBytes - (b.mainBytes + b.mmprojBytes);
          case "size-desc":
            return b.mainBytes + b.mmprojBytes - (a.mainBytes + a.mmprojBytes);
          case "name":
            return a.displayName.localeCompare(b.displayName);
          default:
            return 0;
        }
      });
    }
    return list;
    // isInstalled 闭包依赖 localFilenames，不入 deps（恒等比较拿不到稳定引用）；
    // 改 local 会触发 recommended 引用变化时一起 re-render，等价
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recommended, filterCap, filterStatus, filterBrand, sortKey, local]);

  const onDownloadRecommended = async (rec: RecommendedModel) => {
    if (busyRecs.has(rec.mainFile)) return; // 防重复点
    // hfName 是 HF URL 上的文件名，saveAs 是落盘的本地名（mmproj 必须独立否则同名覆盖）。
    // main 文件每个 rec 不同，hfName === saveAs；mmproj 用 rec-aware 唯一名。
    const items: { hfName: string; saveAs: string; bytes: number }[] = [
      { hfName: rec.mainFile, saveAs: rec.mainFile, bytes: rec.mainBytes },
    ];
    if (rec.mmprojFile) {
      items.push({
        hfName: rec.mmprojFile,
        saveAs: mmprojSaveAs(rec),
        bytes: rec.mmprojBytes,
      });
    }
    setError(null);
    setBusyRecs((s) => {
      const n = new Set(s);
      n.add(rec.mainFile);
      return n;
    });
    try {
      // 串行下：一个文件下完再开下一个，省网络竞争 + 进度展示更清晰。
      // dedup 按 saveAs 作 key——同 rec 重复点直接复用 promise；不同 rec 的 mmproj
      // 即使 HF 同名（unsloth 系列都叫 mmproj-F16.gguf），saveAs 不同所以不会撞 key。
      // 下完不自动分配 step——用户在卡上点 step 1 / step 2 toggle 显式分配。
      for (const it of items) {
        await downloadModelDedup(rec.repo, it.hfName, it.bytes, it.saveAs);
        clearModelDownloadProgress(it.saveAs);
      }
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusyRecs((s) => {
        const n = new Set(s);
        n.delete(rec.mainFile);
        return n;
      });
    }
  };

  /**
   * 给某个推荐模型 toggle 段总结（summary）分配。
   *
   * 互斥由后端隐式保证（setStepModel 直接覆盖，不累加）；前端这里只负责
   * 区分"当前已在用 → 清空"和"否则 → 切换到该模型"两种情况。
   */
  const onToggleStep = async (rec: RecommendedModel) => {
    setError(null);
    const current = settings?.ai.summaryMain || settings?.ai.activeMain || "";
    try {
      if (current === rec.mainFile) {
        // 已在用 → 清空覆盖（后端 fallback 到 activeMain）
        await api.setStepModel("summary", "", null);
      } else {
        // mmproj 用 saveAs（落盘文件名），且只在本地真的存在时才写——
        // "已安装"只看 main，用户手动导入 main 时 mmproj 多半缺席，
        // 写个不存在的文件名会让引擎启动时加载失败
        const mmprojLocal =
          rec.mmprojFile && localFilenames.has(mmprojSaveAs(rec))
            ? mmprojSaveAs(rec)
            : null;
        await api.setStepModel("summary", rec.mainFile, mmprojLocal);
      }
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /**
   * 云端卡 Text 按钮 toggle：在「选中云端为段总结」和「fallback 回本地」之间切换。
   *
   * 通过把 `summary_main` 写成 [`SUMMARY_CLOUD_SENTINEL`] 来表示选中云端，后端
   * 看到 sentinel + `external_enabled=true` 才路由到 External。
   * 跟本地卡的 toggle 互斥：本地卡 set summary 是真实文件名、覆盖 sentinel；
   * 云端卡 set sentinel、覆盖任何本地文件名。一处状态，互斥自然成立。
   */
  const onToggleCloudSummary = async () => {
    setError(null);
    if (!cloudIsSelectedAsSummary) {
      // 启用方向：先弹隐私确认（应用名/分类/活动时间线将上传），确认后再真正切换
      setCloudConfirm(true);
      return;
    }
    try {
      await api.setStepModel("summary", "", null);
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /** 隐私确认弹窗点了"确认"：真正把 summary 槽位写成云端 sentinel。 */
  const onConfirmCloudSelect = async () => {
    setCloudConfirm(false);
    try {
      await api.setStepModel("summary", SUMMARY_CLOUD_SENTINEL, null);
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  const customMainBusy = !!customMainFile && busyFiles.has(customMainFile);
  const customMmprojBusy =
    !!customMmprojFile && busyFiles.has(customMmprojFile);
  const customBusy = customMainBusy || customMmprojBusy;

  const onDownloadCustom = async () => {
    const repo = customRepo.trim();
    const mainFile = customMainFile.trim();
    const mmprojFile = customMmprojFile.trim();
    if (!repo || !mainFile) {
      setError(t("aiSettings.models.custom.errorRequired"));
      return;
    }
    setError(null);
    try {
      // expectedBytes = 0 关闭容差比对——自定义下载不知道精确字节数
      await downloadModelDedup(repo, mainFile, 0);
      clearModelDownloadProgress(mainFile);
      if (mmprojFile) {
        await downloadModelDedup(repo, mmprojFile, 0);
        clearModelDownloadProgress(mmprojFile);
      }
      await Promise.all([refresh(), reload()]);
      // 下完不再自动设为"在用"；用户去顶部 picker 选 step 1/2。保留输入值方便回看。
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /** 导入本地 GGUF 文件：弹系统文件选择框（可多选），逐个拷进模型目录。
   *  进度复用下载事件通道（modelDownloads store 的全局 listener），所以这里
   *  只需把选中文件的 basename 塞进 importingFiles 让 UI 显示"导入中"。 */
  const onImportLocal = async () => {
    setError(null);
    let picked: string | string[] | null;
    try {
      picked = await openDialog({
        multiple: true,
        filters: [{ name: "GGUF", extensions: ["gguf"] }],
        title: t("aiSettings.models.import.dialogTitle"),
      });
    } catch (e) {
      logError("models.import.dialog", e);
      return;
    }
    if (picked == null) return; // 用户取消
    const paths = Array.isArray(picked) ? picked : [picked];
    if (paths.length === 0) return;

    for (const srcPath of paths) {
      const name = srcPath.split(/[\\/]/).pop() || srcPath;
      setImportingFiles((s) => new Set(s).add(name));
      try {
        await api.importModel(srcPath);
        clearModelDownloadProgress(name);
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      } finally {
        setImportingFiles((s) => {
          const n = new Set(s);
          n.delete(name);
          return n;
        });
      }
    }
    await Promise.all([refresh(), reload()]);
  };

  const onUninstallRecommended = (rec: RecommendedModel) => {
    setConfirmingUninstall(rec);
  };

  const doUninstallRecommended = async (rec: RecommendedModel) => {
    setConfirmingUninstall(null);
    setError(null);
    try {
      await api.deleteModel(rec.mainFile);
      if (rec.mmprojFile) {
        // 删 saveAs 后的真实落盘文件名（不是 HF 上的裸名）
        await api.deleteModel(mmprojSaveAs(rec));
      }
      await refresh();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /** 本地模型卡的 summary toggle：跟推荐卡同逻辑，但 main/mmproj 用本地真实落盘名。
   *  mmproj 为自动配对的结果（纯文本模型为 null，只写 main）。 */
  const onToggleLocalStep = async (
    main: ModelEntry,
    mmproj: ModelEntry | null,
  ) => {
    setError(null);
    const current = settings?.ai.summaryMain || settings?.ai.activeMain || "";
    try {
      if (current === main.filename) {
        await api.setStepModel("summary", "", null);
      } else {
        await api.setStepModel("summary", main.filename, mmproj?.filename ?? null);
      }
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /** 删本地模型：main + 自动配对的 mmproj 一起删（孤儿 mmproj 传 null 只删自己）。 */
  const onDeleteLocal = async (
    main: ModelEntry,
    mmproj: ModelEntry | null,
  ) => {
    setError(null);
    try {
      await api.deleteModel(main.filename);
      if (mmproj) await api.deleteModel(mmproj.filename);
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  // 静态选项常量——能力 / 状态 / 排序的取值固定，i18n 文案随当前 t() 解析
  const capOptions: SimplePickerOption<FilterCap>[] = [
    { value: "all", label: t("aiSettings.models.toolbar.capAll") },
    { value: "vision", label: t("aiSettings.models.toolbar.capVision") },
    { value: "text", label: t("aiSettings.models.toolbar.capText") },
  ];
  const statusOptions: SimplePickerOption<FilterStatus>[] = [
    { value: "all", label: t("aiSettings.models.toolbar.statusAll") },
    {
      value: "installed",
      label: t("aiSettings.models.toolbar.statusInstalled"),
    },
    {
      value: "not-installed",
      label: t("aiSettings.models.toolbar.statusNotInstalled"),
    },
  ];
  const sortOptions: SimplePickerOption<SortKey>[] = [
    { value: "default", label: t("aiSettings.models.toolbar.sortDefault") },
    { value: "size-asc", label: t("aiSettings.models.toolbar.sortSizeAsc") },
    {
      value: "size-desc",
      label: t("aiSettings.models.toolbar.sortSizeDesc"),
    },
    { value: "name", label: t("aiSettings.models.toolbar.sortName") },
  ];

  return (
    <div className={styles.modelsSection}>
      {error ? <div className={styles.engineError}>{error}</div> : null}

      <div className={styles.modelsToolbar}>
        <div
          className={styles.modelsToolbarItem}
          title={t("aiSettings.models.toolbar.capAll")}
        >
          <Cpu
            size={14}
            strokeWidth={2}
            className={styles.modelsToolbarIcon}
          />
          <SimplePicker<FilterCap>
            value={filterCap}
            options={capOptions}
            onChange={setFilterCap}
          />
        </div>
        <div
          className={styles.modelsToolbarItem}
          title={t("aiSettings.models.toolbar.statusAll")}
        >
          <HardDrive
            size={14}
            strokeWidth={2}
            className={styles.modelsToolbarIcon}
          />
          <SimplePicker<FilterStatus>
            value={filterStatus}
            options={statusOptions}
            onChange={setFilterStatus}
          />
        </div>
        <div
          className={styles.modelsToolbarItem}
          title={t("aiSettings.models.toolbar.brandAll")}
        >
          <Tag
            size={14}
            strokeWidth={2}
            className={styles.modelsToolbarIcon}
          />
          <SimplePicker<string>
            value={filterBrand}
            options={brandOptions}
            onChange={setFilterBrand}
          />
        </div>
        <span className={styles.modelsToolbarSpacer} />
        <div className={styles.modelsToolbarItem}>
          <ArrowUpDown
            size={14}
            strokeWidth={2}
            className={styles.modelsToolbarIcon}
          />
          <SimplePicker<SortKey>
            value={sortKey}
            options={sortOptions}
            onChange={setSortKey}
          />
        </div>
      </div>

      <div className={styles.modelList}>
        {/* 云端 API 启用时第一行展示云端卡——表明段总结当前不走本地推荐里的任何
            模型，而是路由到 Cloud API tab 配的 endpoint + model。固定置顶，不参与筛选/
            排序。Text 是可点 toggle，跟本地卡的 summary 是 radio 关系；
            启用方向先过隐私确认弹窗。 */}
        {settings?.ai.externalEnabled ? (
          <div className={`${styles.modelCard} ${styles.modelCardCloud}`}>
            <div className={styles.modelCardRow}>
              <div className={styles.modelCardLeft}>
                <Cloud
                  size={22}
                  strokeWidth={2}
                  className={styles.modelCardCloudIcon}
                />
                <span className={styles.modelCardName}>
                  {t("aiSettings.models.cloud.cardTitle", {
                    provider: settings.ai.externalProvider || "cloud",
                  })}
                </span>
                <span className={styles.modelCardSize}>
                  {settings.ai.model ||
                    t("aiSettings.models.cloud.modelUnset")}
                </span>
              </div>
              <div className={styles.modelCardRight}>
                {/* Text（段总结）：可点击 toggle，跟本地卡的 summary 是 radio 关系 */}
                <button
                  type="button"
                  className={`${styles.modelStepToggle} ${
                    cloudIsSelectedAsSummary ? styles.modelStepToggleActive : ""
                  }`}
                  onClick={onToggleCloudSummary}
                  title={
                    cloudIsSelectedAsSummary
                      ? t("aiSettings.models.card.step2ToggleOffTooltip")
                      : t("aiSettings.models.card.step2ToggleOnTooltip")
                  }
                >
                  {t("aiSettings.models.card.step2")}
                </button>
              </div>
            </div>
          </div>
        ) : null}

        {displayed.map((rec) => (
          <RecommendedCard
            key={rec.mainFile}
            rec={rec}
            installed={isInstalled(rec)}
            usedForSummary={summaryMain === rec.mainFile}
            busyFiles={busyFiles}
            partialMap={partialMap}
            progress={progress}
            onDownload={onDownloadRecommended}
            onPause={onPauseDownload}
            onToggleStep={onToggleStep}
            onUninstall={onUninstallRecommended}
          />
        ))}

        {/* 本地模型区——用户自己导入 / 手动放进模型目录、但不在推荐列表里的文件。
            没有这个区，导入的非推荐模型（如 gemma-4-12B）拷进去后 UI 上无处显示。
            main 自动配对 mmproj（按文件名前缀）。 */}
        {localPairs.length > 0 || unpairedMmprojs.length > 0 ? (
          <>
            <div className={styles.modelCustomHint}>
              {t("aiSettings.models.local.title")}
            </div>
            {localPairs.map(({ main, mmproj }) => (
              <LocalModelCard
                key={main.filename}
                main={main}
                mmproj={mmproj}
                usedForSummary={summaryMain === main.filename}
                onToggleStep={onToggleLocalStep}
                onDelete={onDeleteLocal}
              />
            ))}
            {unpairedMmprojs.map((m) => (
              <LocalModelCard
                key={m.filename}
                main={m}
                mmproj={null}
                orphanMmproj
                usedForSummary={false}
                onToggleStep={onToggleLocalStep}
                onDelete={onDeleteLocal}
              />
            ))}
          </>
        ) : null}

        {/* 自定义 HuggingFace 仓库下载——比推荐卡更通用，但风险也高（用户得自己挑兼容
            llama.cpp 的 GGUF）。放在所有推荐卡之后；表单本身仍可折叠收起。 */}
        <CustomHfDownload
          show={showCustom}
          onToggle={() => setShowCustom(!showCustom)}
          repo={customRepo}
          setRepo={setCustomRepo}
          mainFile={customMainFile}
          setMainFile={setCustomMainFile}
          mmprojFile={customMmprojFile}
          setMmprojFile={setCustomMmprojFile}
          busy={customBusy}
          mainBusy={customMainBusy}
          mmprojBusy={customMmprojBusy}
          progress={progress}
          onDownload={onDownloadCustom}
        />

        {/* 导入本地已有 GGUF 文件——给"自己下好/网盘拷来/别的机器传来"的模型一个入口。
            拷贝进模型目录，源文件不动，进入清单后跟下载来的模型完全等价。 */}
        <ImportLocalModel
          importing={importingFiles}
          progress={progress}
          onImport={onImportLocal}
        />
      </div>
      <ConfirmDialog
        open={confirmingUninstall != null}
        title={t("aiSettings.models.uninstallConfirmTitle", {
          name: confirmingUninstall?.displayName ?? "",
        })}
        message={t("aiSettings.models.uninstallConfirmMessage", {
          extra: confirmingUninstall?.mmprojFile
            ? t("aiSettings.models.uninstallConfirmExtra")
            : "",
        })}
        variant="danger"
        onConfirm={() => {
          const rec = confirmingUninstall;
          if (rec) void doUninstallRecommended(rec);
        }}
        onCancel={() => setConfirmingUninstall(null)}
      />
      {/* 云端选择的隐私确认：Text（段总结）= 应用名/分类/活动时间线上传（轻警示）。 */}
      <ConfirmDialog
        open={cloudConfirm}
        title={t("aiSettings.models.cloud.summaryConfirmTitle")}
        message={t("aiSettings.models.cloud.summaryConfirmMessage", {
          provider: settings?.ai.externalProvider || "cloud",
        })}
        confirmLabel={t("aiSettings.models.cloud.summaryConfirmAccept")}
        variant="primary"
        onConfirm={() => void onConfirmCloudSelect()}
        onCancel={() => setCloudConfirm(false)}
      />
    </div>
  );
}

/**
 * 导入本地模型文件按钮——弹系统文件选择框（可多选 .gguf），把选中文件拷进
 * 模型目录。无表单、无展开，一个按钮完成。导入进行中原地切成进度态并禁用。
 */
function ImportLocalModel({
  importing,
  progress,
  onImport,
}: {
  importing: ReadonlySet<string>;
  progress: Record<string, ModelDownloadProgress>;
  onImport: () => void;
}) {
  const { t } = useTranslation();
  const busy = importing.size > 0;
  // 只显示第一个正在导入的文件名；多选时其余排队，逐个显示
  const current = busy ? Array.from(importing)[0] : "";
  // 拷贝进度复用下载事件通道，按落盘名索引；有总字节数才显示百分比
  const cur = current ? progress[current] : null;
  const pct =
    cur && cur.total ? Math.floor((cur.downloaded / cur.total) * 100) : null;
  const label =
    pct != null
      ? t("aiSettings.models.import.importingPct", { name: current, pct })
      : t("aiSettings.models.import.importing", { name: current });

  return (
    <button
      type="button"
      className={styles.modelExpandBtn}
      onClick={onImport}
      disabled={busy}
      title={t("aiSettings.models.import.tooltip")}
    >
      {busy ? (
        <Loader2
          size={14}
          strokeWidth={2}
          className={styles.testSpin}
        />
      ) : (
        <FolderInput size={14} strokeWidth={2} />
      )}
      {busy ? label : t("aiSettings.models.import.button")}
    </button>
  );
}

/**
 * 自定义 HuggingFace 仓库下载表单——给"推荐列表里没有但 llama.cpp 能跑的 GGUF"
 * 留个口子。Repo + main 文件名必填；mmproj 仅 vision 模型需要。
 *
 * 不做容差比对（expected_bytes=0），用户填的是文件名而不是字节数；后端会按 HF
 * 给的 content-length 跑完就当成功。
 */
function CustomHfDownload({
  show,
  onToggle,
  repo,
  setRepo,
  mainFile,
  setMainFile,
  mmprojFile,
  setMmprojFile,
  busy,
  mainBusy,
  mmprojBusy,
  progress,
  onDownload,
}: {
  show: boolean;
  onToggle: () => void;
  repo: string;
  setRepo: (v: string) => void;
  mainFile: string;
  setMainFile: (v: string) => void;
  mmprojFile: string;
  setMmprojFile: (v: string) => void;
  busy: boolean;
  mainBusy: boolean;
  mmprojBusy: boolean;
  progress: Record<string, ModelDownloadProgress>;
  onDownload: () => void;
}) {
  const { t } = useTranslation();
  const canSubmit =
    !busy && repo.trim().length > 0 && mainFile.trim().length > 0;
  const activeFile = mainBusy ? mainFile : mmprojBusy ? mmprojFile : "";
  const activeProgress = activeFile ? progress[activeFile] : null;
  const activeIsMmproj = activeFile === mmprojFile && !!mmprojFile;

  return (
    <>
      <button
        type="button"
        className={styles.modelExpandBtn}
        onClick={onToggle}
        disabled={busy && !show}
        title={
          busy && !show ? t("aiSettings.models.expand.busyTooltip") : undefined
        }
      >
        {show ? (
          <ChevronDown
            size={14}
            strokeWidth={2}
            className={`${styles.modelExpandChevron} ${styles.modelExpandChevronOpen}`}
          />
        ) : (
          <Plus size={14} strokeWidth={2} />
        )}
        {show
          ? t("aiSettings.models.custom.collapse")
          : t("aiSettings.models.custom.expand")}
      </button>

      <div
        className={`${styles.modelTailWrap} ${
          show ? styles.modelTailWrapOpen : ""
        }`}
        aria-hidden={!show}
      >
        <div className={styles.modelTailInner}>
          <div className={styles.modelCustomCard}>
            <div className={styles.modelCustomHint}>
              {t("aiSettings.models.custom.hint")}
            </div>
            <label className={styles.modelCustomField}>
              <span className={styles.modelCustomLabel}>
                {t("aiSettings.models.custom.repoLabel")}
              </span>
              <input
                type="text"
                className={styles.externalInput}
                placeholder={t("aiSettings.models.custom.repoPlaceholder")}
                value={repo}
                onChange={(e) => setRepo(e.target.value)}
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                disabled={busy}
              />
            </label>
            <label className={styles.modelCustomField}>
              <span className={styles.modelCustomLabel}>
                {t("aiSettings.models.custom.mainFileLabel")}
              </span>
              <input
                type="text"
                className={styles.externalInput}
                placeholder={t("aiSettings.models.custom.mainFilePlaceholder")}
                value={mainFile}
                onChange={(e) => setMainFile(e.target.value)}
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                disabled={busy}
              />
            </label>
            <label className={styles.modelCustomField}>
              <span className={styles.modelCustomLabel}>
                {t("aiSettings.models.custom.mmprojFileLabel")}
              </span>
              <input
                type="text"
                className={styles.externalInput}
                placeholder={t(
                  "aiSettings.models.custom.mmprojFilePlaceholder",
                )}
                value={mmprojFile}
                onChange={(e) => setMmprojFile(e.target.value)}
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="off"
                disabled={busy}
              />
            </label>
            <div className={styles.modelCustomActions}>
              <button
                type="button"
                className={styles.testBtn}
                onClick={onDownload}
                disabled={!canSubmit}
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
            </div>
            {busy && activeProgress ? (
              <div className={styles.engineProgressWrap}>
                <div className={styles.engineProgressBar}>
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
        </div>
      </div>
    </>
  );
}

/**
 * 本地模型卡片——展示用户导入 / 手动放入、不在推荐列表里的 GGUF 文件。
 *
 * 结构比推荐卡简化：无 logo / caps / 下载按钮，只有文件名 + 大小 + Text
 * toggle + 删除。`mmproj` 是自动配对的视觉投影文件。
 * `orphanMmproj=true` 时 `main` 其实是个没配到主模型的孤儿 mmproj，
 * 只显示说明 + 删除（mmproj 不能单独当模型用）。
 */
function LocalModelCard({
  main,
  mmproj,
  orphanMmproj = false,
  usedForSummary,
  onToggleStep,
  onDelete,
}: {
  main: ModelEntry;
  mmproj: ModelEntry | null;
  orphanMmproj?: boolean;
  usedForSummary: boolean;
  onToggleStep: (main: ModelEntry, mmproj: ModelEntry | null) => void;
  onDelete: (main: ModelEntry, mmproj: ModelEntry | null) => void;
}) {
  const { t } = useTranslation();
  const totalGB =
    (main.sizeBytes + (mmproj?.sizeBytes ?? 0)) / 1024 / 1024 / 1024;

  return (
    <div className={styles.modelCard}>
      <div className={styles.modelCardRow}>
        <div className={styles.modelCardLeft}>
          <div className={styles.modelCardIdentity}>
            <div className={styles.modelCardNameRow}>
              <span className={styles.modelCardName}>{main.filename}</span>
              <span className={styles.modelCardSize}>
                {t("aiSettings.models.card.approxSize", {
                  size: totalGB.toFixed(1),
                })}
              </span>
            </div>
            {orphanMmproj ? (
              <span className={styles.modelCardSize}>
                {t("aiSettings.models.local.orphanMmproj")}
              </span>
            ) : mmproj ? (
              <span className={styles.modelCardSize}>
                {t("aiSettings.models.local.pairedMmproj", {
                  name: mmproj.filename,
                })}
              </span>
            ) : null}
          </div>
        </div>
        <div className={styles.modelCardRight}>
          {orphanMmproj ? null : (
            <button
              type="button"
              className={`${styles.modelStepToggle} ${
                usedForSummary ? styles.modelStepToggleActive : ""
              }`}
              onClick={() => onToggleStep(main, mmproj)}
              title={
                usedForSummary
                  ? t("aiSettings.models.card.step2ToggleOffTooltip")
                  : t("aiSettings.models.card.step2ToggleOnTooltip")
              }
            >
              {t("aiSettings.models.card.step2")}
            </button>
          )}
          <button
            type="button"
            className={styles.uninstallOutline}
            onClick={() => onDelete(main, mmproj)}
            title={t("aiSettings.models.card.uninstallTooltipInstalled")}
          >
            <Trash2 size={14} strokeWidth={1.85} />
            {t("aiSettings.models.card.uninstall")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** 推荐模型卡片——单行紧凑：左边名字 + 大小 + ⓘ tooltip，右贴齐下载按钮。 */
function RecommendedCard({
  rec,
  installed,
  usedForSummary,
  busyFiles,
  partialMap,
  progress,
  onDownload,
  onPause,
  onToggleStep,
  onUninstall,
}: {
  rec: RecommendedModel;
  installed: boolean;
  /** 该模型当前是段总结选择 → Text toggle 显示已激活 */
  usedForSummary: boolean;
  busyFiles: Set<string>;
  /** 半成品 map：file → 已下字节数。结合 inflight 判断 paused */
  partialMap: Readonly<Record<string, number>>;
  progress: Record<string, ModelDownloadProgress>;
  onDownload: (rec: RecommendedModel) => void;
  onPause: (filename: string) => void;
  onToggleStep: (rec: RecommendedModel) => void;
  onUninstall: (rec: RecommendedModel) => void;
}) {
  const { t } = useTranslation();
  const totalGB = (rec.mainBytes + rec.mmprojBytes) / 1024 / 1024 / 1024;
  // mmproj 在本地落盘用 saveAs（rec-aware 唯一名，避免 unsloth 系列 mmproj-F16.gguf
  // 跨 rec 同名互覆盖 / 串扰），busy 检测、progress 索引、partial 检测都用 saveAs。
  const mmprojLocal = rec.mmprojFile ? mmprojSaveAs(rec) : "";
  // 只看 busyFiles（模块级 store，跨挂载存活）：mainFile / mmprojSaveAs 都已按 rec
  // 唯一，无串扰风险。以前这里还与门了组件本地的 isRecBusy——切走页面再回来
  // busyRecs 丢失、下载还在跑，卡片就错误显示成"继续/下载"且没有进度条。
  const mainBusy = busyFiles.has(rec.mainFile);
  const mmprojBusy = !!mmprojLocal && busyFiles.has(mmprojLocal);
  const busy = mainBusy || mmprojBusy;
  const activeFile = mainBusy
    ? rec.mainFile
    : mmprojBusy
      ? mmprojLocal
      : null;
  const activeProgress = activeFile ? progress[activeFile] : null;
  const activeIsMmproj = activeFile === mmprojLocal && !!mmprojLocal;
  // 已暂停态：partial 存在但不在 inflight。仅看 mainFile（每个 rec 唯一）的 partial
  // 即可——用户在 mmproj 阶段暂停时 mainFile 已下完不在 partial 里，但本 UI 折中：
  // 只要 main 不在 partial 就当作未暂停（实际效果 OK，因为 main 完整就够走 step）。
  const mainPaused = !mainBusy && rec.mainFile in partialMap;
  const paused = !busy && mainPaused;
  const pausedBytes = mainPaused ? partialMap[rec.mainFile] : 0;

  return (
    <div className={styles.modelCard}>
      <div className={styles.modelCardRow}>
        <div className={styles.modelCardLeft}>
          {(BRAND_LOGOS[rec.brand] ?? rec.logoUrl) ? (
            // onError 是 React 资源事件，不是用户交互
            // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
            <img
              className={styles.modelCardLogo}
              src={BRAND_LOGOS[rec.brand] ?? rec.logoUrl ?? undefined}
              alt=""
              loading="lazy"
              referrerPolicy="no-referrer"
              onError={(e) => {
                // logo 加载失败时隐藏 img 元素，让卡片不带占位坑
                e.currentTarget.style.display = "none";
              }}
            />
          ) : null}
          {/* 名 + caps 上下两行堆叠在 logo 右侧；logo 用 align-items:flex-start
              视觉上占满两行高度（square avatar 风格）。size 跟在 ⓘ 后面给
              用户一眼看到"模型多大"，不用扫到卡片右边。 */}
          <div className={styles.modelCardIdentity}>
            <div className={styles.modelCardNameRow}>
              <span className={styles.modelCardName}>{rec.displayName}</span>
              <button
                type="button"
                className={styles.engineInfoWrap}
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
              </button>
              <span className={styles.modelCardSize}>
                {t("aiSettings.models.card.approxSize", {
                  size: totalGB.toFixed(1),
                })}
              </span>
            </div>
            {/* 能力 / 定位 caps：来自 recommended-models.json 的 `caps` 数组
                （如 ["VISION","TEXT","DEFAULT"]）；color/class 走 CAP_CLASS_MAP，
                新加未知 type 会 fallback 到灰色不报错。
                title 走 i18n `card.capsTooltips.<CAP>`；i18next 找不到 key 时返回 key
                自身，这里用 `defaultValue: ""` 让未注册的 cap silently 无 tooltip。 */}
            {rec.caps.length > 0 ? (
              <div className={styles.modelCardCaps}>
                {rec.caps.map((cap) => {
                  // i18next 默认找不到 key 返回 key 自身——`defaultValue: ""`
                  // 让未注册的 cap 拿到空串，下面据此决定要不要渲染 tooltip。
                  const tip = t(
                    `aiSettings.models.card.capsTooltips.${cap}`,
                    { defaultValue: "" },
                  );
                  return (
                    <span
                      key={cap}
                      className={`${styles.capBadge} ${CAP_CLASS_MAP[cap] ?? styles.capBadgeUnknown}`}
                    >
                      {cap}
                      {tip ? (
                        <span className={styles.capBadgeTip} role="tooltip">
                          {tip}
                        </span>
                      ) : null}
                    </span>
                  );
                })}
              </div>
            ) : null}
          </div>
        </div>
        <div className={styles.modelCardRight}>
          {!installed ? (
            busy ? (
              // 在跑 → 显示「暂停」按钮
              <button
                type="button"
                className={styles.testBtn}
                onClick={() => activeFile && onPause(activeFile)}
                title={t("aiSettings.models.card.pauseTooltip")}
              >
                <Pause size={14} strokeWidth={2} />
                {t("aiSettings.models.card.pause")}
              </button>
            ) : paused ? (
              // 已暂停（有 partial 但不在 inflight） → 显示「继续」按钮 + 已下进度
              <button
                type="button"
                className={styles.downloadOutline}
                onClick={() => onDownload(rec)}
                title={t("aiSettings.models.card.resumeTooltip", {
                  size: (pausedBytes / 1024 / 1024).toFixed(1),
                })}
              >
                <Play size={14} strokeWidth={2} />
                {t("aiSettings.models.card.resume")}
              </button>
            ) : (
              // 未下载、未暂停 → 标准「下载」按钮（outline 风，发现态而非支付级 CTA）
              <button
                type="button"
                className={styles.downloadOutline}
                onClick={() => onDownload(rec)}
              >
                <Download size={14} strokeWidth={2} />
                {t("aiSettings.models.card.download")}
              </button>
            )
          ) : (
            /* 已下载状态不再单独显 badge —— Text toggle + 右侧 trash
               已经足够暗示模型可用 / 可卸载，避免视觉重复。
               Text toggle：纯文本任务，所有模型都能用，不禁用 */
            <button
              type="button"
              className={`${styles.modelStepToggle} ${
                usedForSummary ? styles.modelStepToggleActive : ""
              }`}
              onClick={() => onToggleStep(rec)}
              title={
                usedForSummary
                  ? t("aiSettings.models.card.step2ToggleOffTooltip")
                  : t("aiSettings.models.card.step2ToggleOnTooltip")
              }
            >
              {t("aiSettings.models.card.step2")}
            </button>
          )}
          <button
            type="button"
            className={styles.uninstallOutline}
            onClick={() => onUninstall(rec)}
            disabled={!installed || busy}
            title={
              installed
                ? t("aiSettings.models.card.uninstallTooltipInstalled")
                : t("aiSettings.models.card.uninstallTooltipNotInstalled")
            }
          >
            <Trash2 size={14} strokeWidth={1.85} />
            {t("aiSettings.models.card.uninstall")}
          </button>
        </div>
      </div>
      {busy && activeProgress ? (
        <div className={styles.engineProgressWrap}>
          <div className={styles.engineProgressBar}>
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
