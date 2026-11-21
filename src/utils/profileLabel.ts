import type { CloudConnection } from "../api/hindsight";

/**
 * 已保存云端连接的显示名。
 *
 * 模型名里通常已经带了厂商（`deepseek-v4-pro` / `kimi-k2.7-code`），再前缀一次
 * dialect 就成了「kimi · kimi-k2.7-code」这样的复读。只有当模型名看不出出处时
 * （`gpt-5.6-luna` 之于 openai）才补上 dialect。
 */
export function profileLabel(p: Pick<CloudConnection, "dialect" | "endpoint" | "model">): string {
  const model = p.model.trim();
  const dialect = p.dialect.trim();
  if (!model) {
    try {
      return new URL(p.endpoint).host;
    } catch {
      return p.endpoint;
    }
  }
  if (!dialect) return model;
  return model.toLowerCase().startsWith(dialect.toLowerCase()) ? model : `${dialect} · ${model}`;
}
