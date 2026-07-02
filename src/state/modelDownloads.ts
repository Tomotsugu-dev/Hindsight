/**
 * 模型下载的全局状态：进度 Map + 全局 listener + 同文件请求去重。
 *
 * 提到 module-level 是为了修两个 bug：
 *
 * 1. 进度条丢：原来 `MODEL_DOWNLOAD_EVENT` listener 在 ModelsSection 组件内部
 *    用 useEffect 注册。切侧边栏让 ModelsSection unmount，cleanup 调 unlisten，
 *    后端继续发的事件就没人接了；切回来 progress state 是空的、按钮也回到"下载"。
 *
 * 2. 重新下载失败：第一次 `download_model` 命令仍在后端 stream 数据写
 *    `<file>.partial`，文件 handle 持有中。用户切走再回来点下载发出第二次 invoke，
 *    后端进 `download_from_hf` 走到 `File::create(temp)`——Windows 上同名文件
 *    被前一个 task 持有 handle 时 create 会抛 IO 错误（OS 共享冲突），第二次直接失败。
 *
 *    前端在这里加 `inflight` Map 做去重：同名文件已经有 in-flight promise 时，
 *    复用它而不是再 invoke 一遍后端。这样切走再切回来再点也不会触发后端冲突。
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  api,
  MODEL_DOWNLOAD_EVENT,
  type ModelDownloadProgress,
} from "../api/hindsight";
import { logError } from "../lib/logger";

type ProgressMap = Readonly<Record<string, ModelDownloadProgress>>;

// 进度快照：每次写整体替换（immutable），useSyncExternalStore 的 getSnapshot
// 才能返回稳定引用；同样写法保证 React 用 Object.is 比较时能正确检测变化。
let progressSnap: ProgressMap = Object.freeze({});

// 在跑的下载请求；key = 文件名（跟 progress 同 key）。
const inflight: Map<string, Promise<string>> = new Map();
let inflightSnap: ReadonlySet<string> = Object.freeze(new Set<string>());

// 半成品文件 + 已下字节数（来自 list_partial_downloads）。决定 UI 是否给某文件渲染
// "继续"按钮——partial 存在且不在 inflight = 已暂停 / 等续传。
let partialSnap: Readonly<Record<string, number>> = Object.freeze({});

const listeners = new Set<() => void>();

// listener 启动一次永不解除——app 整个生命周期内活着。
let listenerInit: Promise<UnlistenFn> | null = null;

function notify(): void {
  listeners.forEach((cb) => cb());
}

function ensureListener(): Promise<UnlistenFn> {
  if (!listenerInit) {
    listenerInit = listen<ModelDownloadProgress>(MODEL_DOWNLOAD_EVENT, (ev) => {
      progressSnap = Object.freeze({
        ...progressSnap,
        [ev.payload.file]: ev.payload,
      });
      notify();
    });
  }
  return listenerInit;
}

function refreshInflight(): void {
  inflightSnap = Object.freeze(new Set(inflight.keys()));
}

export function subscribeModelDownloads(cb: () => void): () => void {
  void ensureListener();
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export function getProgressSnapshot(): ProgressMap {
  return progressSnap;
}

export function getInflightSnapshot(): ReadonlySet<string> {
  return inflightSnap;
}

/** 半成品快照：file → 已下字节数。判断是否处于"已暂停"状态用 `!inflightSnap.has(f) && f in partialSnap`。 */
export function getPartialSnapshot(): Readonly<Record<string, number>> {
  return partialSnap;
}

/** 拉一次 list_partial_downloads，把结果同步到 partialSnap。
 *  下载失败 / 暂停后 / 卸载后等场景调用，让 UI 刷新"是否还有 partial 在那"。
 *  失败仅 log，不抛——partial 列表是辅助 UI，拿不到不影响主流程。 */
export async function refreshPartials(): Promise<void> {
  try {
    const list = await api.listPartialDownloads();
    const next: Record<string, number> = {};
    for (const p of list) next[p.filename] = p.downloadedBytes;
    partialSnap = Object.freeze(next);
    notify();
  } catch (e) {
    logError("modelDownloads.refreshPartials", e);
  }
}

/** 暂停某文件下载——后端 cancel 命令翻 flag，inflight promise 会以
 *  "download cancelled:" 前缀的错误 reject；本函数不等待，立即返回。 */
export async function cancelModelDownload(file: string): Promise<void> {
  try {
    await api.cancelModelDownload(file);
  } catch (e) {
    logError("modelDownloads.cancel", e);
  }
}

/** 下载完成（成功或失败）后清掉该文件的进度条。 */
export function clearModelDownloadProgress(file: string): void {
  if (file in progressSnap) {
    const next = { ...progressSnap };
    delete next[file];
    progressSnap = Object.freeze(next);
    notify();
  }
}

/**
 * 下载某文件；同名（按 saveAs / file）已经在跑时复用现有 promise。
 *
 * `saveAs` 用于让落盘文件名跟 HF URL 上的文件名解耦——多个 rec 的 mmproj 在 HF 上
 * 常常同名（比如 unsloth 系列都是 mmproj-F16.gguf），落盘必须用 rec-aware 的唯一名。
 * inflight key / progress event / cancel 都按 saveAs 索引。
 *
 * 进度通过 [`subscribeModelDownloads`] 拉取的 progressSnap 表达——这里只负责
 * 发起 / 复用 invoke，不直接处理进度。
 */
export function downloadModelDedup(
  repo: string,
  file: string,
  expectedBytes: number,
  saveAs?: string,
): Promise<string> {
  const key = saveAs ?? file;
  const existing = inflight.get(key);
  if (existing) return existing;

  // listen() 注册要跟 core 走一个来回，注册完成前后端 emit 的进度事件会丢
  //（首次下载头几个事件没进度条）。先等注册完成再 invoke；注册失败也照常下载。
  const p = ensureListener()
    .catch(() => undefined)
    .then(() => api.downloadModel(repo, file, expectedBytes, saveAs))
    .finally(() => {
      inflight.delete(key);
      refreshInflight();
      // 下载收尾（成功 / 失败 / 取消）都刷一遍 partial：成功时 partial 已被 rename 走，
      // partialSnap 该文件条目消失；取消 / 失败时 partial 还在，UI 应进入"已暂停"状态
      void refreshPartials();
      notify();
    });
  inflight.set(key, p);
  refreshInflight();
  notify();
  return p;
}
