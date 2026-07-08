//! Chat:自然语言查询屏幕记忆的 agent 系统。
//!
//! 分层(设计见 docs/design/screen-memory.md §7):
//! - [`tools`]  工具层 + 四道墙的②③④(语义校验/固定参数化查询/只读连接);
//! - [`llm`]    双适配器:云端原生 tools 协议 / 本地 grammar 约束解码(第①道墙);
//! - [`engine`] agent 循环器:步数上限、重复去重、降级阶梯、引用绑定。

pub mod engine;
pub mod lang;
pub mod llm;
pub mod store;
pub mod tools;
