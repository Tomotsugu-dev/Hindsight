import { SUMMARY_CLOUD_SENTINEL, type AiConfig } from "../../api/hindsight";

/** 云端三元组是否配齐（enabled + endpoint + model 缺一不可）。 */
export function chatCloudReady(ai: AiConfig): boolean {
  return ai.externalEnabled && ai.endpoint.trim() !== "" && ai.model.trim() !== "";
}

/**
 * Chat 路由判定的前端镜像——必须与后端 AiConfig::chat_use_cloud 逐字对齐：
 * chatMain 显式本地文件名 → 永远本地；空（自动）或 sentinel → 云端配齐即云端。
 */
export function chatUsesCloud(ai: AiConfig): boolean {
  const c = ai.chatMain.trim();
  if (c !== "" && c !== SUMMARY_CLOUD_SENTINEL) return false;
  return chatCloudReady(ai);
}

/**
 * 本地路径实际加载的模型文件名，镜像后端 AiConfig::effective_chat_main：
 * chatMain 显式文件名优先；否则同 step 2 的 fallback 链
 * （summaryMain 空或 sentinel → activeMain）。
 * 返回空串 = 没有可用本地模型（后端会报"需要一个语言模型"）。
 */
export function chatLocalModelName(ai: AiConfig): string {
  const c = ai.chatMain.trim();
  if (c !== "" && c !== SUMMARY_CLOUD_SENTINEL) return c;
  const s = ai.summaryMain.trim();
  return s === "" || s === SUMMARY_CLOUD_SENTINEL ? ai.activeMain.trim() : s;
}
