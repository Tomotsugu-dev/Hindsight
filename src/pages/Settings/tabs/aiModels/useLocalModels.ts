import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  type ModelDownloadProgress,
  type ModelEntry,
  type RecommendedModel,
} from "../../../../api/hindsight";
import {
  cancelModelDownload,
  clearModelDownloadProgress,
  downloadModelDedup,
  getInflightSnapshot,
  getPartialSnapshot,
  getProgressSnapshot,
  refreshPartials,
  subscribeModelDownloads,
} from "../../../../state/modelDownloads";
import { useSettings } from "../../../../state/settings";
import { logError } from "../../../../lib/logger";
import {
  groupLocalFiles,
  isRecommendedInstalled,
  mmprojSaveAs,
  type LocalItem,
} from "./localFiles";

/** Which part of the page an error belongs to. */
export type ModelsErrorPlace = "mine" | "catalog";

export interface LocalModels {
  recommended: RecommendedModel[];
  /** The files on disk, grouped into models. */
  items: LocalItem[];
  isInstalled: (rec: RecommendedModel) => boolean;
  /** Download progress by saved file name. */
  progress: Record<string, ModelDownloadProgress>;
  /** Saved file names that are downloading now. */
  busyFiles: ReadonlySet<string>;
  /** Saved file name → bytes of a paused download. */
  partials: Readonly<Record<string, number>>;
  /** Names of the files being imported now. */
  importing: ReadonlySet<string>;
  error: { place: ModelsErrorPlace; message: string } | null;
  download: (rec: RecommendedModel) => Promise<void>;
  pause: (file: string) => void;
  remove: (item: LocalItem) => Promise<void>;
  importFiles: () => Promise<void>;
  downloadCustom: (repo: string, mainFile: string, mmprojFile: string) => Promise<void>;
}

function message(e: unknown): string {
  return typeof e === "string" ? e : String(e);
}

/**
 * Read the recommended models and the files on disk, and download, pause, import and delete them.
 *
 * Progress and running downloads live in `state/modelDownloads`, so they survive leaving the page.
 */
export function useLocalModels(): LocalModels {
  const { t } = useTranslation();
  const { reload } = useSettings();
  const [recommended, setRecommended] = useState<RecommendedModel[]>([]);
  const [local, setLocal] = useState<ModelEntry[]>([]);
  const [importing, setImporting] = useState<ReadonlySet<string>>(new Set<string>());
  const [error, setError] = useState<LocalModels["error"]>(null);
  const progress = useSyncExternalStore(
    subscribeModelDownloads,
    getProgressSnapshot,
    getProgressSnapshot,
  );
  const inflight = useSyncExternalStore(
    subscribeModelDownloads,
    getInflightSnapshot,
    getInflightSnapshot,
  );
  const busyFiles = useMemo(() => new Set(inflight), [inflight]);
  const partials = useSyncExternalStore(
    subscribeModelDownloads,
    getPartialSnapshot,
    getPartialSnapshot,
  );

  const refresh = async () => {
    try {
      const [rec, loc] = await Promise.all([api.listRecommendedModels(), api.listLocalModels()]);
      setRecommended(rec);
      setLocal(loc);
    } catch (e) {
      logError("models.refresh", e);
    }
  };

  useEffect(() => {
    void refresh();
    // 上次没下完的文件要显示「继续」
    void refreshPartials();
  }, []);

  const items = useMemo(() => groupLocalFiles(local, recommended), [local, recommended]);

  const download = async (rec: RecommendedModel) => {
    if (busyFiles.has(rec.mainFile)) return;
    // 主文件和 mmproj 依次下：mmproj 落盘用带主文件名的唯一名，HF 上还是原名
    const files = [{ hfName: rec.mainFile, saveAs: rec.mainFile, bytes: rec.mainBytes }];
    if (rec.mmprojFile) {
      files.push({ hfName: rec.mmprojFile, saveAs: mmprojSaveAs(rec), bytes: rec.mmprojBytes });
    }
    setError(null);
    try {
      for (const f of files) {
        await downloadModelDedup(rec.repo, f.hfName, f.bytes, f.saveAs);
        clearModelDownloadProgress(f.saveAs);
      }
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      // 点「暂停」也会走到这里，那不是错误
      if (!message(e).startsWith("download cancelled")) {
        setError({ place: "catalog", message: message(e) });
      }
    }
  };

  const remove = async (item: LocalItem) => {
    const files =
      item.kind === "orphanMmproj"
        ? [item.file.filename]
        : [item.main.filename, ...(item.mmproj ? [item.mmproj.filename] : [])];
    setError(null);
    try {
      for (const f of files) await api.deleteModel(f);
    } catch (e) {
      setError({ place: "mine", message: message(e) });
    }
    await Promise.all([refresh(), reload()]);
  };

  const importFiles = async () => {
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
    if (picked == null) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    // 拷贝进度走下载事件，按源文件名索引
    for (const srcPath of paths) {
      const name = srcPath.split(/[\\/]/).pop() || srcPath;
      setImporting((s) => new Set(s).add(name));
      try {
        await api.importModel(srcPath);
        clearModelDownloadProgress(name);
      } catch (e) {
        setError({ place: "catalog", message: message(e) });
      } finally {
        setImporting((s) => {
          const n = new Set(s);
          n.delete(name);
          return n;
        });
      }
    }
    await Promise.all([refresh(), reload()]);
  };

  const downloadCustom = async (repo: string, mainFile: string, mmprojFile: string) => {
    if (!repo.trim() || !mainFile.trim()) {
      setError({ place: "catalog", message: t("aiSettings.models.custom.errorRequired") });
      return;
    }
    setError(null);
    try {
      // 不知道准确大小，传 0 关掉大小比对
      await downloadModelDedup(repo.trim(), mainFile.trim(), 0);
      clearModelDownloadProgress(mainFile.trim());
      if (mmprojFile.trim()) {
        await downloadModelDedup(repo.trim(), mmprojFile.trim(), 0);
        clearModelDownloadProgress(mmprojFile.trim());
      }
      await Promise.all([refresh(), reload()]);
    } catch (e) {
      setError({ place: "catalog", message: message(e) });
    }
  };

  return {
    recommended,
    items,
    isInstalled: (rec) => isRecommendedInstalled(rec, local),
    progress,
    busyFiles,
    partials,
    importing,
    error,
    download,
    pause: (file) => void cancelModelDownload(file),
    remove,
    importFiles,
    downloadCustom,
  };
}
