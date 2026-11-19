mod kinds;
mod lang;
#[cfg(test)]
mod test_util;
mod text;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

pub use self::kinds::Query;
pub use self::lang::ChatLang;
use crate::ai::model::{ToolCall, ToolSpec};
use crate::ai::policy::AiDataPolicy;
use crate::error::Result;
use crate::storage::Connection;

/// 给模型查库用的工具：给模型看的说明、检查模型发来的调用、执行。
///
/// 一次提问开一个：检查日期要用 `today`，结果和报错的文字按 `lang` 写。
pub struct Tools {
    ctx: ToolCtx,
    today: NaiveDate,
    lang: ChatLang,
}

/// 调用不合法的原因，写给模型看。
#[derive(Debug)]
pub struct ToolError(pub String);

impl Tools {
    /// 打开主库和记忆库的只读连接。
    pub async fn open_readonly(today: NaiveDate, lang: ChatLang) -> Result<Self> {
        Ok(Self {
            ctx: ToolCtx::open_readonly().await?,
            today,
            lang,
        })
    }

    /// 给模型看的工具列表，说明按提示词语言写。
    pub fn specs(&self) -> Vec<ToolSpec> {
        kinds::specs(self.lang)
    }

    /// 检查模型发来的调用：参数读不读得懂，值合不合法。
    pub fn validate(&self, call: &ToolCall) -> std::result::Result<Query, ToolError> {
        kinds::parse(call, self.today, self.lang)
    }

    /// 执行检查过的调用。引用编号从 `next_citation` 开始，文字最多 `max_chars` 字。
    ///
    /// 工具按 `max_chars` 少列几条；这里再截一次，工具算错了也超不过。
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

/// 证据引用——前端渲染证据卡的数据。`index` 是本轮对话内的全局编号,
/// 模型在答案里写 [index] 引用它。Deserialize 供聊天历史从库里回读。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: usize,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    /// 证据帧(命中行首现帧/会话首帧);可能已被 retention 删除,前端兜底
    pub frame_path: Option<String>,
}

/// 工具执行结果:给模型看的紧凑文本 + 给前端的结构化引用。
pub struct ToolOutput {
    pub content: String,
    pub citations: Vec<Citation>,
}

/// 两个库的只读连接。
struct ToolCtx {
    main: Connection,
    mem: Connection,
    /// AI 能看到什么。第一次用时从主库读。
    policy: OnceCell<AiDataPolicy>,
}

impl ToolCtx {
    /// 打开主库与记忆库的只读连接。
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

    /// AI 能看到什么。一个问题里只从主库读一次。
    ///
    /// 读失败时，用到它的工具执行失败，不会给出没筛过的数据。
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

/// FTS MATCH 注入防护:每个词包成带引号的字符串字面量(内部引号剥除),
/// 多词以空格连接 = 隐式 AND。模型/用户输入永远进不了 FTS 语法位。
/// pub(crate):搜索页命令([`crate::commands::screen_memory::memory_search`])同款复用。
pub(crate) fn fts_literal(keywords: &[String]) -> String {
    keywords
        .iter()
        .map(|k| format!("\"{}\"", k.replace('"', " ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// LIKE 通配符转义(ESCAPE '\')。pub(crate) 理由同 [`fts_literal`]。
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
