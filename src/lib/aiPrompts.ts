/** AI 总结的内置 system prompt 元数据（前端用，给 UI 编辑器展示默认值 / 重置回填）。
 *
 * ## 数据 vs 代码
 *
 * 三套语言的 prompt 文本是 *数据*，存在 [src/lib/prompts/](./prompts/)。通过 Vite
 * `?raw` import 编译时嵌入 bundle——零运行时读盘，发布产物自带，改 prompt 内容只
 * 动 `.md` 文件，不动 `.ts` 代码。
 *
 * 后端 `src-tauri/resources/prompts/` 维护一份对应的副本，供 LLM 实际生成时使用。
 * 前后端各自管自己的资源边界，**不**跨 crate 反向引用——保持模块边界干净。同步
 * 靠 commit 时人眼 diff，未来必要时上 CI check 文本一致。 */

import type { PromptLanguage } from "../api/hindsight";
import zhText from "./prompts/system_zh.md?raw";
import enText from "./prompts/system_en.md?raw";
import jaText from "./prompts/system_ja.md?raw";

/** 内置默认 system prompt——按语言索引，源文件在 ./prompts/system_<lang>.md。
 *  当前应用语言由 i18n 决定（暂未接入），settings.ai.promptLanguage 跟着自动切换。 */
export const DEFAULT_SYSTEM_PROMPTS: Record<PromptLanguage, string> = {
  zh: zhText.trimEnd(),
  en: enText.trimEnd(),
  ja: jaText.trimEnd(),
};

/** 把 PromptLanguage 映射到 PromptOverrides 的字段名。 */
export function overrideKey(lang: PromptLanguage): "systemZh" | "systemEn" | "systemJa" {
  switch (lang) {
    case "zh":
      return "systemZh";
    case "en":
      return "systemEn";
    case "ja":
      return "systemJa";
  }
}
