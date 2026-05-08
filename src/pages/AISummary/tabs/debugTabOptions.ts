// SimplePicker 下拉选项映射 + 类型，跨 DebugTab / DebugSettingsTab 共用。
//
// 抽出原因：DebugTab.tsx 和 DebugStateContext.tsx 同文件混合 component 与
// 非 component 导出会破坏 Vite Fast Refresh（react-refresh/only-export-components）。
// 这里只放纯 TS 工具——picker 选项构造、key↔后端数值的映射——保持 .ts 后缀，
// 让 DebugTab.tsx 自身只导出 React 组件。

import type { TFunction } from "i18next";

/** 单段最大图片数选项。"max" 映射到 sanitize 上限 100000，"无限制"等于把段内
 *  所有截图全送给 LLM——撑爆 ctx 时 LLM 会返 400 被段标 error，用户回头再调小。 */
export type MaxImagesKey = "15" | "30" | "max";
export function buildMaxImagesOptions(
  t: TFunction,
): Array<{ value: MaxImagesKey; label: string }> {
  return [
    { value: "15", label: t("aiSummary.debug.pickerOptions.maxImages15") },
    { value: "30", label: t("aiSummary.debug.pickerOptions.maxImages30") },
    { value: "max", label: t("aiSummary.debug.pickerOptions.maxImagesUnlimited") },
  ];
}

export function maxImagesToOption(n: number): MaxImagesKey {
  // 1000 起算"无限制"——这种大值正常路径不会出现，只有用户主动选 max 才会写入
  if (n >= 1000) return "max";
  if (n >= 30) return "30";
  return "15";
}
export function optionToMaxImages(v: MaxImagesKey): number {
  if (v === "max") return 100_000;
  return parseInt(v, 10);
}

/** llama-server `--batch-size` / `--ubatch-size`。"default" = 不传，走 llama.cpp 默认 512。
 *  改值会触发引擎 stop+start 重启；调试跑完无条件 stop，下次正常日报跑回到默认。 */
export type BatchKey = "default" | "1024" | "2048" | "4096";
// Batch 选项纯英文 + 数字，所有语言都保持一致，无需走 t()
export const BATCH_OPTIONS: Array<{ value: BatchKey; label: string }> = [
  { value: "default", label: "Batch 512" },
  { value: "1024", label: "Batch 1024" },
  { value: "2048", label: "Batch 2048" },
  { value: "4096", label: "Batch 4096" },
];
export function batchToOption(n: number | null): BatchKey {
  if (n === 1024) return "1024";
  if (n === 2048) return "2048";
  if (n === 4096) return "4096";
  return "default";
}
/** "default" → null（让 overrides.batchSize 留空，后端走默认）；其它 → 数值 */
export function optionToBatch(v: BatchKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}

/** 并发槽位数 = llama-server `-np` + 后端 step 1 image describe 并发数。
 *  两边一致才有效。"1" = 串行（历史行为）；> 1 = 并发同时跑 N 张图描述。 */
export type SlotsKey = "1" | "2" | "4" | "8";
export function buildSlotsOptions(
  t: TFunction,
): Array<{ value: SlotsKey; label: string }> {
  return [
    { value: "1", label: t("aiSummary.debug.pickerOptions.slots1") },
    { value: "2", label: t("aiSummary.debug.pickerOptions.slots2") },
    { value: "4", label: t("aiSummary.debug.pickerOptions.slots4") },
    { value: "8", label: t("aiSummary.debug.pickerOptions.slots8") },
  ];
}
export function slotsToOption(n: number): SlotsKey {
  if (n >= 8) return "8";
  if (n >= 4) return "4";
  if (n >= 2) return "2";
  return "1";
}
export function optionToSlots(v: SlotsKey): number {
  return parseInt(v, 10);
}

/** 每 slot 的 ctx 上限（单位 token）。后端 `--ctx-size = ctxSize × slots`。
 *  启动时按总量一次性吃 KV cache（~30KB / token）；选大了 5090 也只是几 GB，
 *  CPU 用户保持「默认 (8K)」就行。 */
export type CtxKey = "default" | "16384" | "32768" | "65536";
export function buildCtxOptions(
  t: TFunction,
): Array<{ value: CtxKey; label: string }> {
  return [
    { value: "default", label: t("aiSummary.debug.pickerOptions.ctxDefault") },
    { value: "16384", label: t("aiSummary.debug.pickerOptions.ctx16k") },
    { value: "32768", label: t("aiSummary.debug.pickerOptions.ctx32k") },
    { value: "65536", label: t("aiSummary.debug.pickerOptions.ctx64k") },
  ];
}
export function ctxToOption(n: number | null): CtxKey {
  if (n === 16384) return "16384";
  if (n === 32768) return "32768";
  if (n === 65536) return "65536";
  return "default";
}
/** "default" → null（让 overrides.ctxSize 留空，后端走 8K 默认）；其它 → 数值 */
export function optionToCtx(v: CtxKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}
