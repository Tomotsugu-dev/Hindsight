import { useSettings } from "../../../state/settings";
import type { AiConfig } from "../../../api/hindsight";

/**
 * AISettings 各 tab 公用：拿当前 ai 子配置 + 一个 spread-merge 的 updateAi。
 *
 * 必须 spread 旧 ai 一次：useSettings.update 的 patch 是浅合并（顶层），
 * 直接调 update({ ai: { endpoint: v } }) 会把 settings.ai 整个换成 { endpoint: v }，
 * 后端 #[serde(default)] 又会把缺字段填默认值，把用户已存的其他字段擦掉。
 */
export function useAiSettings() {
  const { settings, update, reload } = useSettings();
  const ai = settings?.ai;
  const updateAi = (patch: Partial<AiConfig>) => {
    if (!ai) return;
    update({ ai: { ...ai, ...patch } });
  };
  return { settings, ai, updateAi, reload };
}
