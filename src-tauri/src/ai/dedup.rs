//! 余弦相似度阈值去重（Phase 1C）。
//!
//! 算法：贪心单连通去重。按时间顺序遍历，每张图跟"已保留"池倒序比较；
//! 任一池内向量的余弦相似度 ≥ 阈值即视为重复，丢弃。
//!
//! 输入向量必须**事先 L2 归一化**（见 [`crate::ai::embedding`]）——这样余弦
//! 相似度退化成 dot product，省一次 sqrt + 除法。
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

/// 跑余弦阈值去重。返回保留的 metas（顺序与输入一致，子集）。
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
) -> Vec<ScreenshotMeta> {
    if metas.len() != embeddings.len() {
        log::warn!(
            "dedup_by_embedding: metas/embeddings 长度不一致（{} vs {}），跳过去重",
            metas.len(),
            embeddings.len()
        );
        return metas;
    }
    if metas.is_empty() {
        return metas;
    }

    let mut keep_idx: Vec<usize> = Vec::with_capacity(metas.len());
    keep_idx.push(0); // 第一张永远保留
    'outer: for i in 1..metas.len() {
        // 跟已保留池倒序比——大概率最近的一张就是最像的，命中即跳
        for &j in keep_idx.iter().rev() {
            if cosine_normalized(&embeddings[i], &embeddings[j]) >= threshold {
                continue 'outer;
            }
        }
        keep_idx.push(i);
    }

    // 按 keep_idx 顺序拼出子集（保留 metas 原顺序）
    let mut keep_iter = keep_idx.into_iter().peekable();
    let mut out = Vec::new();
    for (idx, meta) in metas.into_iter().enumerate() {
        if keep_iter.peek().copied() == Some(idx) {
            out.push(meta);
            keep_iter.next();
        }
    }
    out
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
    fn keeps_first_drops_identical() {
        // 三张图，第二张跟第一张完全一样，第三张完全垂直
        let metas = vec![make_meta("a"), make_meta("b"), make_meta("c")];
        let embs = vec![
            unit(vec![1.0, 0.0]),
            unit(vec![1.0, 0.0]),
            unit(vec![0.0, 1.0]),
        ];
        let kept = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].path, "a");
        assert_eq!(kept[1].path, "c");
    }

    #[test]
    fn empty_input_passthrough() {
        let kept = dedup_by_embedding(Vec::new(), &[], 0.95);
        assert!(kept.is_empty());
    }

    #[test]
    fn length_mismatch_passthrough() {
        let metas = vec![make_meta("a"), make_meta("b")];
        let embs = vec![unit(vec![1.0, 0.0])]; // 只有 1 个
        let kept = dedup_by_embedding(metas, &embs, 0.95);
        assert_eq!(kept.len(), 2); // 跳过去重，原样返回
    }

    #[test]
    fn high_threshold_keeps_more() {
        // 两张相似度 0.9 的图：threshold 0.95 时都保留，0.85 时只留第一张
        let metas = vec![make_meta("a"), make_meta("b")];
        let a = unit(vec![1.0, 0.0]);
        let b = unit(vec![0.9, 0.43589]); // dot ≈ 0.9
        let embs = vec![a, b];
        assert_eq!(
            dedup_by_embedding(metas.clone(), &embs, 0.95).len(),
            2,
            "0.95 阈值都保留"
        );
        assert_eq!(
            dedup_by_embedding(metas, &embs, 0.85).len(),
            1,
            "0.85 阈值合并"
        );
    }
}
