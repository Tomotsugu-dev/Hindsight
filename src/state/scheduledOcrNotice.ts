/**
 * 定时补识别的系统通知:后端定时批开跑时 emit
 * `memory://scheduled-ocr-started`,这里弹一条系统级通知——
 * 后台 OCR 是有感的 CPU 活动,静默跑是"风扇为什么转"类困惑的温床。
 *
 * 放 module level(与 dailySummary 的 listener 同款理由):
 * 通知不依赖任何页面 mount,窗口收进托盘时也要能弹。
 * 文案走 i18n(后端不知道界面语言,所以由前端负责弹)。
 */

import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import i18next from "i18next";
import { logWarn } from "../lib/logger";

const SCHEDULED_OCR_STARTED_EVENT = "memory://scheduled-ocr-started";

let inited = false;

/** 应用启动时调用一次;重复调用 no-op。 */
export function initScheduledOcrNotice(): void {
  if (inited) return;
  inited = true;
  void listen<{ pending: number }>(SCHEDULED_OCR_STARTED_EVENT, async (ev) => {
    try {
      let granted = await isPermissionGranted();
      if (!granted) {
        granted = (await requestPermission()) === "granted";
      }
      if (!granted) return; // 用户拒绝授权:静默,不骚扰
      sendNotification({
        title: i18next.t("notifications.scheduledOcr.title"),
        body: i18next.t("notifications.scheduledOcr.body", {
          count: ev.payload.pending,
        }),
      });
    } catch (e) {
      logWarn("scheduledOcrNotice", e);
    }
  });
}
