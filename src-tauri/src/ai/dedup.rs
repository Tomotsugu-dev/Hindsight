//! 余弦相似度阈值去重（Phase 1C）。
//!
//! 算法：贪心单连通去重。按时间顺序遍历，每张图跟"已保留"池倒序比较；
//! 任一池内向量的余弦相似度 ≥ 阈值即视为重复，丢弃。
//!
//! 输入向量必须**事先 L2 归一化**（见 [`crate::ai::embedding`]）——这样余弦
//! 相似度退化成 dot product，省一次 sqrt + 除法。
//!
//! ## 标题守卫
//!
//! MobileNet 是 ImageNet 分类骨干，特征编码的是版面/配色/纹理，**对文字是瞎的**：
//! 同一编辑器开不同文件、同一店铺不同商品页，嵌入几乎相同 → 会被错误合并。
//! `window_title` 是免费的内容信号——两帧标题都非空且不同时**禁止合并**
//! （不同文件名/商品名/网页名 = 内容不同），标题相同或任一侧缺失才回落嵌入判定。
//!
//! ## 合并留痕
//!
//! 被丢的帧记录 `(member_path → representative_path)` 映射，调用方持久化到
//! `screenshot_dedup_map` 表。收益：搜索命中能报"该内容出现于 21:10–21:40"的
//! 时间段而非孤立时刻；将来给被合并帧补 OCR/描述有账可查；也能反向审计去重质量。
//!
//! ## 为什么不带时间窗参数
//!
//! 调用方 [`crate::ai::summary_runner::DaySummaryRunner::run_one_segment`] 在
//! `ai.segments` 切出的每个段内独立调本函数——段间天然隔离，跨段不合并。
//! 这是用户配置 segments 的语义边界，不需要再多一个 `time_window_minutes`
//! 让用户决策。
//!
//! ## 性能
//!
//! 复杂度 O(n²)。段内最多几千张，1000² = 1M 次 dot product × 576 维 = 576M flops，
//! CPU 单线程 < 1 s。不需要 OOM 防护——active 池由段大小自然限制。

use crate::repo::ai_summaries::ScreenshotMeta;

/// 去重结果：保留的帧 + 被合并帧的归属映射。
pub struct DedupOutcome {
    /// 保留的 metas（顺序与输入一致，子集）
    pub kept: Vec<ScreenshotMeta>,
    /// `(member_path, representative_path)`——member 因与 representative 相似被丢
    pub merged: Vec<(String, String)>,
}

/// 标准化窗口标题用于守卫比较：trim + 压缩连续空白。
/// 返回 None = 无标题信号（缺失或空串），守卫对该帧不生效。
fn norm_title(t: &Option<String>) -> Option<String> {
    let raw = t.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// 跑余弦阈值去重（带标题守卫），返回保留帧 + 合并映射。
///
/// `threshold` 取值范围 0..=1（实际有意义区间 0.85..=0.99）：
///   - 0.95：用户实跑数据 ~70% 去重率（POC 验证）
///   - 0.99：极保守，只丢几乎像素级一样的；去重率 ~30%
///   - 0.85：激进，截图轻微滚动也合并；可能误删
///
/// `embeddings` 必须跟 `metas` 一一对齐（同长度，同顺序）。长度不一致返回原 metas
/// 不去重——总比 panic 后整段失败强；调用方 caller 应保证对齐。
pub fn dedup_by_embedding(
    metas: Vec<ScreenshotMeta>,
    embeddings: &[Vec<f32>],
    threshold: f32,
) -> DedupOutcome {
    if metas.len() != embeddings.len() {
        log::warn!(
            "dedup_by_embedding: metas/embeddings 长度不一致（{} vs {}），跳过去重",
            metas.len(),
            embeddings.len()
        );
        return DedupOutcome {
            kept: metas,
            merged: Vec::new(),
        };
    }
    if metas.is_empty() {
        return DedupOutcome {
            kept: metas,
            merged: Vec::new(),
        };
    }

    let titles: Vec<Option<String>> = metas.iter().map(|m| norm_title(&m.window_title)).collect();

    let mut keep_idx: Vec<usize> = Vec::with_capacity(metas.len());
    let mut merged: Vec<(String, String)> = Vec::new();
    keep_idx.push(0); // 第一张永远保留
    'outer: for i in 1..metas.len() {
        // 跟已保留池倒序比——大概率最近的一张就是最像的，命中即跳
        for &j in keep_idx.iter().rev() {
            // 标题守卫：两侧都有标题且不同 → 内容不同，禁止并入 j（继续找池里其它候选）
            if let (Some(ti), Some(tj)) = (&titles[i], &titles[j]) {
                if ti != tj {
                    continue;
                }
            }
            if cosine_normalized(&embeddings[i], &embeddings[j]) >= threshold {
                merged.push((metas[i].path.clone(), metas[j].path.clone()));
                continue 'outer;
            }
        }
        keep_idx.push(i);
    }

    // 按 keep_idx 顺序拼出子集（保留 metas 原顺序）
    let mut keep_iter = keep_idx.into_iter().peekable();
    let mut kept = Vec::new();
    for (idx, meta) in metas.into_iter().enumerate() {
        if keep_iter.peek().copied() == Some(idx) {
            kept.push(meta);
            keep_iter.next();
        }
    }
    DedupOutcome { kept, merged }
}

/// L2 归一化向量的余弦相似度 = dot product。两边必须**事先归一化**。
/// 长度不一致返回 0（视为完全不相似）——不 panic，让调用方继续跑。
fn cosine_normalized(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_meta(path: &str) -> ScreenshotMeta {
        ScreenshotMeta {
            path: path.to_string(),
            app_display: "test".to_string(),
            category_name: None,
            window_title: None,
        }
    }

    fn make_meta_titled(path: &str, title: &str) -> ScreenshotMeta {
        ScreenshotMeta {
            window_title: Some(title.to_string()),
            ..make_meta(path)
        }
    }

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n == 0.0 {
            return v;
        }
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn keeps_first_drops_identical_and_records_merge() {
        // 三张图，第二张跟第一张完全一样，第三张完全垂直
        let metas = vec![make_meta("a"), make_meta("b"), make_meta("c")];
        let embs = vec![
            unit(vec![1.0, 0.0]),
            unit(vec![1.0, 0.0]),
            unit(vec![0.0, 1.0]),
        ];
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 2);
        assert_eq!(out.kept[0].path, "a");
        assert_eq!(out.kept[1].path, "c");
        // 合并留痕：b 被并进 a
        assert_eq!(out.merged, vec![("b".to_string(), "a".to_string())]);
    }

    #[test]
    fn empty_input_passthrough() {
        let out = dedup_by_embedding(Vec::new(), &[], 0.95);
        assert!(out.kept.is_empty());
        assert!(out.merged.is_empty());
    }

    #[test]
    fn length_mismatch_passthrough() {
        let metas = vec![make_meta("a"), make_meta("b")];
        let embs = vec![unit(vec![1.0, 0.0])]; // 只有 1 个
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 2); // 跳过去重，原样返回
        assert!(out.merged.is_empty());
    }

    #[test]
    fn high_threshold_keeps_more() {
        // 两张相似度 0.9 的图：threshold 0.95 时都保留，0.85 时只留第一张
        let metas = vec![make_meta("a"), make_meta("b")];
        let a = unit(vec![1.0, 0.0]);
        let b = unit(vec![0.9, 0.43589]); // dot ≈ 0.9
        let embs = vec![a, b];
        assert_eq!(
            dedup_by_embedding(metas.clone(), &embs, 0.95).kept.len(),
            2,
            "0.95 阈值都保留"
        );
        assert_eq!(
            dedup_by_embedding(metas, &embs, 0.85).kept.len(),
            1,
            "0.85 阈值合并"
        );
    }

    #[test]
    fn title_guard_blocks_merge_of_different_titles() {
        // 嵌入完全相同（同店铺版面），但标题不同 = 不同商品页 → 必须都保留
        let metas = vec![
            make_meta_titled("a", "Keychron K8 - 淘宝网"),
            make_meta_titled("b", "iPhone 15 - 淘宝网"),
        ];
        let embs = vec![unit(vec![1.0, 0.0]), unit(vec![1.0, 0.0])];
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 2, "标题不同禁止合并");
        assert!(out.merged.is_empty());
    }

    #[test]
    fn title_guard_allows_merge_of_same_title() {
        // 标题相同（空白差异被标准化掉）+ 嵌入相同 → 正常合并
        let metas = vec![
            make_meta_titled("a", "main.rs — Hindsight"),
            make_meta_titled("b", "main.rs   —  Hindsight"),
        ];
        let embs = vec![unit(vec![1.0, 0.0]), unit(vec![1.0, 0.0])];
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.merged, vec![("b".to_string(), "a".to_string())]);
    }

    #[test]
    fn title_guard_inactive_when_missing() {
        // 一侧有标题一侧没有 → 守卫不生效，回落嵌入判定（合并）
        let metas = vec![make_meta_titled("a", "某页面"), make_meta("b")];
        let embs = vec![unit(vec![1.0, 0.0]), unit(vec![1.0, 0.0])];
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 1);
    }

    #[test]
    fn different_title_can_still_merge_into_matching_kept_frame() {
        // a(标题X) 与 b(标题Y) 版面相同但标题不同 → b 保留；
        // c(标题Y) 与 b 同标题同版面 → c 并入 b 而不是 a
        let metas = vec![
            make_meta_titled("a", "X"),
            make_meta_titled("b", "Y"),
            make_meta_titled("c", "Y"),
        ];
        let embs = vec![
            unit(vec![1.0, 0.0]),
            unit(vec![1.0, 0.0]),
            unit(vec![1.0, 0.0]),
        ];
        let out = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(out.kept.len(), 2);
        assert_eq!(out.merged, vec![("c".to_string(), "b".to_string())]);
    }
}
