//! 命中上下文：在屏幕文字里找到关键词，前后各留一段。搜索页和对话的工具都用它。

/// 在 `text` 里找到 `word`，往前留 `before` 个字，往后留 `after` 个字。两端截掉了就补省略号。
///
/// 找不到 `word` 时从头开始截。屏幕文字是 OCR 行拼起来的，换行换成空格，好在一行里显示。
pub(crate) fn snippet(text: &str, word: &str, before: usize, after: usize) -> String {
    let flat = text.replace(['\n', '\r'], " ");
    let chars: Vec<char> = flat.chars().collect();
    let hit = find_ci(&chars, word).unwrap_or(0);
    let start = hit.saturating_sub(before);
    let end = (hit + word.chars().count() + after).min(chars.len());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_both_sides_and_marks_the_cuts() {
        let text = format!("{}报销单{}", "前".repeat(30), "后".repeat(30));
        let s = snippet(&text, "报销", 5, 4);
        assert_eq!(s, "…前前前前前报销单后后后…");
    }

    #[test]
    fn no_marks_when_nothing_is_cut() {
        assert_eq!(snippet("请提交报销单", "报销", 20, 60), "请提交报销单");
    }

    #[test]
    fn english_ignores_case() {
        assert_eq!(
            snippet("Open the Rust Book now", "rust book", 0, 0),
            "…Rust Book…"
        );
    }

    #[test]
    fn starts_from_the_beginning_when_the_word_is_missing() {
        assert_eq!(snippet("abcdefgh", "xyz", 2, 2), "abcde…");
    }

    #[test]
    fn line_breaks_become_spaces() {
        assert_eq!(
            snippet("第一行\n报销\r\n第三行", "报销", 20, 60),
            "第一行 报销  第三行"
        );
    }
}
