//! 屏幕记忆(L2/L3)命令:回填 + 手动触发消化 + 未入索引统计。
//!
//! 定时触发与调试页观测后续接。

use tauri::State;

use crate::memory::{digest, MemoryDb};
use crate::storage::{DbPool, SqliteResultExt};

/// 记忆库句柄的 managed state。None = 启动时打开失败(帧登记同样停用),
/// 命令层对 None 返回明确错误而不是 panic。
pub struct MemoryState(pub Option<MemoryDb>);

fn require(mem: &MemoryState) -> Result<&MemoryDb, String> {
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
) -> Result<digest::DigestReport, String> {
    let db = require(&mem)?;
    digest::run(db).await.map_err(String::from)
}

/// 请求停止当前正在进行的消化批(banner 的停止按钮)——手动批与常驻批的
/// 当前轮都会停。翻标志即返回,循环帧间感知、最多一帧(~1s)后停;
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

/// 大小写不敏感(仅 ASCII fold,CJK 原样)的子串定位,返回字符索引。
/// O(n·m) 滑窗:text 物化列最多几十 KB、m ≤ 64,毫秒内,不值得上索引结构。
fn find_ci(hay: &[char], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| {
        hay[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

/// 命中上下文:定位首个词,前后各截一段;两端被截断时补省略号。
/// 会话文本是 OCR 行拼接,换行统一成空格便于单行展示。
fn make_snippet(text: &str, word: &str) -> String {
    let flat = text.replace(['\n', '\r'], " ");
    let chars: Vec<char> = flat.chars().collect();
    let hit = find_ci(&chars, word).unwrap_or(0);
    let start = hit.saturating_sub(SNIPPET_BEFORE_CHARS);
    let end = (hit + word.chars().count() + SNIPPET_AFTER_CHARS).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// 全文搜索屏幕记忆(搜索页):按空白拆词、隐式 AND,命中会话按时间倒序分页。
///
/// 两条路径:
/// - 全部词 ≥ 3 字符 → FTS trigram 索引(快路径)
/// - 含短词(中文双字词很常见)→ trigram 对 <3 字符的查询词拿不出任何结果,
///   整个查询退化为物化列 `text` 的 LIKE 全扫——会话行数万级、文本列几十 KB
///   级,毫秒内完成,可接受
///
/// 每条命中附"首个命中词所在行的首现帧"(session_lines 行级留痕),前端用它
/// 定位到具体截图与时刻。
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
    let limit = i64::from(limit.unwrap_or(30).min(200));
    let offset = i64::from(offset.unwrap_or(0));
    // 混合路径:≥3 字符的词走 trigram FTS 缩小集合,短词(<3,trigram 拿不出
    // 结果)只在该集合内 LIKE 过滤。仅当全部是短词时才退化为全表 LIKE 扫——
    // 文本永久保留,全扫成本随库龄线性涨,能靠 FTS 缩集就绝不全扫。
    let long_words: Vec<String> = words
        .iter()
        .filter(|w| w.chars().count() >= 3)
        .cloned()
        .collect();
    let short_words: Vec<String> = words
        .iter()
        .filter(|w| w.chars().count() < 3)
        .cloned()
        .collect();
    let first_word = words[0].clone();
    let first_like = crate::chat::tools::like_pattern(&first_word);

    let (total, hits) =
        db.0.call(move |conn| {
            type Row = (i64, String, String, String, String, String);
            let row_of = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            };
            const SELECT_COLS: &str = "s.id, COALESCE(s.app_id,''), COALESCE(s.title,''), \
                                       s.started_ts, s.ended_ts, s.text";

            let (total, rows): (i64, Vec<Row>) = if !long_words.is_empty() {
                // FTS 缩集(长词) + 集合内 LIKE(短词,可为空)
                let fts = crate::chat::tools::fts_literal(&long_words);
                let short_patterns: Vec<String> = short_words
                    .iter()
                    .map(|w| crate::chat::tools::like_pattern(w))
                    .collect();
                // 参数 ?1 = FTS 词串,短词 LIKE 从 ?2 起编号
                let short_cond = (0..short_patterns.len())
                    .map(|i| format!(" AND s.text LIKE ?{} ESCAPE '\\'", i + 2))
                    .collect::<String>();
                let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&fts];
                for p in &short_patterns {
                    bind.push(p);
                }
                let total = conn
                    .query_row(
                        &format!(
                            "SELECT COUNT(*)
                             FROM text_sessions_fts
                             JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                             WHERE text_sessions_fts MATCH ?1{short_cond}"
                        ),
                        bind.as_slice(),
                        |r| r.get(0),
                    )
                    .db()?;
                let mut stmt = conn
                    .prepare(&format!(
                        "SELECT {SELECT_COLS}
                         FROM text_sessions_fts
                         JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                         WHERE text_sessions_fts MATCH ?1{short_cond}
                         ORDER BY s.started_ts DESC LIMIT {limit} OFFSET {offset}"
                    ))
                    .db()?;
                let rows = stmt
                    .query_map(bind.as_slice(), row_of)
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                (total, rows)
            } else {
                // 全部是短词:别无选择,物化列 LIKE 全扫
                let patterns: Vec<String> = words
                    .iter()
                    .map(|w| crate::chat::tools::like_pattern(w))
                    .collect();
                let cond = (1..=patterns.len())
                    .map(|i| format!("s.text LIKE ?{i} ESCAPE '\\'"))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let total = conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM text_sessions s WHERE {cond}"),
                        rusqlite::params_from_iter(patterns.iter()),
                        |r| r.get(0),
                    )
                    .db()?;
                let mut stmt = conn
                    .prepare(&format!(
                        "SELECT {SELECT_COLS} FROM text_sessions s WHERE {cond}
                         ORDER BY s.started_ts DESC LIMIT {limit} OFFSET {offset}"
                    ))
                    .db()?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(patterns.iter()), row_of)
                    .db()?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .db()?;
                (total, rows)
            };

            // 每条命中补证据帧:该会话里第一条含首词的行的首现帧(同 chat 搜索工具口径)
            let mut hits = Vec::with_capacity(rows.len());
            for (id, app, title, started_ts, ended_ts, text) in rows {
                let frame: Option<(String, String)> = conn
                    .query_row(
                        "SELECT first_path, first_ts FROM session_lines
                         WHERE session_id = ?1 AND text LIKE ?2 ESCAPE '\\' LIMIT 1",
                        rusqlite::params![id, first_like],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                hits.push(MemorySearchHit {
                    session_id: id,
                    app,
                    title,
                    started_ts,
                    ended_ts,
                    snippet: make_snippet(&text, &first_word),
                    frame_path: frame.as_ref().map(|(p, _)| p.clone()),
                    frame_ts: frame.map(|(_, t)| t),
                });
            }
            Ok((total as u64, hits))
        })
        .await
        .map_err(|e| e.to_string())?;

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
    tokio::task::spawn_blocking(move || -> Result<Vec<[f32; 4]>, String> {
        let eng = crate::ai::ocr::OcrEngine::load().map_err(|e| e.to_string())?;
        let lines = eng.recognize_file(&file).map_err(|e| e.to_string())?;
        Ok(lines
            .into_iter()
            .filter(|l| {
                let lt = l.text.to_lowercase();
                words.iter().any(|w| lt.contains(w.as_str()))
            })
            .filter_map(|l| l.box_norm)
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
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
