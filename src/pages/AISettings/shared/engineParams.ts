// ────────────────────────────────────────────────────────────────────────
//  引擎参数 picker 选项 ——
//  跟 DebugTab 同款语义（每 slot 的 ctx，--ctx-size 后端会乘 parallel_slots）；
//  这里写的是 settings.ai 全局值，DailyTab 跑日报时引擎按这套参数启动。
// ────────────────────────────────────────────────────────────────────────

export type EngineBatchKey = "default" | "1024" | "2048" | "4096";

// label 纯数值——picker 外面挂的 Row label 已说明语义（"批大小" / "并发" / "上下文"），
// 选项内不需要再重复术语前缀。
export const ENGINE_BATCH_OPTIONS: Array<{ value: EngineBatchKey; label: string }> = [
  { value: "default", label: "512" },
  { value: "1024", label: "1024" },
  { value: "2048", label: "2048" },
  { value: "4096", label: "4096" },
];

export function engineBatchToOption(n: number | null): EngineBatchKey {
  if (n === 1024) return "1024";
  if (n === 2048) return "2048";
  if (n === 4096) return "4096";
  return "default";
}

export function engineOptionToBatch(v: EngineBatchKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}

export type EngineSlotsKey = "1" | "2" | "4" | "8";

export const ENGINE_SLOTS_OPTIONS: Array<{ value: EngineSlotsKey; label: string }> = [
  { value: "1", label: "1" },
  { value: "2", label: "2" },
  { value: "4", label: "4" },
  { value: "8", label: "8" },
];

export function engineSlotsToOption(n: number | null): EngineSlotsKey {
  if (n != null && n >= 8) return "8";
  if (n != null && n >= 4) return "4";
  if (n != null && n >= 2) return "2";
  return "1";
}

export function engineOptionToSlots(v: EngineSlotsKey): number | null {
  const n = parseInt(v, 10);
  return n <= 1 ? null : n;
}

export type EngineCtxKey = "default" | "16384" | "32768" | "65536";

export const ENGINE_CTX_OPTIONS: Array<{ value: EngineCtxKey; label: string }> = [
  { value: "default", label: "8K" },
  { value: "16384", label: "16K" },
  { value: "32768", label: "32K" },
  { value: "65536", label: "64K" },
];

export function engineCtxToOption(n: number | null): EngineCtxKey {
  if (n === 16384) return "16384";
  if (n === 32768) return "32768";
  if (n === 65536) return "65536";
  return "default";
}

export function engineOptionToCtx(v: EngineCtxKey): number | null {
  return v === "default" ? null : parseInt(v, 10);
}

/** 估算当前引擎参数组合下的总 VRAM / RAM 占用（GB）。
 *  参数量从 active_main 文件名抠 "NB"，
 *  Q4 经验比例：weights ≈ B × 0.55 GB，kv ≈ B × 18 KB/token × ctx_total。
 *  误差 ±20%，仅作 OOM 早期警告用。 */
export function estimateVramGB(
  modelName: string,
  parallelSlots: number,
  ctxSize: number,
): { totalGB: number; weightsGB: number; kvGB: number; params: number } {
  const m = modelName.match(/(\d+(?:\.\d+)?)\s*B/i);
  const params = m ? parseFloat(m[1]) : 4;
  const weightsGB = params * 0.55;
  const kvPerTokenKB = 18 * params;
  const totalCtx = ctxSize * Math.max(1, parallelSlots);
  const kvGB = (kvPerTokenKB * totalCtx) / 1024 / 1024;
  const overheadGB = 2;
  return {
    totalGB: weightsGB + kvGB + overheadGB,
    weightsGB,
    kvGB,
    params,
  };
}

/** 平台变体 ID → 人话加速类型标签 */
export function humanAccelLabel(
  platformId: string,
  t: (key: string) => string,
): string {
  switch (platformId) {
    case "win-cuda-12.4-x64":
      return t("aiSettings.engine.accel.cuda12");
    case "win-cuda-13.1-x64":
      return t("aiSettings.engine.accel.cuda13");
    case "win-cpu-x64":
      return t("aiSettings.engine.accel.winCpu");
    case "macos-arm64":
      return t("aiSettings.engine.accel.macArm");
    case "macos-x64":
      return t("aiSettings.engine.accel.macIntel");
    case "ubuntu-x64":
      return t("aiSettings.engine.accel.linuxCpu");
    default:
      return platformId;
  }
}
