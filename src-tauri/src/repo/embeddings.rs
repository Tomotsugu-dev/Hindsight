//! 截图 embedding 缓存（Phase 1C — 相似度去重）。
//!
//! 表结构见 [storage::migrations] 的 v22 (`SCREENSHOT_EMBEDDINGS_SQL`)。
//! 主键 `(screenshot_path, model_id)` —— 同一张截图可同时存多个 backbone 的 embedding，
//! 模型升级换 `model_id` 即可，旧行自然失效不冲突。
//!
//! BLOB 序列化：`f32 × dim` 的 little-endian raw bytes（4 字节/float × 576 = 2304 B/张）。
//! 总开销：1 万张缓存 ~22 MB；不进 sync_outbox（本地产物，跨设备同步无意义）。

use std::collections::HashMap;

use rusqlite::types::ToSql;

use crate::error::Result;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 批量取 embedding：传一组路径 + model_id，返回 path → embedding 的 map。
/// 缺失的 path 不会出现在 map 里——caller 用 `HashMap::contains_key` 判定要不要重算。
pub async fn get_batch(
    pool: &DbPool,
    paths: &[String],
    model_id: &str,
) -> Result<HashMap<String, Vec<f32>>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let paths_owned: Vec<String> = paths.to_vec();
    let model_id = model_id.to_string();
    let map = pool
        .0
        .call(move |conn| {
            // 动态拼 IN 占位：每批 paths 数量不固定
            let placeholders = vec!["?"; paths_owned.len()].join(",");
            let sql = format!(
                "SELECT screenshot_path, embedding
                   FROM screenshot_embeddings
                  WHERE model_id = ?
                    AND screenshot_path IN ({placeholders})"
            );
            let mut params: Vec<&dyn ToSql> = Vec::with_capacity(1 + paths_owned.len());
            params.push(&model_id);
            for p in &paths_owned {
                params.push(p);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    let p: String = r.get(0)?;
                    let blob: Vec<u8> = r.get(1)?;
                    Ok((p, blob))
                })
                .db()?;
            let mut out: HashMap<String, Vec<f32>> = HashMap::new();
            for row in it {
                let (p, blob) = row.db()?;
                if let Some(vec) = bytes_to_f32_vec(&blob) {
                    out.insert(p, vec);
                }
                // blob 长度异常的行直接当 cache miss 跳过——下次重算覆盖
            }
            Ok(out)
        })
        .await?;
    Ok(map)
}

/// 批量写入 / 更新一组 embedding。`dim` 自动取 vector 长度。
/// 同 path + model_id 已存在时覆盖（ON CONFLICT DO UPDATE）。
pub async fn upsert_batch(
    pool: &DbPool,
    rows: Vec<(String, &'static str, Vec<f32>)>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO screenshot_embeddings(screenshot_path, model_id, dim, embedding)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(screenshot_path, model_id) DO UPDATE SET
                             dim        = excluded.dim,
                             embedding  = excluded.embedding,
                             created_at = datetime('now')",
                    )
                    .db()?;
                for (path, model_id, embedding) in &rows {
                    let blob = f32_slice_to_bytes(embedding);
                    stmt.execute(rusqlite::params![
                        path,
                        model_id,
                        embedding.len() as i64,
                        blob,
                    ])
                    .db()?;
                }
            }
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// f32 slice → little-endian byte vec。每个 f32 占 4 字节。
fn f32_slice_to_bytes(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// 反向：little-endian byte slice → Vec<f32>。长度非 4 的倍数返回 None。
fn bytes_to_f32_vec(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().ok()?;
        out.push(f32::from_le_bytes(arr));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_f32_bytes() {
        // 0.5 / 1024 / -2.5 三个普通值 + NaN 走完整 round-trip；0.5/1024 是 f32 精确值
        let v: Vec<f32> = vec![1.0, -2.5, 0.5, 1024.0, f32::NAN];
        let bytes = f32_slice_to_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        let back = bytes_to_f32_vec(&bytes).expect("valid");
        assert_eq!(back.len(), v.len());
        for (a, b) in v.iter().zip(back.iter()) {
            if a.is_nan() {
                assert!(b.is_nan());
            } else {
                assert!((a - b).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn bytes_misaligned_returns_none() {
        let bad = [0u8, 1, 2]; // 3 bytes 不是 4 倍数
        assert!(bytes_to_f32_vec(&bad).is_none());
    }
}
