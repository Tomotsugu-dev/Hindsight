mod kinds;
mod lang;
#[cfg(test)]
mod test_util;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

pub use self::kinds::Query;
pub use self::lang::ChatLang;
use crate::ai::model::{ToolCall, ToolSpec};
use crate::ai::policy::AiDataPolicy;
use crate::error::Result;
use crate::storage::Connection;

/// The tools the model uses to query the database: it gives the tool descriptions, checks each
/// call from the model, and runs it.
///
/// Open one per question. The date, the language, and what the AI may see hold only for that
/// question.
pub struct Tools {
    ctx: ToolCtx,
    today: NaiveDate,
    lang: ChatLang,
}

/// Why a tool call was rejected.
#[derive(Debug)]
pub struct ToolError(pub String);

impl Tools {
    /// Opens read-only connections to the main database and the memory database.
    pub async fn open_readonly(today: NaiveDate, lang: ChatLang) -> Result<Self> {
        Ok(Self {
            ctx: ToolCtx::open_readonly().await?,
            today,
            lang,
        })
    }

    /// Lists the tools for the model, with descriptions in the prompt language.
    pub fn specs(&self) -> Vec<ToolSpec> {
        kinds::specs(self.lang)
    }

    /// Checks a call from the model: whether its arguments can be read, and whether their values
    /// are valid.
    pub fn validate(&self, call: &ToolCall) -> std::result::Result<Query, ToolError> {
        kinds::parse(call, self.today, self.lang)
    }

    /// Runs a checked call.
    ///
    /// Citations are numbered from `next_citation`. Text over `max_chars` characters is cut, and
    /// "…" is added at the end. The search tools list fewer rows to stay within `max_chars`. The
    /// timeline and stats tools do not, so only this cut limits them.
    pub async fn execute(
        &self,
        query: &Query,
        next_citation: usize,
        max_chars: usize,
    ) -> Result<ToolOutput> {
        let mut out = query
            .execute(&self.ctx, next_citation, max_chars, self.lang)
            .await?;
        out.content = truncate(&out.content, max_chars);
        Ok(out)
    }
}

/// A piece of evidence. The front end shows it as an evidence card.
///
/// `index` is unique within this round of the chat, and the model cites it as `[index]` in the
/// answer. `Deserialize` is for reading chat history back from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: usize,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    /// The screenshot for this evidence: the first frame of the match or of the session. It may
    /// have been deleted after the retention period, and the front end handles that.
    pub frame_path: Option<String>,
}

/// The result of a tool: short text for the model, and citations for the front end.
pub struct ToolOutput {
    pub content: String,
    pub citations: Vec<Citation>,
}

/// Read-only connections to the two databases.
struct ToolCtx {
    main: Connection,
    mem: Connection,
    /// What the AI may see. It is read from the main database on first use.
    policy: OnceCell<AiDataPolicy>,
}

impl ToolCtx {
    /// Opens read-only connections to the main database and the memory database.
    async fn open_readonly() -> Result<Self> {
        use rusqlite::OpenFlags;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let main = Connection::open_with_flags(crate::storage::db_path()?, flags).await?;
        let mem = Connection::open_with_flags(crate::memory::memory_db_path()?, flags).await?;
        Ok(Self {
            main,
            mem,
            policy: OnceCell::new(),
        })
    }

    /// Returns what the AI may see.
    ///
    /// After the first successful read, it is not read again for this question. If the read
    /// fails, the tool that needs it fails too, and never returns unfiltered data.
    async fn policy(&self) -> Result<&AiDataPolicy> {
        self.policy
            .get_or_try_init(|| AiDataPolicy::load(&self.main))
            .await
    }
}

/// Report-scope WHERE clause for activities (?1 = from, ?2 = to; extra params
/// number from ?3). Pair it with
/// [`FROM_ACTIVITY_GROUP`](crate::repo::sql::FROM_ACTIVITY_GROUP), whose `g`
/// alias it depends on.
///
/// Same shape as repo/reports.rs: the group is the cross-OS source of truth,
/// groups explicitly assigned to `hidden` are dropped, and rows flagged by an
/// ignore rule (`excluded = 1`) are dropped too. Ungrouped activities
/// (`g.category_id` NULL) still pass — `IS NOT` is a NULL-safe comparison, so
/// only an explicit 'hidden' is excluded.
const ACTIVITY_WHERE: &str = "WHERE a.local_date BETWEEN ?1 AND ?2
       AND g.category_id IS NOT 'hidden'
       AND a.excluded = 0";

/// Guards FTS MATCH against injection.
///
/// Each word becomes a quoted string, and any double quote inside it becomes a space. Words are
/// joined by spaces, which FTS reads as AND. So input from the model or the user never reaches
/// the FTS syntax. It is `pub(crate)` because the search page command
/// ([`crate::commands::screen_memory::memory_search`]) uses it too.
pub(crate) fn fts_literal(keywords: &[String]) -> String {
    keywords
        .iter()
        .map(|k| format!("\"{}\"", k.replace('"', " ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escapes the LIKE wildcards, for use with `ESCAPE '\'`.
///
/// It is `pub(crate)` for the same reason as [`fts_literal`].
pub(crate) fn like_pattern(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::kinds::read::timeline::TimelineQuery;
    use super::test_util::{ctx_with, day};
    use super::*;

    /// 用中文执行一次检查过的调用，引用从 1 编号，文字最多 4000 字。
    async fn run(ctx: &ToolCtx, q: Query) -> ToolOutput {
        q.execute(ctx, 1, 4000, ChatLang::ZhHans).await.unwrap()
    }

    #[tokio::test]
    async fn execute_cuts_the_text_at_max_chars() {
        let ctx = ctx_with(
            "",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'A','t');",
        )
        .await;
        let tools = Tools {
            ctx,
            today: day().1,
            lang: ChatLang::ZhHans,
        };
        let q = Query::GetTimeline(TimelineQuery { range: day() });

        let full = tools.execute(&q, 1, 4000).await.unwrap();
        assert!(full.content.chars().count() > 20, "{}", full.content);
        assert!(!full.content.ends_with('…'), "{}", full.content);

        // 超了就截到 20 字，再加一个省略号；证据不截
        let cut = tools.execute(&q, 1, 20).await.unwrap();
        assert_eq!(cut.content.chars().count(), 21, "{}", cut.content);
        assert!(cut.content.ends_with('…'), "{}", cut.content);
        assert_eq!(cut.citations.len(), full.citations.len());
    }

    #[test]
    fn fts_injection_neutralized() {
        // 模型想夹带 FTS 语法/引号 → 全部变成字面量
        let kws = vec![r#"a" OR b NEAR(c)"#.to_string()];
        let lit = fts_literal(&kws);
        assert_eq!(lit, "\"a  OR b NEAR(c)\"");
        // LIKE 通配符转义
        assert_eq!(like_pattern("100%_a"), "%100\\%\\_a%");
    }

    /// 手动验收(不经过 LLM):直连真实库跑 get_timeline、search_titles 和
    /// search_screen_text,打印模型将看到的工具原文和证据帧数。
    /// 只读连接,不写任何数据。跑法(在 src-tauri 目录下):
    ///
    ///   CHAT_DATE=2026-07-19 CHAT_KW=B站 \
    ///     cargo test --lib manual_real_db -- --ignored --nocapture
    ///
    /// 可选:
    /// - CHAT_DATE_TO=2026-07-19 跨日范围(默认单日);
    /// - CHAT_KW 不设则只跑时间线;
    /// - CHAT_MEM_EMPTY=1 把记忆库换成空库(主库仍真实)——模拟
    ///   "从没开过截图/OCR"的用户,看没有屏幕文字索引时的回答。
    #[tokio::test]
    #[ignore]
    async fn manual_real_db() {
        let date = std::env::var("CHAT_DATE").expect("设 CHAT_DATE=YYYY-MM-DD");
        let from = NaiveDate::parse_from_str(&date, "%Y-%m-%d").expect("CHAT_DATE 格式不对");
        let to = std::env::var("CHAT_DATE_TO")
            .ok()
            .map(|t| NaiveDate::parse_from_str(&t, "%Y-%m-%d").expect("CHAT_DATE_TO 格式不对"))
            .unwrap_or(from);
        let ctx = if std::env::var("CHAT_MEM_EMPTY").is_ok() {
            // 真实主库 + 空记忆库(borrow 测试 schema)= 零截图用户视角
            let empty = ctx_with("", "").await;
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
            let main = Connection::open_with_flags(crate::storage::db_path().unwrap(), flags)
                .await
                .unwrap();
            ToolCtx {
                main,
                mem: empty.mem,
                policy: OnceCell::new(),
            }
        } else {
            ToolCtx::open_readonly().await.unwrap()
        };

        let out = run(
            &ctx,
            Query::GetTimeline(TimelineQuery { range: (from, to) }),
        )
        .await;
        let with_frame = out
            .citations
            .iter()
            .filter(|c| c.frame_path.is_some())
            .count();
        println!("\n── get_timeline({from} ~ {to}) ──────────────");
        println!("{}", out.content);
        println!(
            "(引用 {} 条,其中带证据帧 {} 条)",
            out.citations.len(),
            with_frame
        );

        if let Ok(kw) = std::env::var("CHAT_KW") {
            for name in ["search_titles", "search_screen_text"] {
                let call = ToolCall {
                    id: "manual".into(),
                    name: name.into(),
                    arguments: serde_json::json!({
                        "keywords": [kw], "date_from": from.to_string(), "date_to": to.to_string()
                    }),
                };
                let q = kinds::parse(&call, to, ChatLang::ZhHans).unwrap();
                let out = run(&ctx, q).await;
                println!("\n── {name}(\"{kw}\", {from} ~ {to}) ──────");
                println!("{}", out.content);
                println!("(引用 {} 条)", out.citations.len());
            }
        }
    }
}
