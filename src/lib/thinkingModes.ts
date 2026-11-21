/**
 * 思考模式能选的档位。和后端 dialect 的 effective_reasoning 一致，两边一起改：
 * - deepseek：low / high / max，没有 medium；
 * - kimi、openrouter：low / medium / high；
 * - openai：发了工具就不能思考，对话只剩「关闭」；日报、周报不发工具，有 low / medium / high；
 * - 本地和其它云端：只有开关，high 显示成「开启」。
 */
export function thinkingModes(provider: string | null, withTools: boolean): string[] {
  switch (provider) {
    case "deepseek":
      return ["auto", "low", "high", "max", "off"];
    case "kimi":
    case "kimi-cn":
    case "openrouter":
      return ["auto", "low", "medium", "high", "off"];
    case "openai":
      return withTools ? ["off"] : ["auto", "low", "medium", "high", "off"];
    default:
      return ["auto", "high", "off"];
  }
}

/**
 * 换了服务商后，存着的档位可能不在新的表里：强度类就近取 high（后端也这样降级），
 * 否则取 auto，再没有就取第一档。
 */
export function normalizeThinking(raw: string, modes: string[]): string {
  if (modes.includes(raw)) return raw;
  if (["low", "medium", "high", "max", "on"].includes(raw) && modes.includes("high")) return "high";
  return modes.includes("auto") ? "auto" : modes[0];
}

/** 档位的文案 key。只有开关的服务商没有强度，high 叫「开启」。 */
export function thinkingLabelKey(mode: string, modes: string[]): string {
  return mode === "high" && !hasLevels(modes)
    ? "chat.thinkingMode.on"
    : `chat.thinkingMode.${mode}`;
}

/** 有没有强度可选（而不只是开、关）。 */
export function hasLevels(modes: string[]): boolean {
  return modes.length > 3;
}
