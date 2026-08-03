import type { ExternalProfile } from "../api/hindsight";

/**
 * 已保存云端配置的显示名。
 *
 * 模型名里通常已经带了厂商（`deepseek-v4-pro` / `kimi-k2.7-code`），再前缀一次
 * provider 就成了「kimi · kimi-k2.7-code」这样的复读。只有当模型名看不出出处时
 * （`gpt-5.6-luna` 之于 openai）才补上 provider。
 *
 * 用**完整** provider 做前缀判断，不取首段——否则 `kimi` 与 `kimi-cn`（国际站 /
 * 国内站）会显示成同一个标签。`kimi-k2.7-code` 不以 `kimi-cn` 开头，于是国内站
 * 那条仍保留前缀，两者可区分。
 */
export function profileLabel(p: ExternalProfile): string {
  const model = p.model.trim();
  const provider = p.provider.trim();
  if (!model) {
    // 没填模型:退回域名,再不行才用原始 endpoint
    try {
      return new URL(p.endpoint).host;
    } catch {
      return p.endpoint;
    }
  }
  if (!provider) return model;
  return model.toLowerCase().startsWith(provider.toLowerCase())
    ? model
    : `${provider} · ${model}`;
}
