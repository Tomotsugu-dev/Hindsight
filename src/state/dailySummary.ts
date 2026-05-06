/**
 * 日报生成（source=daily）的全局状态：在跑标志 + listener + 命令包装。
 *
 * 修跟模型下载同款的 bug：原来"运行中"标志（generating / runningIdx /
 * enginePhase / topError）和 `SUMMARY_PROGRESS_EVENT` listener 都在 DailyTab
 * 内部。切侧边栏让 DailyTab unmount → useEffect cleanup 把 listener unlisten 了，
 * 后端继续发的 progress 事件没人接；切回来 generating 是 false，按钮显示成
 * "开始总结"，用户再点会触发后端并发跑一次。
 *
 * 提到 module level：
 * - listener 单例化、永不解除——app 整个生命周期持续监听
 * - "在跑"状态切走再回来仍准确，按钮、进度行、运行中段下标都自动恢复
 * - segment_done 事件通过单独的回调 set 派发给当前 mount 的 DailyTab，
 *   让它把 row 落到本地 rows Map 里（rows 是 per-date 的，不进 store）
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import i18next from "i18next";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type SummaryProgress,
} from "../api/hindsight";
import { logWarn } from "../lib/logger";

export interface DailyRunningSnapshot {
  /** 是否有 daily run 正在跑。按钮"停止"<->"开始"切换的根。 */
  generating: boolean;
  /** 在跑的日期；DailyTab 切日期时用来判断"我现在看的这天是不是在跑的那天"，
   *  避免在 view 别的日期时也显示"停止"按钮。 */
  runningDate: string | null;
  /** 当前跑到的段下标；engine_starting 阶段为 null。 */
  runningIdx: number | null;
  /** 当前段图数（segment_started 给）。 */
  runningImages: number | null;
  /** 当前段已完成的逐图描述数（image_described 累加）。
   *  segment_started 时清零、新段开跑也清零；用来给 UI 渲染 "X / Y 张"。 */
  runningDone: number;
  /** 引擎冷启动提示文案；非 null 时段卡上方会显示一行 hint。 */
  enginePhase: string | null;
  /** 顶层错误（all_done / segment_done 不算）；null = 无错。
   *  DailyTab 用 useSyncExternalStore 读，组件展示后调 clearTopError 清。 */
  topError: string | null;
}

const EMPTY_SNAP: DailyRunningSnapshot = Object.freeze({
  generating: false,
  runningDate: null,
  runningIdx: null,
  runningImages: null,
  runningDone: 0,
  enginePhase: null,
  topError: null,
});

let snap: DailyRunningSnapshot = EMPTY_SNAP;
const listeners = new Set<() => void>();

type SegmentDoneCallback = (ev: SummaryProgress) => void;
const segmentDoneListeners = new Set<SegmentDoneCallback>();

let listenerInit: Promise<UnlistenFn> | null = null;

function notify(): void {
  listeners.forEach((cb) => cb());
}

function ensureListener(): void {
  if (listenerInit) return;
  listenerInit = listen<SummaryProgress>(SUMMARY_PROGRESS_EVENT, (ev) => {
    const p = ev.payload;
    // 只接 daily source；调试 tab 的事件 (source="debug") 走它自己的 listener
    if (p.source !== "daily") return;

    switch (p.phase) {
      case "engine_starting":
        snap = Object.freeze({
          ...snap,
          generating: true,
          runningDate: p.date,
          enginePhase: p.message ?? i18next.t("aiSummary.daily.engineLoading"),
        });
        notify();
        break;
      case "segment_started":
        snap = Object.freeze({
          ...snap,
          generating: true,
          runningDate: p.date,
          enginePhase: null,
          runningIdx: p.segmentIdx ?? null,
          runningImages: p.imagesTotal ?? null,
          runningDone: 0,
        });
        notify();
        break;
      case "image_described":
        // 段内每完成一张图 +1；buffer_unordered 完成顺序不可预期，
        // 用计数累加而不是从 imageIndex 推（imageIndex 不是单调递增的）
        snap = Object.freeze({
          ...snap,
          runningDone: snap.runningDone + 1,
        });
        notify();
        break;
      case "segment_done":
        // segment_done 不变 generating（后面可能还有段要跑）；只清 runningIdx。
        // 派发给当前 mount 的 DailyTab 让它 setRows 落库这一段。
        segmentDoneListeners.forEach((cb) => cb(p));
        snap = Object.freeze({
          ...snap,
          runningIdx: null,
          runningImages: null,
          runningDone: 0,
        });
        notify();
        break;
      case "all_done":
      case "cancelled":
        snap = Object.freeze({
          ...EMPTY_SNAP,
          // 保留 topError——若用户切走的是错误前一刻，回来还能看到
          topError: snap.topError,
        });
        notify();
        break;
      case "error":
        snap = Object.freeze({
          ...EMPTY_SNAP,
          topError: p.message ?? i18next.t("aiSummary.daily.errors.generationFailed"),
        });
        notify();
        break;
    }
  });
}

export function subscribeDailySummary(cb: () => void): () => void {
  ensureListener();
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export function getDailyRunningSnapshot(): DailyRunningSnapshot {
  return snap;
}

/** DailyTab 注册段完成回调；listener 卸载时自动清理。 */
export function subscribeSegmentDone(cb: SegmentDoneCallback): () => void {
  ensureListener();
  segmentDoneListeners.add(cb);
  return () => {
    segmentDoneListeners.delete(cb);
  };
}

export function clearTopError(): void {
  if (snap.topError != null) {
    snap = Object.freeze({ ...snap, topError: null });
    notify();
  }
}

export function setTopError(msg: string): void {
  snap = Object.freeze({
    ...snap,
    topError: msg,
    generating: false,
    runningDate: null,
    enginePhase: null,
    runningIdx: null,
    runningImages: null,
    runningDone: 0,
  });
  notify();
}

/**
 * 启动一次 daily run。
 *
 * invoke 前先乐观置 generating=true（让按钮立刻变"停止"，不等首个事件来）；
 * invoke 抛错时回滚并设 topError。成功就交给事件流接管 generating 复位。
 */
export async function startDailyGenerate(
  date: string,
  forceRefresh: boolean,
): Promise<void> {
  ensureListener();
  snap = Object.freeze({
    ...EMPTY_SNAP,
    generating: true,
    runningDate: date,
  });
  notify();
  try {
    await api.generateDaySummary(date, forceRefresh, null);
    // generateDaySummary 在 all_done 后 resolve；事件流已经把 generating 置回 false
  } catch (e) {
    setTopError(typeof e === "string" ? e : String(e));
    throw e;
  }
}

/** 停止当前 daily run；后端会在下一段开跑前检测 cancel 标志退出，
 *  期间若有段已经在跑得 chat 必须等它跑完才能 yield。 */
export async function cancelDailyGenerate(): Promise<void> {
  try {
    await api.cancelDaySummary();
  } catch (e) {
    logWarn("dailySummary.cancel", e);
  }
}

/**
 * 重试单段。后端 `run_one_segment_only` 不发 all_done 事件，只发 segment_started
 * / segment_done——这里需要在 await 完手动复位 generating（事件流不会替它复位）。
 */
export async function retryDailySegment(
  date: string,
  segmentIdx: number,
): Promise<void> {
  ensureListener();
  snap = Object.freeze({
    ...EMPTY_SNAP,
    generating: true,
    runningDate: date,
  });
  notify();
  try {
    await api.retrySummarySegment(date, segmentIdx, null);
    // retry 后端不发 all_done；这里命令 resolve 时显式复位
    snap = Object.freeze({ ...EMPTY_SNAP, topError: snap.topError });
    notify();
  } catch (e) {
    setTopError(typeof e === "string" ? e : String(e));
    throw e;
  }
}
