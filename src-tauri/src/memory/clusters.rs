//! L4 视觉簇:新颖度判定 + 归属留痕(screen-memory.md §3 L4)。
//!
//! 只处理"文本解释不了的帧"(OCR 字符量低于阈值的视觉主导帧)。
//! 每帧嵌入与当日已有簇的代表向量比余弦:像 → 附着(记归属,不进 L5);
//! 都不像 → 新簇(代表帧 = 该帧),`description` 留空即 L5 的待办位。
//!
//! 标题守卫:两帧标题都非空且不同 → 禁止附着该簇——同版面不同内容
//! (不同视频/不同图集)嵌入可能很像,标题是最便宜的内容级保险。

use rusqlite::params;

use super::MemoryDb;
use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 簇半径(余弦相似度下限)。0.90 为设计建议值,**待真实数据标定**——
/// 与 L1 的 f 同样方法:跑真实视觉日 + 画廊人工核对后再定案。
const CLUSTER_RADIUS: f32 = 0.90;

/// 当日簇的内存缓存——消化 run 内存活,跨 run 从库重建。
/// 全天几十簇 × 512 维,暴力扫描微秒级,不需要向量索引。
pub struct ClusterBook {
    date: String,
    items: Vec<ClusterItem>,
}

struct ClusterItem {
    id: i64,
    title: String,
    embedding: Vec<f32>,
}

impl ClusterBook {
    /// 加载某日已有簇(跨 run/常驻 tick 连续的关键)。
    pub async fn load(db: &MemoryDb, date: &str) -> Result<Self> {
        let d = date.to_string();
        let items =
            db.0.call(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id, title, embedding FROM clusters WHERE local_date = ?1")
                    .db()?;
                let out = stmt
                    .query_map([d], |r| {
                        Ok(ClusterItem {
                            id: r.get(0)?,
                            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            embedding: blob_to_f32(&r.get::<_, Vec<u8>>(2)?),
                        })
                    })
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                Ok(out)
            })
            .await?;
        Ok(Self {
            date: date.to_string(),
            items,
        })
    }

    pub fn date(&self) -> &str {
        &self.date
    }

    /// 归属判定:附着最相似的达标簇,否则建新簇。返回簇 id。
    /// `title` 为帧的标准化标题(守卫比较用)。
    pub async fn assign(
        &mut self,
        db: &MemoryDb,
        frame_path: &str,
        title: &str,
        embedding: Vec<f32>,
    ) -> Result<i64> {
        // 嵌入已 L2 归一化,余弦 = 点积
        let mut best: Option<(i64, f32)> = None;
        for item in &self.items {
            // 标题守卫:两者都非空且不同 → 此簇不可附着
            if !title.is_empty() && !item.title.is_empty() && item.title != title {
                continue;
            }
            let sim = dot(&item.embedding, &embedding);
            if best.is_none_or(|(_, b)| sim > b) {
                best = Some((item.id, sim));
            }
        }
        if let Some((id, sim)) = best {
            if sim >= CLUSTER_RADIUS {
                return Ok(id);
            }
        }

        // 新视觉场景 → 建簇,该帧即代表
        let (date, path, t, blob) = (
            self.date.clone(),
            frame_path.to_string(),
            title.to_string(),
            f32_to_blob(&embedding),
        );
        let id =
            db.0.call(move |conn| {
                conn.execute(
                    "INSERT INTO clusters(local_date, rep_path, title, embedding)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![date, path, t, blob],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await?;
        self.items.push(ClusterItem {
            id,
            title: title.to_string(),
            embedding,
        });
        Ok(id)
    }
}

/// 记录帧的视觉归属 + 嵌入(供 P2 文本→图检索)。
pub async fn record_frame(
    db: &MemoryDb,
    frame_path: &str,
    cluster_id: i64,
    embedding: &[f32],
) -> Result<()> {
    let (path, blob) = (frame_path.to_string(), f32_to_blob(embedding));
    db.0.call(move |conn| {
        conn.execute(
            "UPDATE frames SET cluster_id = ?2 WHERE path = ?1",
            params![path, cluster_id],
        )
        .db()?;
        conn.execute(
            "INSERT OR REPLACE INTO frame_embeddings(path, embedding) VALUES (?1, ?2)",
            params![path, blob],
        )
        .db()?;
        Ok(())
    })
    .await?;
    Ok(())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn f32_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(dir: usize) -> Vec<f32> {
        // 构造互相正交的单位向量(余弦 0),以及同向向量(余弦 1)
        let mut v = vec![0f32; 8];
        v[dir] = 1.0;
        v
    }

    #[tokio::test]
    async fn attach_similar_and_split_novel() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut book = ClusterBook::load(&db, "2026-07-05").await.unwrap();

        // 第一帧 → 新簇
        let a = book.assign(&db, "a.jpg", "视频甲", unit(0)).await.unwrap();
        // 同向向量(余弦 1.0 ≥ 半径) → 附着同簇
        let b = book.assign(&db, "b.jpg", "视频甲", unit(0)).await.unwrap();
        assert_eq!(a, b);
        // 正交向量(余弦 0 < 半径) → 新簇
        let c = book.assign(&db, "c.jpg", "视频甲", unit(1)).await.unwrap();
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn title_guard_blocks_attach() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut book = ClusterBook::load(&db, "2026-07-05").await.unwrap();

        let a = book.assign(&db, "a.jpg", "视频甲", unit(0)).await.unwrap();
        // 嵌入完全相同但标题不同 → 守卫禁附,强制新簇
        let b = book.assign(&db, "b.jpg", "视频乙", unit(0)).await.unwrap();
        assert_ne!(a, b);
        // 空标题不触发守卫 → 附着最相似簇
        let c = book.assign(&db, "c.jpg", "", unit(0)).await.unwrap();
        assert!(c == a || c == b);
    }

    #[tokio::test]
    async fn book_reloads_across_runs() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let mut book = ClusterBook::load(&db, "2026-07-05").await.unwrap();
        let a = book.assign(&db, "a.jpg", "视频甲", unit(0)).await.unwrap();
        record_frame(&db, "a.jpg", a, &unit(0)).await.unwrap();

        // 新 run 重建簇册 → 同场景仍附着旧簇
        let mut book2 = ClusterBook::load(&db, "2026-07-05").await.unwrap();
        let b = book2.assign(&db, "b.jpg", "视频甲", unit(0)).await.unwrap();
        assert_eq!(a, b);
    }
}
