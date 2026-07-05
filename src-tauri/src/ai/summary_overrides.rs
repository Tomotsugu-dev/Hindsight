//! 调试用：单次 generate 调用对 settings.ai 的局部覆盖（不写 settings 全局）。
//!
//! 任意字段 `None` = 走 settings.ai 的值；`Some(_)` = 本次跑生效，不留痕。
//! 数值字段会经过跟 sanitize 一样的 clamp，保证 override 不会越界。
//!
//! `system_prompt` 是文本覆盖，会写到 ai.prompt_overrides 当前语言对应的字段
//! ——空字符串等价"清覆盖走默认"。
//!
//! 字段与 [`AiConfig`] 1:1 对应：新增 AiConfig 字段时若想支持 override，
//! 也要在本结构体加同名字段并在 [`AiOverrides::with_overrides`] 里加分支。

use serde::Deserialize;

use crate::ai::config::{AiConfig, SUMMARY_CLOUD_SENTINEL};

/// 调试覆盖参数。从 generate_day_summary 等命令的 `overrides` 字段反序列化进来；
/// 详见模块顶部说明。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverrides {
    pub excluded_categories: Option<Vec<String>>,
    /// 段总结的 system prompt 覆盖文本（按当前语言写入 prompt_overrides）
    pub system_prompt: Option<String>,
    /// llama-server `--batch-size` / `--ubatch-size`（取一致值）；调试用，不进 settings。
    /// 旧字段是 fallback——`summary_*` 未传时降级使用。
    pub batch_size: Option<u32>,
    /// llama-server `-np`：并行槽位数。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub parallel_slots: Option<u32>,
    /// **每个 slot** 的 ctx 上限。最终 `--ctx-size = ctx_size × parallel_slots`。
    /// 详见 [`Self::batch_size`] 关于 fallback 语义。
    pub ctx_size: Option<u32>,

    /// 段总结阶段的 batch；`None` = fallback 到 [`Self::batch_size`]。
    pub summary_batch_size: Option<u32>,
    /// 段总结阶段的 `-np`（推荐恒为 1）；`None` = fallback 到 [`Self::parallel_slots`]。
    pub summary_parallel_slots: Option<u32>,
    /// 段总结阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    pub summary_ctx_size: Option<u32>,

    /// 本次跑是否走云端段总结。`Some(true)` = 强制走 ExternalChatClient，
    /// `Some(false)` = 强制本地，`None` = 沿用 settings.ai。
    /// endpoint / model / api_key 永远沿用 settings 全局值——这里只决定路径。
    pub external_enabled: Option<bool>,
}

impl AiOverrides {
    /// 这次调用是否需要重启引擎（任一启动级 override 非 None）。
    /// 仅 debug 路径用——触发跑前 stop+start with overrides，跑后再 stop
    /// 让默认日报回到 settings 默认。
    pub(crate) fn needs_engine_restart(&self) -> bool {
        self.batch_size.is_some()
            || self.parallel_slots.is_some()
            || self.ctx_size.is_some()
            || self.summary_batch_size.is_some()
            || self.summary_parallel_slots.is_some()
            || self.summary_ctx_size.is_some()
    }

    /// 把 override 应用到一份 `AiConfig` 上，返回合并后的新值（不就地改原值）。
    pub fn with_overrides(self, mut ai: AiConfig) -> AiConfig {
        if let Some(v) = self.excluded_categories {
            ai.excluded_categories = v;
        }
        // 文本 prompt 覆盖按当前 prompt_language 写到对应字段；空串等同 "走默认"
        let lang = ai.prompt_language.clone();
        if let Some(v) = self.system_prompt {
            match lang.as_str() {
                "en" => ai.prompt_overrides.system_en = v,
                "ja" => ai.prompt_overrides.system_ja = v,
                "pt" => ai.prompt_overrides.system_pt = v,
                _ => ai.prompt_overrides.system_zh = v,
            }
        }
        // 引擎启动级覆盖：debug 用户在调试 tab 选了值时把它合并进 ai.* 字段；
        // daily 路径不传 AiOverrides 就走 settings.ai 默认值。
        if let Some(v) = self.batch_size {
            ai.batch_size = Some(v);
        }
        if let Some(v) = self.parallel_slots {
            ai.parallel_slots = Some(v);
        }
        if let Some(v) = self.ctx_size {
            ai.ctx_size = Some(v);
        }
        if let Some(v) = self.summary_batch_size {
            ai.summary_batch_size = Some(v);
        }
        if let Some(v) = self.summary_parallel_slots {
            ai.summary_parallel_slots = Some(v);
        }
        if let Some(v) = self.summary_ctx_size {
            ai.summary_ctx_size = Some(v);
        }
        // 云端段总结路径开关：Debug tab 的「云端 API」section toggle 触发。
        // build_step2() 看 ai.summary_use_cloud()——同时需要 external_enabled=true
        // 且 summary_main == SUMMARY_CLOUD_SENTINEL。这里两个一起改，保证 override
        // 语义不被 sentinel 二态门破坏。
        if let Some(v) = self.external_enabled {
            ai.external_enabled = v;
            if v {
                ai.summary_main = SUMMARY_CLOUD_SENTINEL.to_string();
            } else if ai.summary_main.trim() == SUMMARY_CLOUD_SENTINEL {
                // 当前本来选定云端，但 Debug 要本地跑这次 → 临时清成 fallback 到 active_main
                ai.summary_main = String::new();
            }
        }
        ai
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_keeps_original() {
        let base = AiConfig::default();
        let original = base.clone();
        let merged = AiOverrides::default().with_overrides(base);
        assert_eq!(merged.batch_size, original.batch_size);
        assert_eq!(merged.parallel_slots, original.parallel_slots);
        assert_eq!(merged.ctx_size, original.ctx_size);
    }

    #[test]
    fn some_overrides_get_applied() {
        let base = AiConfig::default();
        let merged = AiOverrides {
            batch_size: Some(2048),
            parallel_slots: Some(4),
            ctx_size: Some(16384),
            ..Default::default()
        }
        .with_overrides(base);
        assert_eq!(merged.batch_size, Some(2048));
        assert_eq!(merged.parallel_slots, Some(4));
        assert_eq!(merged.ctx_size, Some(16384));
    }

    #[test]
    fn engine_restart_detection() {
        assert!(!AiOverrides::default().needs_engine_restart());
        assert!(AiOverrides {
            batch_size: Some(512),
            ..Default::default()
        }
        .needs_engine_restart());
        assert!(AiOverrides {
            parallel_slots: Some(2),
            ..Default::default()
        }
        .needs_engine_restart());
        assert!(AiOverrides {
            ctx_size: Some(8192),
            ..Default::default()
        }
        .needs_engine_restart());
    }
}
