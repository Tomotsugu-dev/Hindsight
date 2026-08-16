//! OpenAI 兼容协议的共用件:输出预算字段名 + 400 请求自愈。
//!
//! 聊天(`chat::llm`)与日报/周报(`ai::llm`)是两套独立的传输层,读同一组
//! endpoint/model/api_key。它们曾各自演化:聊天发 `max_completion_tokens`
//! 并带自愈层,摘要硬编码 `max_tokens` 且非 2xx 直接判失败——于是同一个
//! OpenAI 端点下"聊天好用、日报一个字也生不出来"。规则放这里共用,
//! 新增规则一次对两边生效。

use serde_json::{json, Value};

/// 输出预算的字段名。云端用 OpenAI 现行的 `max_completion_tokens`:实测
/// (2026-08)gpt-5.6 起对 `max_tokens` 直接 400「not supported with this
/// model」,而 DeepSeek/Moonshot 两个名字都认——统一发新名字是唯一通吃解。
/// 本地 llama-server 沿用 `max_tokens`。
pub(crate) fn budget_key(is_cloud: bool) -> &'static str {
    if is_cloud {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

// ── 请求自愈 ────────────────────────────────────────────────────
//
// 云端 400 绝大多数是"参数不合这家/这个模型的口味",而**错误信息本身就写明
// 了该怎么改**。与其为每个 provider 硬编码知识(追不上 API 演进,更盖不住
// custom 端点的无穷组合),不如按对方的说法改一改重发。OpenAI 兼容生态的
// 错误措辞高度同构,同一套规则对没见过的端点一样有效。
//
// 规则与安全边界经 `scripts/llm/selfheal_probe.py` 验证:mock 侧复现真机
// 抓到的 400 原文(8 场景全自愈)、边界侧 4 场景全部按预期干净放弃、
// 真端点 DeepSeek×2 / OpenAI / Moonshot 端到端通过。

/// 自愈重试上限。够用:实测最长的链是"改名 → 补 none"两轮。
pub(crate) const MAX_HEAL_ROUNDS: u32 = 3;

/// 绝不可为了让请求通过而删掉的字段。删了确实能 200,但功能已经废了——
/// 例如删 `tools` 后 Chat 失去查库能力却装作一切正常,静默失效比报错更坏。
const PROTECTED_FIELDS: [&str; 4] = ["model", "messages", "tools", "tool_choice"];

/// 思考控制字段全集(各家口径不同,撤销时一并清掉)。
const THINKING_FIELDS: [&str; 4] = [
    "thinking",
    "reasoning",
    "reasoning_effort",
    "chat_template_kwargs",
];

/// 取错误信息里第 `n` 个单引号包裹的片段(0 起)。
fn quoted(msg: &str, n: usize) -> Option<&str> {
    msg.split('\'').nth(n * 2 + 1)
}

/// 取 `start` 与 `end` 之间的片段。
fn between<'a>(msg: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &msg[msg.find(start)? + start.len()..];
    Some(rest[..rest.find(end)?].trim())
}

fn remove_field(body: &mut Value, key: &str) -> bool {
    if PROTECTED_FIELDS.contains(&key) {
        return false;
    }
    body.as_object_mut().and_then(|o| o.remove(key)).is_some()
}

/// 按 400 的错误信息就地修正请求体。返回 false = 没有规则能修,该放弃。
pub(crate) fn heal_request(body: &mut Value, err_msg: &str) -> bool {
    let low = err_msg.to_ascii_lowercase();

    // R2 参数改名:对方直接给了新名字(`'max_tokens' ... use 'max_completion_tokens' instead`)
    if low.contains("is not supported") && low.contains("instead") {
        if let (Some(old), Some(new)) = (quoted(err_msg, 0), quoted(err_msg, 1)) {
            if let Some(o) = body.as_object_mut() {
                if let Some(v) = o.remove(old) {
                    o.insert(new.to_string(), v);
                    return true;
                }
            }
        }
    }

    // R3 值不被支持:降到错误信息里列出的第一个合法值,拿不到就删字段
    if low.contains("does not support") {
        if let Some(param) = quoted(err_msg, 0) {
            if body.get(param).is_some() {
                // `Supported values are: 'none', 'low', ...` —— 取第一个
                let fallback = low
                    .find("supported values")
                    .and_then(|i| quoted(&err_msg[i..], 0).map(str::to_string));
                return match fallback {
                    Some(v) => {
                        body[param] = json!(v);
                        true
                    }
                    None => remove_field(body, param),
                };
            }
        }
    }

    // R4 组合不被支持(`Function tools with reasoning_effort are not supported`):
    // 把被点名的参数压到安全值。注意"根本没发这个参数"也会被拒——OpenAI 带
    // tools 时默认档就非 none,所以要**补**上而不只是改。
    if let Some(param) = between(&low, "with ", " are not supported") {
        if param == "reasoning_effort" {
            if body.get(param).and_then(Value::as_str) != Some("none") {
                body["reasoning_effort"] = json!("none");
                return true;
            }
        } else if remove_field(body, param) {
            return true;
        }
    }

    // R5 模型强制思考(Moonshot `only type=enabled is allowed for this model`,
    // OpenRouter `Reasoning is mandatory ...`):撤掉全部思考控制,按默认行为跑
    if low.contains("only type=enabled")
        || (low.contains("reasoning") && low.contains("mandatory"))
        || low.contains("cannot be disabled")
    {
        let mut hit = false;
        for f in THINKING_FIELDS {
            hit |= remove_field(body, f);
        }
        if hit {
            return true;
        }
    }

    // R1 未知参数:最通用的一条——对方不认识的字段直接删。受保护字段除外
    // (端点若连 tools 都不认,应当干净失败,让用户知道它不能用于对话)
    if low.contains("unknown parameter") {
        if let Some(p) = quoted(err_msg, 0) {
            return remove_field(body, p);
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自愈规则表。用例的错误文案全部取自真机实测原文
    /// (scripts/llm/thinking_probe.py + selfheal 原型),不是编的。
    #[test]
    fn heal_request_rules_cover_real_world_400s() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");

        // R2 参数改名:OpenAI gpt-5.6 起拒收 max_tokens
        let mut b = json!({"model": "m", "max_tokens": 1024});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.")
        ));
        assert_eq!(b["max_completion_tokens"], 1024);
        assert!(b.get("max_tokens").is_none());

        // R1 未知参数:切服务商后残留的别家字段
        let mut b = json!({"model": "m", "thinking": {"type": "disabled"}});
        assert!(heal_request(
            &mut b,
            &e400("Unknown parameter: 'thinking'.")
        ));
        assert!(b.get("thinking").is_none());

        // R3 值不被支持:降到错误信息里列出的第一个合法值
        let mut b = json!({"model": "m", "reasoning_effort": "minimal"});
        assert!(heal_request(
            &mut b,
            &e400("Unsupported value: 'reasoning_effort' does not support 'minimal' with this model. Supported values are: 'none', 'low', 'medium', 'high'.")
        ));
        assert_eq!(b["reasoning_effort"], "none");

        // R4 组合不被支持:参数**不在报文里**也要能补上(OpenAI 默认档非 none)
        let mut b = json!({"model": "m", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna in /v1/chat/completions.")
        ));
        assert_eq!(b["reasoning_effort"], "none");
        // 已经是 none 还报同样的错 → 无计可施,不能空转
        assert!(!heal_request(
            &mut b,
            &e400("Function tools with reasoning_effort are not supported for gpt-5.6-luna.")
        ));

        // R5 模型强制思考(Moonshot kimi-k2.7-code):撤掉全部思考控制
        let mut b = json!({"model": "m", "reasoning_effort": "none", "tools": []});
        assert!(heal_request(
            &mut b,
            &e400("invalid thinking: only type=enabled is allowed for this model")
        ));
        assert!(b.get("reasoning_effort").is_none());
        assert!(b.get("tools").is_some(), "撤思考不得波及 tools");

        // 无法识别的错误:干净放弃,不瞎改
        let mut b = json!({"model": "m", "max_tokens": 1});
        assert!(!heal_request(&mut b, &e400("内部错误 E5012")));
        assert_eq!(b["max_tokens"], 1);
    }

    /// 安全边界:能"修好"反而是事故的场景。
    /// 删掉 tools 请求确实会 200,但 Chat 从此查不了库还装作正常——
    /// 静默失效比报错更坏,这里必须拒绝修复。
    #[test]
    fn heal_request_never_sacrifices_core_fields() {
        let e400 = |s: &str| format!("HTTP 400 Bad Request: {{\"error\":{{\"message\":\"{s}\"}}}}");
        for f in PROTECTED_FIELDS {
            let mut b = json!({"model": "m", "messages": [], "tools": [], "tool_choice": "auto"});
            assert!(
                !heal_request(&mut b, &e400(&format!("Unknown parameter: '{f}'."))),
                "{f} 不该为了让请求通过被删"
            );
            assert!(b.get(f).is_some(), "{f} 必须原样保留");
        }
    }
}
