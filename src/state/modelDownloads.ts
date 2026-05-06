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

type ProgressMap = Readonly<Record<string, ModelDownloadProgress>>;

// 进度快照：每次写整体替换（immutable），useSyncExternalStore 的 getSnapshot
// 才能返回稳定引用；同样写法保证 React 用 Object.is 比较时能正确检测变化。
let progressSnap: ProgressMap = Object.freeze({});

// 在跑的下载请求；key = 文件名（跟 progress 同 key）。
const inflight: Map<string, Promise<string>> = new Map();
let inflightSnap: ReadonlySet<string> = Object.freeze(new Set<string>());

const listeners = new Set<() => void>();

// listener 启动一次永不解除——app 整个生命周期内活着。
let listenerInit: Promise<UnlistenFn> | null = null;

function notify(): void {
  listeners.forEach((cb) => cb());
}

function ensureListener(): void {
  if (listenerInit) return;
  listenerInit = listen<ModelDownloadProgress>(MODEL_DOWNLOAD_EVENT, (ev) => {
    progressSnap = Object.freeze({
      ...progressSnap,
      [ev.payload.file]: ev.payload,
    });
    notify();
  });
}

function refreshInflight(): void {
  inflightSnap = Object.freeze(new Set(inflight.keys()));
}

export function subscribeModelDownloads(cb: () => void): () => void {
  ensureListener();
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
 * 下载某文件；同名文件已经在跑时复用现有 promise。
 *
 * 进度通过 [`subscribeModelDownloads`] 拉取的 progressSnap 表达——这里只负责
 * 发起 / 复用 invoke，不直接处理进度。
 */
export function downloadModelDedup(
  repo: string,
  file: string,
  expectedBytes: number,
): Promise<string> {
  ensureListener();
  const existing = inflight.get(file);
  if (existing) return existing;

  const p = api
    .downloadModel(repo, file, expectedBytes)
    .finally(() => {
      inflight.delete(file);
      refreshInflight();
      notify();
    });
  inflight.set(file, p);
  refreshInflight();
  notify();
  return p;
}
