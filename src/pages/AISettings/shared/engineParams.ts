// ────────────────────────────────────────────────────────────────────────
//  引擎参数 picker 选项 ——
//  跟 DebugTab 同款语义（每 slot 的 ctx，--ctx-size 后端会乘 parallel_slots）；
//  这里写的是 settings.ai 全局值，DailyTab 跑日报时引擎按这套参数启动。
// ────────────────────────────────────────────────────────────────────────

import type { VramInfo } from "../../../api/hindsight";

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

/** "系统总量" → "实际可用做推理的预算"（GB）。
 *  - discrete (NVIDIA)：显存几乎全可用，1.0
 *  - unified (Apple)：留 30% 给系统 + 其它进程，0.7（业界惯例）
 *  UI 始终显示 `vi.totalGb` 原始值（24 GB 就是 24 GB）；只有 OOM 判断 /
 *  推荐参数算法里才走这个 helper 折算。 */
export function effectiveVramGB(vi: VramInfo): number {
  return vi.source === "unified" ? vi.totalGb * 0.7 : vi.totalGb;
}

/** OOM 风险等级——`estimateVramGB` 跟系统 VRAM 对照后给的红绿灯。
 *  阈值：< 60% 安全；60–85% 接近上限；> 85% 可能 OOM。
 *  传入的 `effectiveSystemGB` 应来自 `effectiveVramGB(systemVram)`，
 *  系统 VRAM 未知（CPU-only 机器）→ 返 null，前端跳过红绿灯展示。 */
export type VramRisk = "safe" | "near" | "danger";

export function classifyVramRisk(
  estimateGB: number,
  effectiveSystemGB: number | null,
): VramRisk | null {
  if (effectiveSystemGB == null || effectiveSystemGB <= 0) return null;
  const ratio = estimateGB / effectiveSystemGB;
  if (ratio < 0.6) return "safe";
  if (ratio < 0.85) return "near";
  return "danger";
}

/** 一键最优引擎参数推荐结果。三个字段跟 `settings.ai.batchSize / parallelSlots / ctxSize`
 *  一一对应，`null` 表示用 llama.cpp 默认值（保持 settings 字段空状态）。 */
export interface RecommendedEngineParams {
  batchSize: number | null;
  parallelSlots: number;
  ctxSize: number | null;
  /** UI 文案分支：noGpu = 没显存 / Mac Intel；tightFit = 显存装不下；computed = 正常算出来。 */
  rationale: "noGpu" | "tightFit" | "computed";
}

/** 推荐算法的"阶段模式"：
 *  - "describe": 图描述（多图并行）→ slots 优先，ctx 中等够用即可
 *  - "summary": 段总结（单段串行）→ slots 强制 1，ctx 拉满 */
export type EngineRecommendMode = "describe" | "summary";

/**
 * 算"对当前硬件 + 模型而言最快又安全"的引擎参数组合。
 *
 * 算法核心：
 * 1. 留 35% 显存 margin 给 vision encoder 临时峰值 + 系统进程
 * 2. (slots, ctx) 在网格里挑最大化加权 log 分数的可行组合：
 *    - describe 模式：`log2(slots) + 0.4×log2(ctx/8K)`（slots 权重高）
 *    - summary 模式：`SLOTS=[1]`，仅挑 ctx 最大可行（slots 强制 1）
 * 3. batch 单独决策：discrete 24GB+ 给 2048、12GB+ 给 1024、其它默认；
 *    Apple unified 不上大 batch（Metal 收益小、内存峰值高 30%）
 *
 * 不做的事：不预测 quantization（假设 Q4_K_M）、不看 SM 数 / 算力等级、
 * 不自动覆盖用户已设值（只在用户按"应用推荐"时才生效）。
 */
export function recommendEngineParams(
  systemVram: VramInfo | null,
  modelName: string,
  platformId: string | undefined,
  mode: EngineRecommendMode = "describe",
): RecommendedEngineParams {
  // 抠模型参数量；正则跟 estimateVramGB 保持一致
  const m = modelName.match(/(\d+(?:\.\d+)?)\s*B/i);
  const params = m ? parseFloat(m[1]) : 4;

  const cpuOnly =
    !systemVram ||
    platformId === "win-cpu-x64" ||
    platformId === "ubuntu-x64" ||
    platformId === "macos-x64";
  if (cpuOnly) {
    return {
      batchSize: null,
      parallelSlots: 1,
      ctxSize: null,
      rationale: "noGpu",
    };
  }

  // 用 effectiveVramGB 折算（unified × 0.7、discrete 不打折），再留 35% margin
  const targetGB = effectiveVramGB(systemVram!) * 0.65;
  const weightsGB = params * 0.55;
  const overheadGB = 2;
  const kvBudgetGB = Math.max(targetGB - weightsGB - overheadGB, 0);
  if (kvBudgetGB <= 0) {
    return {
      batchSize: null,
      parallelSlots: 1,
      ctxSize: null,
      rationale: "tightFit",
    };
  }

  // 网格搜索：summary 模式只考虑 slots=1；describe 模式 slots 权重更高
  const SLOTS = mode === "summary" ? [1] : [8, 4, 2, 1];
  const CTXES = [65536, 32768, 16384, 8192];
  // describe: slots 优先（权重 1.0），ctx 中等（0.4）；summary: slots 锁 1，仅 ctx 起作用
  const ctxWeight = mode === "summary" ? 1.0 : 0.4;
  let bestSlots = 1;
  let bestCtx = 8192;
  let bestScore = 0;
  for (const slots of SLOTS) {
    for (const ctx of CTXES) {
      // KV cache（GB）= params × 18 KB/token × ctx_total / 1024 / 1024
      const kvGB = (params * 18 * ctx * slots) / (1024 * 1024);
      if (kvGB <= kvBudgetGB) {
        const score = Math.log2(slots) + ctxWeight * Math.log2(ctx / 8192);
        if (score > bestScore) {
          bestSlots = slots;
          bestCtx = ctx;
          bestScore = score;
        }
      }
    }
  }

  // batch：仅 NVIDIA discrete 上分档；Apple unified / 其它默认
  let batchSize: number | null = null;
  if (systemVram!.source === "discrete") {
    if (systemVram!.totalGb >= 24) batchSize = 2048;
    else if (systemVram!.totalGb >= 12) batchSize = 1024;
  }

  return {
    batchSize,
    parallelSlots: bestSlots,
    ctxSize: bestCtx === 8192 ? null : bestCtx,
    rationale: "computed",
  };
}

/** 推荐参数跟当前 settings 三个字段对比是否完全相同，决定按钮 disabled 与否。
 *  `null` 等价比较——`batchSize: null` 跟 `settings.ai.batchSize` 都是 null 时算相同。 */
export function isRecommendedApplied(
  recommended: RecommendedEngineParams,
  current: {
    batchSize: number | null;
    parallelSlots: number | null;
    ctxSize: number | null;
  },
): boolean {
  // parallelSlots 在推荐里恒为 number（1..8），settings 里 null 等价于 1
  const currentSlots = current.parallelSlots ?? 1;
  return (
    recommended.batchSize === current.batchSize &&
    recommended.parallelSlots === currentSlots &&
    recommended.ctxSize === current.ctxSize
  );
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
