import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowUpDown,
  Check,
  ChevronDown,
  Cloud,
  Cpu,
  Download,
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

/**
 * 模型管理 Section（Phase 1B-β）。
 *
 * 顶部展示 Hindsight 内置推荐卡片（HF 一键下载）；下面是用户已下载的本地
 * .gguf 文件清单 + 删除入口 + 自定义 HF 表单。被 ModelsTab 包一层 Section 直接渲染。
 */
export function ModelsSection() {
  const { t } = useTranslation();
  // settings 用来读 describeMain / summaryMain：判断每张推荐卡是不是被某个 step 选用，
  // 在卡上显示对应 chip。reload 让下载完成后能拉到刚刷新的本地文件清单。
  const { settings, reload } = useAiSettings();
  const describeMain = settings?.ai.describeMain || settings?.ai.activeMain || "";
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
  const isInstalled = (rec: RecommendedModel): boolean => {
    if (!localFilenames.has(rec.mainFile)) return false;
    if (rec.mmprojFile && !localFilenames.has(mmprojSaveAs(rec))) return false;
    return true;
  };

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
   * 给某个推荐模型 toggle step 1 / step 2 分配。
   *
   * 同 step 互斥由后端隐式保证（setStepModel 直接覆盖，不累加）；前端这里只负责
   * 区分"当前已是该 step → 清空"和"否则 → 切换到该模型"两种情况。
   */
  const onToggleStep = async (
    rec: RecommendedModel,
    step: "describe" | "summary",
  ) => {
    setError(null);
    const current =
      step === "describe"
        ? settings?.ai.describeMain || settings?.ai.activeMain || ""
        : settings?.ai.summaryMain || settings?.ai.activeMain || "";
    try {
      if (current === rec.mainFile) {
        // 已是该 step 在用 → 清空覆盖（后端 fallback 到 activeMain）
        await api.setStepModel(step, "", null);
      } else {
        // mmproj 也用 saveAs（落盘文件名），跟 settings 实际能加载的文件名对齐
        await api.setStepModel(
          step,
          rec.mainFile,
          rec.mmprojFile ? mmprojSaveAs(rec) : null,
        );
      }
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  };

  /**
   * 云端卡 Text 按钮 toggle：在「选中云端为 step 2」和「fallback 回本地」之间切换。
   *
   * 通过把 `summary_main` 写成 [`SUMMARY_CLOUD_SENTINEL`] 来表示选中云端，后端
   * [`build_step2`] 看到 sentinel + `external_enabled=true` 才路由到 External。
   * 跟本地卡的 toggle 互斥：本地卡 set summary 是真实文件名、覆盖 sentinel；
   * 云端卡 set sentinel、覆盖任何本地文件名。一处状态，互斥自然成立。
   */
  const onToggleCloudSummary = async () => {
    setError(null);
    try {
      if (cloudIsSelectedAsSummary) {
        await api.setStepModel("summary", "", null);
      } else {
        await api.setStepModel("summary", SUMMARY_CLOUD_SENTINEL, null);
      }
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
        {/* 云端 API 启用时第一行展示云端卡——表明 step 2 当前不走本地推荐里的任何模型，
            而是路由到 Cloud API tab 配的 endpoint + model。固定置顶，不参与筛选/排序。
            step 1 / step 2 按钮都 disabled：cloud 没 vision 能力（step 1 永远不可用），
            step 2 已是 active（要切换去「云端 API」tab）。两个按钮存在是为了视觉上跟本地
            模型卡对齐 —— 用户一眼能看清云端在哪个 step 上、为啥不能在这里改。 */}
        {settings?.ai.externalEnabled ? (
          <div
            className={`${styles.modelCard} ${styles.modelCardCloud}`}
            title={t("aiSettings.models.cloud.step2Tooltip")}
          >
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
                {/* Vision (Step 1)：云端 deepseek / openai 文本接口不支持图描述，永远 disabled */}
                <button
                  type="button"
                  className={styles.modelStepToggle}
                  disabled
                  title={t("aiSettings.models.cloud.step1DisabledTooltip")}
                >
                  {t("aiSettings.models.card.step1")}
                </button>
                {/* Text (Step 2)：可点击 toggle，跟本地卡的 step 2 是 radio 关系 */}
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
            usedForDescribe={describeMain === rec.mainFile}
            usedForSummary={summaryMain === rec.mainFile}
            isRecBusy={busyRecs.has(rec.mainFile)}
            busyFiles={busyFiles}
            partialMap={partialMap}
            progress={progress}
            onDownload={onDownloadRecommended}
            onPause={onPauseDownload}
            onToggleStep={onToggleStep}
            onUninstall={onUninstallRecommended}
          />
        ))}

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
    </div>
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

/** 推荐模型卡片——单行紧凑：左边名字 + 大小 + ⓘ tooltip，右贴齐下载按钮。 */
function RecommendedCard({
  rec,
  installed,
  usedForDescribe,
  usedForSummary,
  isRecBusy,
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
  /** 该模型当前是 step 1 选择 → step 1 toggle 显示已激活 */
  usedForDescribe: boolean;
  /** 该模型当前是 step 2 选择 → step 2 toggle 显示已激活 */
  usedForSummary: boolean;
  /** 这个 rec 当前是不是 onDownloadRecommended 的发起者。
   *  必须按 rec 维度跟踪——所有 unsloth vision 镜像的 mmproj 都叫 mmproj-F16.gguf，
   *  仅靠 busyFiles.has(裸名) 会让所有 vision 卡都判 busy 串扰进度条。 */
  isRecBusy: boolean;
  busyFiles: Set<string>;
  /** 半成品 map：file → 已下字节数。结合 inflight 判断 paused */
  partialMap: Readonly<Record<string, number>>;
  progress: Record<string, ModelDownloadProgress>;
  onDownload: (rec: RecommendedModel) => void;
  onPause: (filename: string) => void;
  onToggleStep: (
    rec: RecommendedModel,
    step: "describe" | "summary",
  ) => void;
  onUninstall: (rec: RecommendedModel) => void;
}) {
  const { t } = useTranslation();
  const totalGB = (rec.mainBytes + rec.mmprojBytes) / 1024 / 1024 / 1024;
  // mainBusy / mmprojBusy 跟 isRecBusy 与门：所有 unsloth vision 镜像的 mmproj 同名
  // ("mmproj-F16.gguf")，单看 busyFiles.has(裸名) 会让所有 vision 卡串扰显示进度条。
  // 加 isRecBusy（"我自己是不是 onDownloadRecommended 的发起者"）作为第二维度。
  // mmproj 在本地落盘用 saveAs（rec-aware 唯一名，避免 unsloth 系列 mmproj-F16.gguf
  // 跨 rec 同名互覆盖 / 串扰），busy 检测、progress 索引、partial 检测都用 saveAs。
  const mmprojLocal = rec.mmprojFile ? mmprojSaveAs(rec) : "";
  // mainBusy / mmprojBusy 跟 isRecBusy 与门：双维度防误判（mainFile 已经唯一，
  // 加 isRecBusy 是历史遗留保险——以前 mmproj 同名时这是必须的）
  const mainBusy = isRecBusy && busyFiles.has(rec.mainFile);
  const mmprojBusy = isRecBusy && !!mmprojLocal && busyFiles.has(mmprojLocal);
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
          {rec.logoUrl ? (
            // onError 是 React 资源事件，不是用户交互
            // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
            <img
              className={styles.modelCardLogo}
              src={rec.logoUrl}
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
            <>
              {/* 已下载：紫色实心 pill + ✓，跟未下载的「下载」按钮视觉等大，
                  让"模型有没有装"一望即知（旧版只靠 step1/step2 toggle 存在感
                  示意，新用户经常看不出）。卸载走右边的 trash icon button。 */}
              <span className={styles.installedBadge}>
                <Check size={14} strokeWidth={2.4} />
                {t("aiSettings.models.card.installedBadge")}
              </span>
              {/* step 1 toggle：vision=false 时禁用并给 tooltip 解释 */}
              <button
                type="button"
                className={`${styles.modelStepToggle} ${
                  usedForDescribe ? styles.modelStepToggleActive : ""
                }`}
                onClick={() => onToggleStep(rec, "describe")}
                disabled={!rec.vision}
                title={
                  !rec.vision
                    ? t("aiSettings.models.card.step1DisabledTooltip")
                    : usedForDescribe
                      ? t("aiSettings.models.card.step1ToggleOffTooltip")
                      : t("aiSettings.models.card.step1ToggleOnTooltip")
                }
              >
                {t("aiSettings.models.card.step1")}
              </button>
              {/* step 2 toggle：纯文本任务，所有模型都能用，不禁用 */}
              <button
                type="button"
                className={`${styles.modelStepToggle} ${
                  usedForSummary ? styles.modelStepToggleActive : ""
                }`}
                onClick={() => onToggleStep(rec, "summary")}
                title={
                  usedForSummary
                    ? t("aiSettings.models.card.step2ToggleOffTooltip")
                    : t("aiSettings.models.card.step2ToggleOnTooltip")
                }
              >
                {t("aiSettings.models.card.step2")}
              </button>
            </>
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
