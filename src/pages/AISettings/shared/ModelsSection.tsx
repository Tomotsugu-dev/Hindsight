import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  ChevronDown,
  Download,
  HardDrive,
  Info,
  Loader2,
  Trash2,
} from "lucide-react";
import {
  api,
  type ModelDownloadProgress,
  type ModelEntry,
  type RecommendedModel,
} from "../../../api/hindsight";
import {
  clearModelDownloadProgress,
  downloadModelDedup,
  getInflightSnapshot,
  getProgressSnapshot,
  subscribeModelDownloads,
} from "../../../state/modelDownloads";
import { useAiSettings } from "./useAiSettings";
import { logError } from "../../../lib/logger";
import styles from "../AISettings.module.css";

/**
 * 模型管理 Section（Phase 1B-β）。
 *
 * 顶部展示 Hindsight 内置推荐卡片（HF 一键下载）；下面是用户已下载的本地
 * .gguf 文件清单 + 删除入口。
 *
 * 之前是独立 ModelsTab，现在合并到 EngineTab——把"运行时环境"（引擎 + 模型 + 引擎参数）
 * 集中在同一 tab，UI 入口少一个，符合"装好就跑"的常态使用路径。
 */
export function ModelsSection() {
  const { t } = useTranslation();
  // settings 用来读 activeMain：决定哪个推荐 / 本地文件在"当前使用"态。
  // reload 是因为 set_active_model 是旁路命令（不走 update_settings 通道），
  // 写完 settings 后前端 SettingsContext 不会自动 refetch，必须手动 reload。
  const { settings, reload } = useAiSettings();
  const activeMain = settings?.ai.activeMain ?? "";

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
  const [error, setError] = useState<string | null>(null);
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
      logError("models.refresh", e);
    }
  };

  useEffect(() => {
    void refresh();
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
    try {
      // 串行下：一个文件下完再开下一个，省网络竞争 + 进度展示更清晰。
      // dedup 会让同名文件已经在跑时直接复用现有 promise。
      for (const f of files) {
        await downloadModelDedup(rec.repo, f.name, f.bytes);
        clearModelDownloadProgress(f.name);
      }
      await api.setActiveModel(rec.mainFile, rec.mmprojFile ?? null);
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
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
          // 会看不到进度。
          const VISIBLE_DEFAULT = 2;
          const head = recommended.slice(0, VISIBLE_DEFAULT);
          const tail = recommended.slice(VISIBLE_DEFAULT);
          const tailHasBusy = tail.some((rec) => {
            const mainBusy = busyFiles.has(rec.mainFile);
            const mmprojBusy = !!rec.mmprojFile && busyFiles.has(rec.mmprojFile);
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

              {tail.length > 0 ? (
                <div
                  className={`${styles.modelTailWrap} ${
                    isOpen ? styles.modelTailWrapOpen : ""
                  }`}
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

/** 推荐模型卡片——单行紧凑：左边名字 + 大小 + ⓘ tooltip，右贴齐下载按钮。 */
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
  active: boolean;
  busyFiles: Set<string>;
  progress: Record<string, ModelDownloadProgress>;
  onDownload: (rec: RecommendedModel) => void;
  onUse: (rec: RecommendedModel) => void;
  onClear: () => void;
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
            <Info size={12} strokeWidth={2.2} className={styles.engineInfoIcon} />
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
                <Loader2 size={14} strokeWidth={2} className={styles.testSpin} />
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
