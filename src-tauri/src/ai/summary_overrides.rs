//! 调试用：单次 generate 调用对 settings.ai 的局部覆盖（不写 settings 全局）。
//!
//! 任意字段 `None` = 走 settings.ai 的值；`Some(_)` = 本次跑生效，不留痕。
//! 数值字段会经过跟 sanitize 一样的 clamp（max_images 1..=100_000、hash_threshold ≤ 32、
//! hash_window_minutes ≤ 60），保证 override 不会越界。
//!
//! `system_prompt` / `image_describe_prompt` 是文本覆盖，会写到 ai.prompt_overrides /
//! ai.image_describe_overrides 当前语言对应的字段——空字符串等价"清覆盖走默认"。
//!
//! 字段与 [`AiConfig`] 1:1 对应：新增 AiConfig 字段时若想支持 override，
//! 也要在本结构体加同名字段并在 [`AiOverrides::with_overrides`] 里加分支。

use serde::Deserialize;

use crate::ai::config::AiConfig;

/// 调试覆盖参数。从 generate_day_summary / retry_one_image_description 等命令的
/// `overrides` 字段反序列化进来；详见模块顶部说明。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverrides {
    pub excluded_categories: Option<Vec<String>>,
    pub max_images_per_segment: Option<u32>,
    pub hash_threshold: Option<u32>,
    pub hash_window_minutes: Option<u32>,
    /// step 2 段总结的 system prompt 覆盖文本（按当前语言写入 prompt_overrides）
    pub system_prompt: Option<String>,
    /// step 1 单图描述的 system prompt 覆盖文本（按当前语言写入 image_describe_overrides）
    pub image_describe_prompt: Option<String>,
    /// llama-server `--batch-size` / `--ubatch-size`（取一致值）；调试用，不进 settings。
    /// 双套参数语义：旧字段是 fallback——`describe_*` / `summary_*` 未传时降级使用。
    pub batch_size: Option<u32>,
    /// llama-server `-np`：并行槽位数。详见 [`Self::batch_size`] 关于 fallback 语义。
    pub parallel_slots: Option<u32>,
    /// **每个 slot** 的 ctx 上限。最终 `--ctx-size = ctx_size × parallel_slots`。
    /// 详见 [`Self::batch_size`] 关于 fallback 语义。
    pub ctx_size: Option<u32>,

    /// 图描述阶段的 batch；`None` = fallback 到 [`Self::batch_size`]。
    pub describe_batch_size: Option<u32>,
    /// 图描述阶段的 `-np`；`None` = fallback 到 [`Self::parallel_slots`]。
    pub describe_parallel_slots: Option<u32>,
    /// 图描述阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    pub describe_ctx_size: Option<u32>,

    /// 段总结阶段的 batch；`None` = fallback 到 [`Self::batch_size`]。
    pub summary_batch_size: Option<u32>,
    /// 段总结阶段的 `-np`（推荐恒为 1）；`None` = fallback 到 [`Self::parallel_slots`]。
    pub summary_parallel_slots: Option<u32>,
    /// 段总结阶段的每槽 ctx；`None` = fallback 到 [`Self::ctx_size`]。
    pub summary_ctx_size: Option<u32>,

    /// 本次跑是否走云端段总结（step 2）。`Some(true)` = 强制走 ExternalChatClient，
    /// `Some(false)` = 强制本地，`None` = 沿用 settings.ai.external_enabled。
    /// endpoint / model / api_key 永远沿用 settings 全局值——这里只决定路径。
    pub external_enabled: Option<bool>,
}

impl AiOverrides {
    /// 这次调用是否需要重启引擎（任一启动级 override 非 None）。
    /// 仅 debug 路径用——AiOverrides 显式带了 engine 字段时才是「调试覆盖」语义，
    /// 触发跑前 stop+start with overrides，跑后再 stop 让默认日报回到 settings 默认。
    /// daily 路径靠 settings.ai 直接做 ai.* 字段，没这层判断。
    pub(crate) fn needs_engine_restart(&self) -> bool {
        self.batch_size.is_some()
            || self.parallel_slots.is_some()
            || self.ctx_size.is_some()
            || self.describe_batch_size.is_some()
            || self.describe_parallel_slots.is_some()
            || self.describe_ctx_size.is_some()
            || self.summary_batch_size.is_some()
            || self.summary_parallel_slots.is_some()
            || self.summary_ctx_size.is_some()
    }

    /// 把 override 应用到一份 `AiConfig` 上，返回合并后的新值（不就地改原值）。
    ///
    /// clamp 到合法区间后返回；调用方可放心 `Arc<AiConfig>` 等共享原值不被污染。
    pub fn with_overrides(self, mut ai: AiConfig) -> AiConfig {
        if let Some(v) = self.excluded_categories {
            ai.excluded_categories = v;
        }
        if let Some(v) = self.max_images_per_segment {
            // 跟 config.rs sanitize 上限保持一致：10w，让「无限制」档真正不截断
            ai.max_images_per_segment = v.clamp(1, 100_000);
        }
        if let Some(v) = self.hash_threshold {
            ai.hash_threshold = v.min(32);
        }
        if let Some(v) = self.hash_window_minutes {
            ai.hash_window_minutes = v.min(60);
        }
        // 文本 prompt 覆盖按当前 prompt_language 写到对应字段；空串等同 "走默认"
        let lang = ai.prompt_language.clone();
        if let Some(v) = self.system_prompt {
            match lang.as_str() {
                "en" => ai.prompt_overrides.system_en = v,
                "ja" => ai.prompt_overrides.system_ja = v,
                _ => ai.prompt_overrides.system_zh = v,
            }
        }
        if let Some(v) = self.image_describe_prompt {
            match lang.as_str() {
                "en" => ai.image_describe_overrides.system_en = v,
                "ja" => ai.image_describe_overrides.system_ja = v,
                _ => ai.image_describe_overrides.system_zh = v,
            }
        }
        // 引擎启动级覆盖：debug 用户在调试 tab 选了值时把它合并进 ai.batch_size
        // 等字段；daily 路径不传 AiOverrides 就走 settings.ai 默认值。
        // 这样下游统一从 ai.* 取，不用区分两条路。
        if let Some(v) = self.batch_size {
            ai.batch_size = Some(v);
        }
        if let Some(v) = self.parallel_slots {
            ai.parallel_slots = Some(v);
        }
        if let Some(v) = self.ctx_size {
            ai.ctx_size = Some(v);
        }
        if let Some(v) = self.describe_batch_size {
            ai.describe_batch_size = Some(v);
        }
        if let Some(v) = self.describe_parallel_slots {
            ai.describe_parallel_slots = Some(v);
        }
        if let Some(v) = self.describe_ctx_size {
            ai.describe_ctx_size = Some(v);
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
        // 云端段总结路径开关：Debug tab 的「云端 API」section toggle 触发；
        // build_step2() 看 ai.external_enabled 决定 Local vs External。
        if let Some(v) = self.external_enabled {
            ai.external_enabled = v;
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
        assert_eq!(
            merged.max_images_per_segment,
            original.max_images_per_segment
        );
        assert_eq!(merged.hash_threshold, original.hash_threshold);
        assert_eq!(merged.hash_window_minutes, original.hash_window_minutes);
        assert_eq!(merged.batch_size, original.batch_size);
        assert_eq!(merged.parallel_slots, original.parallel_slots);
        assert_eq!(merged.ctx_size, original.ctx_size);
    }

    #[test]
    fn some_overrides_get_applied() {
        let base = AiConfig::default();
        let merged = AiOverrides {
            max_images_per_segment: Some(50),
            hash_threshold: Some(8),
            batch_size: Some(2048),
            parallel_slots: Some(4),
            ctx_size: Some(16384),
            ..Default::default()
        }
        .with_overrides(base);
        assert_eq!(merged.max_images_per_segment, 50);
        assert_eq!(merged.hash_threshold, 8);
        assert_eq!(merged.batch_size, Some(2048));
        assert_eq!(merged.parallel_slots, Some(4));
        assert_eq!(merged.ctx_size, Some(16384));
    }

    #[test]
    fn clamp_max_images() {
        let base = AiConfig::default();
        let merged = AiOverrides {
            max_images_per_segment: Some(0),
            ..Default::default()
        }
        .with_overrides(base);
        assert_eq!(merged.max_images_per_segment, 1, "0 应被钳到下界 1");

        let base = AiConfig::default();
        let merged = AiOverrides {
            max_images_per_segment: Some(999_999),
            ..Default::default()
        }
        .with_overrides(base);
        assert_eq!(merged.max_images_per_segment, 100_000, "上界 10w");
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
