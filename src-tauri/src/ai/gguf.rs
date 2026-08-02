//! GGUF 元数据轻量读取:只回答一个问题——这个本地模型被训练过工具调用吗?
//!
//! 界定标准是**模型作者的声明**而非文件名猜测:GGUF 头里的
//! `tokenizer.chat_template`(随模型发布的官方对话模板)含 `tools` 分支,
//! 或存在 `tokenizer.chat_template.tool_use` 变体键,即认为支持。
//! Gemma 系的模板没有工具分支——正是 Chat 页"很小的本地模型可能不可靠"
//! 的主要来源;Qwen / Llama 3.1+ / Mistral 的模板都有。
//!
//! 解析只顺序遍历 metadata KV(词表数组逐项跳过,BufReader 下毫秒级),
//! 不读任何 tensor 数据。

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// 该 GGUF 的对话模板是否声明了工具调用。
/// `None` = 文件读不出/格式异常——调用方应 fail-open(显示但不标注),
/// 解析失败不能把用户的模型从列表里变没。
pub fn chat_template_supports_tools(path: &Path) -> Option<bool> {
    let file = File::open(path).ok()?;
    // 词表(几万条字符串)也在 metadata 区,顺序小读靠大缓冲吃下
    let mut r = BufReader::with_capacity(1 << 20, file);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let version = read_u32(&mut r)?;
    if !(2..=3).contains(&version) {
        return None;
    }
    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;

    let mut template: Option<String> = None;
    for _ in 0..kv_count {
        let key = read_string(&mut r)?;
        let ty = read_u32(&mut r)?;
        if key == "tokenizer.chat_template.tool_use" {
            // 工具模板单列一键(Command-R / 旧 Qwen 风格)= 明确支持
            return Some(true);
        }
        if key == "tokenizer.chat_template" && ty == 8 {
            template = Some(read_string(&mut r)?);
            // 不能提前返回:tool_use 变体键可能排在后面
            continue;
        }
        skip_value(&mut r, ty)?;
    }
    let t = template?;
    Some(t.contains("tools") || t.contains("tool_call"))
}

fn read_u32(r: &mut impl Read) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

fn read_string(r: &mut impl Read) -> Option<String> {
    let len = read_u64(r)?;
    // 防伪造头拉爆内存:GGUF 元数据字符串不会超过这个量级(模板 ~几十 KB)
    if len > 32 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn skip_bytes(r: &mut impl Read, n: u64) -> Option<()> {
    let copied = std::io::copy(&mut r.take(n), &mut std::io::sink()).ok()?;
    (copied == n).then_some(())
}

/// 跳过一个 metadata 值。类型编号见 GGUF 规范(v2/v3 一致)。
fn skip_value(r: &mut impl Read, ty: u32) -> Option<()> {
    match ty {
        0 | 1 | 7 => skip_bytes(r, 1), // u8 / i8 / bool
        2..=3 => skip_bytes(r, 2),     // u16 / i16
        4..=6 => skip_bytes(r, 4),     // u32 / i32 / f32
        10..=12 => skip_bytes(r, 8),   // u64 / i64 / f64
        8 => {
            let len = read_u64(r)?;
            skip_bytes(r, len)
        }
        9 => {
            let elem_ty = read_u32(r)?;
            let count = read_u64(r)?;
            for _ in 0..count {
                skip_value(r, elem_ty)?;
            }
            Some(())
        }
        _ => None, // 未知类型:长度不可知,放弃解析
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn put_string(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    /// 手造最小 GGUF:magic + v3 头 + 若干 KV。
    fn gguf(kvs: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GGUF");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        b.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for (key, ty, val) in kvs {
            put_string(&mut b, key);
            b.extend_from_slice(&ty.to_le_bytes());
            b.extend_from_slice(val);
        }
        b
    }

    fn string_val(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        put_string(&mut v, s);
        v
    }

    /// string array 值(模拟词表):跳过逻辑必须逐项走对才能读到后面的模板
    fn string_array_val(items: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&8u32.to_le_bytes()); // elem type = string
        v.extend_from_slice(&(items.len() as u64).to_le_bytes());
        for s in items {
            put_string(&mut v, s);
        }
        v
    }

    fn probe(bytes: &[u8]) -> Option<bool> {
        let path =
            std::env::temp_dir().join(format!("gguf-probe-{}.gguf", uuid::Uuid::new_v4().simple()));
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        drop(f);
        let out = chat_template_supports_tools(&path);
        let _ = std::fs::remove_file(&path);
        out
    }

    #[test]
    fn template_with_tools_branch_is_supported() {
        let b = gguf(&[
            // 词表数组排在模板之前:验证数组跳过走得通
            (
                "tokenizer.ggml.tokens",
                9,
                string_array_val(&["<s>", "</s>", "hi"]),
            ),
            (
                "tokenizer.chat_template",
                8,
                string_val("{%- if tools %}...{%- endif %}{{ messages }}"),
            ),
        ]);
        assert_eq!(probe(&b), Some(true));
    }

    #[test]
    fn template_without_tools_is_unsupported() {
        let b = gguf(&[(
            "tokenizer.chat_template",
            8,
            string_val("{{ bos_token }}{% for m in messages %}...{% endfor %}"),
        )]);
        assert_eq!(probe(&b), Some(false));
    }

    #[test]
    fn dedicated_tool_use_template_key_wins() {
        let b = gguf(&[(
            "tokenizer.chat_template.tool_use",
            8,
            string_val("<tool template>"),
        )]);
        assert_eq!(probe(&b), Some(true));
    }

    /// 真机探针:打印 GGUF_PROBE_DIR 目录下每个 GGUF 的工具调用判定,
    /// `cargo test -- --ignored --nocapture` 手动跑,排查"某模型为何(不)被过滤"。
    #[test]
    #[ignore]
    fn probe_local_models_tool_support() {
        let dir = std::env::var("GGUF_PROBE_DIR").expect("设置 GGUF_PROBE_DIR 指向模型目录");
        for entry in std::fs::read_dir(&dir).expect("读目录失败").flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
            {
                println!(
                    "{:?} -> {:?}",
                    path.file_name().unwrap(),
                    chat_template_supports_tools(&path)
                );
            }
        }
    }

    #[test]
    fn missing_template_or_bad_magic_degrade_gracefully() {
        // 无模板键:读不出结论(fail-open,由调用方决定显示)
        let b = gguf(&[("general.architecture", 8, string_val("llama"))]);
        assert_eq!(probe(&b), None);
        // 坏 magic:解析失败
        assert_eq!(probe(b"NOPE1234"), None);
    }
}
