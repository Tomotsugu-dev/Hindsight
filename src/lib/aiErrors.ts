/** 后端 AI 错误码 → 本地化用户文案。
 *
 * 后端（llm.rs 等）把"内容为空"这类成因发成稳定错误码，形如 `[LLM_EMPTY_REASONING] ...`，
 * 码后面跟一段纯英文技术摘要（给日志 / 兜底用）。前端识别 `[CODE]` 前缀，换成本地化的
 * "为什么 + 怎么办"短句；不认识的码 / 无前缀的原始错误按原文显示，保证不丢信息。
 *
 * 跟 sync 引擎的 `[CRED_EXPIRED]` / `[TRANSIENT]` 前缀码同一套思路：后端给机器可读的码，
 * 前端负责本地化展示，避免把后端的硬编码语言 / token 术语直接糊给用户。 */

import i18next from "i18next";

/** 错误码 → i18n key。新增后端错误码时在这里登记即可。 */
const CODE_KEYS: Record<string, string> = {
  LLM_EMPTY_REASONING: "aiSummary.errors.llmEmptyReasoning",
  LLM_EMPTY_EOS: "aiSummary.errors.llmEmptyEos",
  LLM_EMPTY_TRUNCATED: "aiSummary.errors.llmEmptyTruncated",
  LLM_EMPTY: "aiSummary.errors.llmEmpty",
  AI_RUN_BUSY: "aiSummary.errors.runBusy",
};

/** 把后端原始错误串映射成本地化文案：命中已知 `[CODE]` 码走 i18n，否则原样返回。
 *
 *  正则**不锚定开头**：错误码经 Rust 的 error 链包装后到达前端时通常带外层前缀
 *  （`Error::LlmResponse` 的 Display 是 "llm response: [LLM_EMPTY_EOS] ..."），
 *  锚 `^` 会让本函数对它设计要处理的那批错误恰好全部失效。 */
export function localizeAiError(raw: string): string {
  const m = /\[([A-Z_]+)\]/.exec(raw);
  const key = m ? CODE_KEYS[m[1]] : undefined;
  return key ? i18next.t(key) : raw;
}
