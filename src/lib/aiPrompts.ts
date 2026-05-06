/** AI 总结的内置 system prompt 元数据（前端用，给 UI 编辑器展示默认值 / 重置回填）。
 *
 * ## 数据 vs 代码
 *
 * 三套语言的 prompt 文本是 *数据*，权威源在 [src-tauri/resources/prompts/](../../src-tauri/resources/prompts/)，
 * 后端通过 `include_str!` 编译时嵌入；前端通过 Vite `?raw` import 复用同一份文件，
 * 编译时嵌入 bundle——零运行时读盘，发布产物自带，改 prompt 内容只动 `.md` 文件，
 * 不动 `.ts` 代码。前后端共用同一权威源，避免双副本漂移。 */

import type { PromptLanguage } from "../api/hindsight";
import zhText from "../../src-tauri/resources/prompts/system_zh.md?raw";
import enText from "../../src-tauri/resources/prompts/system_en.md?raw";
import jaText from "../../src-tauri/resources/prompts/system_ja.md?raw";
import zhImageText from "../../src-tauri/resources/prompts/image_describe_zh.md?raw";
import enImageText from "../../src-tauri/resources/prompts/image_describe_en.md?raw";
import jaImageText from "../../src-tauri/resources/prompts/image_describe_ja.md?raw";

/** 内置默认 system prompt（step 2 段总结）——按语言索引。 */
export const DEFAULT_SYSTEM_PROMPTS: Record<PromptLanguage, string> = {
  zh: zhText.trimEnd(),
  en: enText.trimEnd(),
  ja: jaText.trimEnd(),
};

/** 内置默认 image describe prompt（step 1 单图描述）——按语言索引。 */
export const DEFAULT_IMAGE_DESCRIBE_PROMPTS: Record<PromptLanguage, string> = {
  zh: zhImageText.trimEnd(),
  en: enImageText.trimEnd(),
  ja: jaImageText.trimEnd(),
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
