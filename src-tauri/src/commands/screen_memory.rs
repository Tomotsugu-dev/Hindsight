//! 屏幕记忆(L2/L3)命令:回填 + 手动触发消化 + 未入索引统计。
//!
//! 定时触发与调试页观测后续接。

use rusqlite::params;
use tauri::State;

use crate::memory::search::{find_sessions, first_line_containing, screenshot_of_line};
use crate::memory::snippet::snippet;
use crate::memory::{digest, MemoryDb};
use crate::storage::{DbPool, DbResult, SqliteResultExt};

/// 记忆库句柄的 managed state。None = 启动时打开失败(帧登记同样停用),
/// 命令层对 None 返回明确错误而不是 panic。
pub struct MemoryState(pub Option<MemoryDb>);

pub(crate) fn require(mem: &MemoryState) -> Result<&MemoryDb, String> {
    mem.0
        .as_ref()
        .ok_or_else(|| "屏幕记忆库不可用(启动时打开失败,详见日志)".to_string())
}

/// 历史回填:把主库已有截图的活动行登记为待消化帧。幂等,重复调用无副作用。
/// 返回登记(含已存在跳过)的行数。
#[tauri::command]
pub async fn memory_backfill(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
) -> Result<u64, String> {
    let db = require(&mem)?;
    digest::backfill_from_activities(&pool, db)
        .await
        .map_err(String::from)
}

/// 手动触发一次消化(OCR → 折叠 → FTS)。已在跑时返回错误。
/// 首次调用会自动下载 OCR 模型(约 21MB)。
#[tauri::command]
pub async fn memory_digest_now(
    mem: State<'_, MemoryState>,
    pool: State<'_, DbPool>,
) -> Result<digest::DigestReport, String> {
    let db = require(&mem)?;
    // 用户主动点「立即回填」= 明确要求再试:撤掉熔断冷却(冷却只拦自动入口)
    digest::clear_cooldown();
    let cfg = crate::repo::settings::load(&pool)
        .await
        .map_err(String::from)?;
    digest::run(db, &cfg.download_sources)
        .await
        .map_err(String::from)
}

/// 请求停止当前正在进行的消化批(banner 的停止按钮)——手动批与常驻批的
/// 当前轮都会停。翻标志即返回,循环帧间感知:正常一帧(~1s)后停,
/// 该帧遇引擎挂起时最长等 worker 超时(90s);
/// 手动批的 `memory_digest_now` 随即正常 resolve 已处理部分的账单。
/// 没有批在跑时调用也静默成功(幂等)。
/// 常驻模式下停的只是当前批,下个周期 tick 仍会继续消化;彻底停走
/// 设置 → 常驻 OCR 开关。
#[tauri::command]
pub fn memory_digest_stop() {
    digest::request_stop();
}

/// 未入索引统计:主库截图全集 vs 记忆库登记/完成情况的两库对账。
/// 近似值——文件可能已被保留策略删除(消化时会计入 skipped),可接受。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingStats {
    /// 主库有截图但 frames 未登记(需回填)
    pub unregistered: u64,
    /// frames 已登记但 OCR 未完成(待处理 + 可重试的失败)
    pub pending_ocr: u64,
    /// 两者之和,前端 banner 的 N
    pub total: u64,
    /// 消化(手动/常驻批)是否正在进行——前端据此在挂载时直接进入
    /// "后台索引中"态,而不是显示带按钮的初始态
    pub digest_running: bool,
}

/// 屏幕记忆搜索的一条命中(搜索页一行结果)。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchHit {
    pub session_id: i64,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    /// 首个命中词的上下文窗口(纯文本,无高亮标记——前端按词自行高亮)
    pub snippet: String,
    /// 命中行的首现帧(截图绝对路径);可能已被保留策略清理,前端需兜底
    pub frame_path: Option<String>,
    /// 首现帧拍摄时刻(RFC3339)
    pub frame_ts: Option<String>,
}

/// `memory_search` 的返回:总命中数 + 当前分页窗口。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResp {
    pub total: u64,
    pub hits: Vec<MemorySearchHit>,
}

/// 搜索词数量上限——防一大段粘贴拼出巨型 SQL;超出的词静默丢弃。
const SEARCH_MAX_WORDS: usize = 8;
/// 单词字符数上限,超长截断(trigram 用前缀已足够定位)。
const SEARCH_MAX_WORD_CHARS: usize = 64;
/// snippet 窗口:首个命中词往前 / 往后各截的字符数。
const SNIPPET_BEFORE_CHARS: usize = 20;
const SNIPPET_AFTER_CHARS: usize = 60;

/// 按字符数截断(不切坏 UTF-8 边界)。
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// 全文搜索屏幕记忆(搜索页):按空白拆词、隐式 AND,命中的段按时间倒序分页。
///
/// 每个词出现在段的文字或窗口标题里都算,见 [`find_sessions`]。
#[tauri::command]
pub async fn memory_search(
    mem: State<'_, MemoryState>,
    query: String,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<MemorySearchResp, String> {
    let db = require(&mem)?;
    let words: Vec<String> = query
        .split_whitespace()
        .take(SEARCH_MAX_WORDS)
        .map(|w| truncate_chars(w, SEARCH_MAX_WORD_CHARS))
        .collect();
    if words.is_empty() {
        return Ok(MemorySearchResp {
            total: 0,
            hits: Vec::new(),
        });
    }
    // limit 上限防前端手滑一次拉全库;offset 由"加载更多"翻页
    let limit = limit.unwrap_or(30).min(200) as usize;
    let offset = offset.unwrap_or(0) as usize;
    db.0.call(move |conn| search_page(conn, &words, limit, offset))
        .await
        .map_err(|e| e.to_string())
}

/// Returns one page of search results: the total number of matching sessions and the sessions on
/// this page, sorted by start time, newest first.
///
/// Every word in `words` must appear in the session text or the window title. Each result gets a
/// snippet and a screenshot by these rules:
/// - Find the first search word that appears in the text, and the line it is on. The snippet is
///   cut around that word, and the screenshot is the frame that line comes from.
/// - If only the window title matches (the text has none of the words), the snippet starts at the
///   beginning of the text, and no screenshot is returned.
/// - If the frame record no longer exists, no screenshot is returned either.
fn search_page(
    conn: &rusqlite::Connection,
    words: &[String],
    limit: usize,
    offset: usize,
) -> DbResult<MemorySearchResp> {
    let sessions = find_sessions(conn, words, None, None)?;
    let total = sessions.len() as u64;
    let mut hits = Vec::new();
    for session in sessions.into_iter().skip(offset).take(limit) {
        let text: String = conn
            .query_row(
                "SELECT text FROM text_sessions WHERE id = ?1",
                params![session.id],
                |r| r.get(0),
            )
            .db()?;
        let body_match = words.iter().find_map(|w| {
            first_line_containing(&text, w).map(|(line_no, _)| (w.as_str(), line_no))
        });
        let (anchor_word, frame) = match body_match {
            Some((word, line_no)) => (word, screenshot_of_line(conn, session.id, line_no)?),
            None => ("", None),
        };
        let (frame_path, frame_ts) = frame.unzip();
        hits.push(MemorySearchHit {
            session_id: session.id,
            app: session.app_id,
            title: session.title,
            started_ts: session.started_ts,
            ended_ts: session.ended_ts,
            snippet: snippet(
                &text,
                anchor_word,
                SNIPPET_BEFORE_CHARS,
                SNIPPET_AFTER_CHARS,
            ),
            frame_path,
            frame_ts,
        });
    }
    Ok(MemorySearchResp { total, hits })
}

/// 会话 OCR 全文(行级并集,阅读序)。截图被保留策略清理后,
/// lightbox 降级为文字视图时用——"图没了字还在"的兑现。
#[tauri::command]
pub async fn memory_session_text(
    mem: State<'_, MemoryState>,
    session_id: i64,
) -> Result<String, String> {
    let db = require(&mem)?;
    db.0.call(move |conn| {
        let text: String = conn
            .query_row(
                "SELECT COALESCE(text,'') FROM text_sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |r| r.get(0),
            )
            .db()?;
        Ok(text)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 在单帧截图上定位关键词:现场对这一帧跑 OCR,返回文本含任一关键词的行框
/// (归一化 [x, y, w, h],左上原点)。框不落库——历史帧同样可定位,点开时即时
/// 计算。macOS Vision 引擎零加载成本;Paddle(Windows)首调需加载 ONNX 会话
/// (秒级),lightbox 场景可接受。
///
/// path 必须是 frames 表登记过的帧——拒绝对任意文件跑 OCR。
#[tauri::command]
pub async fn memory_locate(
    mem: State<'_, MemoryState>,
    path: String,
    words: Vec<String>,
) -> Result<Vec<[f32; 4]>, String> {
    let db = require(&mem)?;
    let words: Vec<String> = words
        .iter()
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .take(SEARCH_MAX_WORDS)
        .collect();
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let p = path.clone();
    let registered: bool =
        db.0.call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM frames WHERE path = ?1",
                    rusqlite::params![p],
                    |r| r.get(0),
                )
                .db()?;
            Ok(n > 0)
        })
        .await
        .map_err(|e| e.to_string())?;
    if !registered {
        return Err("帧未登记".to_string());
    }
    let file = std::path::PathBuf::from(&path);
    if !file.is_file() {
        return Ok(Vec::new()); // 截图已被保留策略清理:无框,前端只展示图缺占位
    }
    // 走 worker 子进程:与消化循环共用同一把串行锁——并发调用识别是实测的
    // 挂死放大器,这里以前每次开图还重载一整套引擎(Windows 上秒级),一并治掉。
    // 最坏排在一帧识别之后(~1s),仍远快于旧的按次加载。
    let lines = crate::ai::ocr_supervisor::global()
        .recognize(&file)
        .await
        .map_err(|e| e.to_string())?;
    Ok(lines
        .into_iter()
        .filter(|l| {
            let lt = l.text.to_lowercase();
            words.iter().any(|w| lt.contains(w.as_str()))
        })
        .filter_map(|l| l.box_norm)
        .collect())
}

/// 两库对账走 Rust 侧集合差,不用 ATTACH:主库是应用唯一写连接,
/// ATTACH 状态残留与异常路径的 DETACH 都是额外锁面;路径全集最坏
/// 十万级 ≈ 几 MB、毫秒级,简单无风险。
#[tauri::command]
pub async fn memory_pending_stats(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
) -> Result<PendingStats, String> {
    let db = require(&mem)?;

    // 主库截图全集(与 digest::backfill_from_activities 同一口径)
    let all_paths: Vec<String> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT screenshot_path FROM activities
                     WHERE screenshot_path IS NOT NULL AND screenshot_path != ''",
                )
                .db()?;
            let out = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await
        .map_err(|e| e.to_string())?;

    // 只算滞留超过 10 分钟的待处理帧:常驻模式 60s 一个 tick,队列里永远有
    // 一两张"刚拍完还没轮到"的在途帧,把它们当积压展示是误报。
    // cutoff 与 frames.ts 同为本地时区 RFC3339,字典序可比。
    let cutoff = (chrono::Local::now() - chrono::Duration::minutes(10)).to_rfc3339();
    let (registered, pending_ocr): (std::collections::HashSet<String>, u64) =
        db.0.call(move |conn| {
            let mut stmt = conn.prepare("SELECT path FROM frames").db()?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
                .db()?;
            let pending: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM frames
                     WHERE (ocr_state = 0 OR (ocr_state = 2 AND attempts < 3))
                       AND ts < ?1",
                    rusqlite::params![cutoff],
                    |r| r.get(0),
                )
                .db()?;
            Ok((paths, pending as u64))
        })
        .await
        .map_err(|e| e.to_string())?;

    let unregistered = all_paths
        .iter()
        .filter(|p| !registered.contains(*p))
        .count() as u64;
    Ok(PendingStats {
        unregistered,
        pending_ocr,
        total: unregistered + pending_ocr,
        digest_running: digest::is_running(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 加一段屏幕文字。第 n 行第一次出现在截图 shots/<段>-<n>.jpg 上。
    async fn add(db: &MemoryDb, start: &str, title: &str, lines: &[&str]) {
        let ts = format!("{start}:00+09:00");
        let date = start[..10].to_string();
        let title = title.to_string();
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        db.0.call(move |c| {
            c.execute(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                 VALUES (?1, ?2, ?2, 'app', ?3, ?4)",
                params![date, ts, title, lines.join("\n")],
            )
            .db()?;
            let id = c.last_insert_rowid();
            for n in 0..lines.len() {
                c.execute(
                    "INSERT INTO frames(path, ts) VALUES (?1, ?2)",
                    params![format!("shots/{id}-{n}.jpg"), ts],
                )
                .db()?;
                c.execute(
                    "INSERT INTO session_frames(session_id, first_line_no, frame_id)
                     VALUES (?1, ?2, last_insert_rowid())",
                    params![id, n as i64],
                )
                .db()?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn page(db: &MemoryDb, words: &[&str], limit: usize, offset: usize) -> MemorySearchResp {
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        db.0.call(move |c| search_page(c, &words, limit, offset))
            .await
            .unwrap()
    }

    fn titles(resp: &MemorySearchResp) -> Vec<&str> {
        resp.hits.iter().map(|h| h.title.as_str()).collect()
    }

    #[tokio::test]
    async fn pages_newest_first_and_counts_every_hit() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        add(&db, "2026-07-01T09:00", "一", &["报销"]).await;
        add(&db, "2026-07-02T09:00", "二", &["报销"]).await;
        add(&db, "2026-07-03T09:00", "三", &["报销"]).await;

        let first = page(&db, &["报销"], 2, 0).await;
        assert_eq!(first.total, 3);
        assert_eq!(titles(&first), ["三", "二"]);
        let rest = page(&db, &["报销"], 2, 2).await;
        assert_eq!(rest.total, 3);
        assert_eq!(titles(&rest), ["一"]);
    }

    #[tokio::test]
    async fn a_word_in_the_title_counts_and_the_snippet_follows_the_word_in_the_text() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        let long = format!("{}请提交报销单", "前".repeat(30));
        add(&db, "2026-07-01T09:00", "财务群", &["闲聊", &long]).await;
        add(&db, "2026-07-01T10:00", "家人群", &["报销"]).await;

        // 「财务群」只在标题里，片段和帧跟着「报销」
        let resp = page(&db, &["财务群", "报销"], 30, 0).await;
        assert_eq!(titles(&resp), ["财务群"]);
        let hit = &resp.hits[0];
        // 词前面留 20 个字
        assert_eq!(hit.snippet, format!("…{}请提交报销单", "前".repeat(17)));
        assert_eq!(hit.frame_path.as_deref(), Some("shots/1-1.jpg"));
        assert_eq!(hit.frame_ts.as_deref(), Some("2026-07-01T09:00:00+09:00"));
    }

    #[tokio::test]
    async fn words_only_in_the_title_have_no_frame() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        add(&db, "2026-07-01T09:00", "财务群", &["第一行", "第二行"]).await;

        let resp = page(&db, &["财务群"], 30, 0).await;
        assert_eq!(resp.hits[0].snippet, "第一行 第二行");
        assert_eq!(resp.hits[0].frame_path, None);
        assert_eq!(resp.hits[0].frame_ts, None);
    }
}
