/**
 * 周报生成（source=weekly）的全局状态：在跑标志 + listener + 命令包装。
 *
 * 跟 daily store 同款架构（参 [`./dailySummary.ts`]）：listener 提到 module level
 * 单例，跨 tab unmount/mount 不丢；切走再回来仍能看到正确的"在跑"状态。
 *
 * 周报跟日报的关键差异：
 * - 没有"段"概念——跑一次 = 一行，进度只关心 engine_starting → summarizing → segment_done → all_done
 * - source="weekly" 跟 daily 在 listener 里 if-source 分流，互不干扰
 * - 取消信号跟 daily 共用全局 SummaryCancel；前端 UI 应保证两个 tab 不并发触发
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import i18next from "i18next";
import {
  api,
  SUMMARY_PROGRESS_EVENT,
  type SegmentSummaryRow,
  type SummaryProgress,
} from "../api/hindsight";
import { logWarn } from "../lib/logger";

/** 周报当前的子阶段：
 *  - "idle"：空闲（没在跑）
 *  - "engine_starting"：本地 llama-server 冷启动加载模型中
 *  - "summarizing"：step 2 单次 chat 在跑（无进度条，只有 spinner） */
export type WeeklyStage = "idle" | "engine_starting" | "summarizing";

export interface WeeklyRunningSnapshot {
  /** 是否有 weekly run 正在跑。按钮"停止"<->"开始"切换的根。 */
  generating: boolean;
  /** 在跑的周（周一日期 "YYYY-MM-DD"）；切周时用来判断"我现在看的这周是不是在跑的那周"。 */
  runningWeek: string | null;
  /** 引擎冷启动 / chat 中——决定卡片 body 文案。 */
  stage: WeeklyStage;
  /** 引擎冷启动提示文案；非 null 时卡片上方会显示一行 hint。 */
  enginePhase: string | null;
  /** 顶层错误；null = 无错。组件用 useSyncExternalStore 读，UI 调 clearTopError 清。 */
  topError: string | null;
}

const EMPTY_SNAP: WeeklyRunningSnapshot = Object.freeze({
  generating: false,
  runningWeek: null,
  stage: "idle" as WeeklyStage,
  enginePhase: null,
  topError: null,
});

let snap: WeeklyRunningSnapshot = EMPTY_SNAP;
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
    // 只接 weekly source；daily / debug 各走自己的 listener
    if (p.source !== "weekly") return;

    switch (p.phase) {
      case "engine_starting":
        snap = Object.freeze({
          ...snap,
          generating: true,
          runningWeek: p.date,
          stage: "engine_starting",
          enginePhase: p.message ?? i18next.t("aiSummary.weekly.engineLoading"),
        });
        notify();
        break;
      case "summarizing":
        snap = Object.freeze({
          ...snap,
          generating: true,
          runningWeek: p.date,
          stage: "summarizing",
          enginePhase: null,
        });
        notify();
        break;
      case "segment_done":
        // 派发给当前 mount 的 WeeklyTab 让它把 row 落到本地 state
        segmentDoneListeners.forEach((cb) => cb(p));
        // 不变 generating——after-segment-done 后端紧跟着发 all_done 收尾
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
          topError: p.message ?? i18next.t("aiSummary.weekly.errors.generationFailed"),
        });
        notify();
        break;
      // 其他 daily 专属阶段（dedup_running / segment_started / image_described / step1_done）
      // weekly 路径不会发出，到这里直接忽略
    }
  });
}

export function subscribeWeeklySummary(cb: () => void): () => void {
  ensureListener();
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export function getWeeklyRunningSnapshot(): WeeklyRunningSnapshot {
  return snap;
}

/** WeeklyTab 注册 segment_done 回调；listener 卸载时自动清理。 */
export function subscribeWeeklyDone(cb: SegmentDoneCallback): () => void {
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
    runningWeek: null,
    stage: "idle",
    enginePhase: null,
  });
  notify();
}

/**
 * 启动一次 weekly run。
 *
 * invoke 前先乐观置 generating=true（让按钮立刻变"停止"，不等首个事件来）；
 * invoke 抛错时回滚并设 topError。成功就交给事件流接管 generating 复位。
 */
export async function startWeeklyGenerate(
  weekStart: string,
  forceRefresh: boolean,
): Promise<void> {
  ensureListener();
  snap = Object.freeze({
    ...EMPTY_SNAP,
    generating: true,
    runningWeek: weekStart,
    stage: "engine_starting",
  });
  notify();
  try {
    await api.generateWeekSummary(weekStart, forceRefresh);
    // generateWeekSummary 在 all_done 后 resolve；事件流已经把 generating 置回 false
  } catch (e) {
    setTopError(typeof e === "string" ? e : String(e));
    throw e;
  }
}

/** 停止当前 weekly run。复用 daily 的全局 cancel signal——后端 chat 已在路上时
 *  无法中断（一次 LLM 调用 30-180s），只能等它跑完。 */
export async function cancelWeeklyGenerate(): Promise<void> {
  try {
    await api.cancelDaySummary();
  } catch (e) {
    logWarn("weeklySummary.cancel", e);
  }
}

/** 拉某周已落库行。前端 useEffect(date) 时调一次。 */
export async function fetchWeeklyRow(
  weekStart: string,
): Promise<SegmentSummaryRow | null> {
  return api.getWeekSummary(weekStart);
}
