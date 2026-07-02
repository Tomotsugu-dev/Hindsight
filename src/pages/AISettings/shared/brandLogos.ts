// 品牌 → 本地 SVG logo 映射。SVG 取自 lobehub/lobe-icons（MIT）。
// catalog 的 logoUrl 是 HF org 头像网络直链——加载慢、可能挂、风格不一；
// 本地映射命中时优先用，没命中的品牌（如个人微调作者）回退 logoUrl。
import deepseekLogo from "../../../assets/model-logos/deepseek.svg";
import gemmaLogo from "../../../assets/model-logos/gemma.svg";
import openaiLogo from "../../../assets/model-logos/openai.svg";
import qwenLogo from "../../../assets/model-logos/qwen.svg";
import zaiLogo from "../../../assets/model-logos/zai.svg";

/** key = catalog 的 brand 字段原值。Google 品牌下全是 Gemma 模型，用 Gemma 菱形更好认。 */
export const BRAND_LOGOS: Record<string, string> = {
  DeepSeek: deepseekLogo,
  Google: gemmaLogo,
  OpenAI: openaiLogo,
  Qwen: qwenLogo,
  "Z.AI": zaiLogo,
};
