//! 云端视觉调用:降采样 → OpenAI 兼容 vision chat → 解析"一句话 + 实体"。
//!
//! 调用规格见 docs/design/cloud-insight.md §4:单图单调用(多图有实测的
//! 注意力稀释)、文字密集内容 1280px 底线、输出契约 ≤80 字、temperature 0。

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::error::{Error, Result};

/// 单次调用输出上限——契约是 ≤80 字,给非拉丁语言与实体列表留余量。
const MAX_TOKENS: u32 = 200;
/// 网络/限流重试:次数与首个退避秒数(指数)。
const RETRIES: u32 = 3;
const BACKOFF_SECS: u64 = 1;

/// 帧洞察提示词——语言跟随 `ai.prompt_language`(洞察喂给总结,同一语言体系)。
fn prompt_for(lang: &str) -> &'static str {
    match lang {
        "tw" => "用一句話說明這張截圖裡使用者正在做什麼;然後另起一行,列出畫面上的關鍵實體(專案名/檔案名/網站/文件主題/工具名),逗號分隔。總共不超過 80 字,不要其他內容。",
        "en" => "In one sentence, state what the user is doing in this screenshot; then on a new line, list the key entities on screen (project/file/site/document topic/tool names), comma-separated. At most 60 words total, nothing else.",
        "ja" => "このスクリーンショットでユーザーが何をしているかを一文で述べ、改行して画面上の重要なエンティティ(プロジェクト名/ファイル名/サイト/文書テーマ/ツール名)をカンマ区切りで列挙してください。全体で 80 字以内、他の内容は不要です。",
        "pt" => "Em uma frase, diga o que o usuário está fazendo nesta captura de tela; em seguida, em nova linha, liste as entidades-chave na tela (projeto/arquivo/site/tema do documento/ferramentas), separadas por vírgula. No máximo 60 palavras, nada mais.",
        _ => "用一句话说明这张截图里用户正在做什么;然后另起一行,列出屏幕上的关键实体(项目名/文件名/网站/文档主题/工具名),逗号分隔。总共不超过 80 字,不要其他内容。",
    }
}

/// 读帧文件并降采样到长边 `max_side`,重编码 JPEG。CPU 工作放 blocking 线程。
pub async fn downscale_jpeg(path: &Path, max_side: u32) -> Result<Vec<u8>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let img = image::open(&path).map_err(|e| Error::Ocr(format!("读帧失败: {e}")))?;
        let (w, h) = (img.width(), img.height());
        let img = if w.max(h) > max_side {
            img.thumbnail(max_side, max_side)
        } else {
            img
        };
        let mut out = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
        enc.encode_image(&img.to_rgb8())
            .map_err(|e| Error::Ocr(format!("JPEG 编码失败: {e}")))?;
        Ok(out)
    })
    .await
    .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))?
}

/// 解析模型回复:首个非空行 = 一句话洞察,其余行拼成实体列表。
/// 两个字段都截断兜底——落库的是"给总结看的物料",不是无界文本。
pub fn parse_reply(text: &str) -> (String, String) {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let insight: String = lines.next().unwrap_or("").chars().take(200).collect();
    let entities: String = lines
        .collect::<Vec<_>>()
        .join(", ")
        .chars()
        .take(300)
        .collect();
    (insight, entities)
}

/// 单帧洞察调用。429/5xx/网络错误指数退避重试 [`RETRIES`] 次。
pub async fn describe(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    jpeg: &[u8],
    lang: &str,
) -> Result<(String, String)> {
    let reply = chat_with_image(client, endpoint, api_key, model, prompt_for(lang), jpeg).await?;
    Ok(parse_reply(&reply))
}

/// 视觉连通性测试:发一张**本地合成**的色块图(不含任何用户数据),
/// 要求一句话描述。返回模型回复原文供前端展示。
pub async fn test_connection(endpoint: &str, api_key: &str, model: &str) -> Result<String> {
    let jpeg = synthetic_image()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    chat_with_image(
        &client,
        endpoint,
        api_key,
        model,
        "Describe this image in one short sentence.",
        &jpeg,
    )
    .await
}

/// 合成测试图:纯色底 + 两个色块。无文字(免字体依赖),但足以验证
/// "端点可达 + 模型有视觉输入能力"。
fn synthetic_image() -> Result<Vec<u8>> {
    let mut img = image::RgbImage::from_pixel(320, 200, image::Rgb([245, 245, 245]));
    for (x0, y0, w, h, c) in [
        (30u32, 40u32, 120u32, 120u32, image::Rgb([200u8, 60, 60])),
        (180, 60, 100, 80, image::Rgb([60, 100, 200])),
    ] {
        for x in x0..(x0 + w).min(320) {
            for y in y0..(y0 + h).min(200) {
                img.put_pixel(x, y, c);
            }
        }
    }
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
    enc.encode_image(&img)
        .map_err(|e| Error::Ocr(format!("合成图编码失败: {e}")))?;
    Ok(out)
}

async fn chat_with_image(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    jpeg: &[u8],
) -> Result<String> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
    let base = endpoint.trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let body = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url",
                 "image_url": {"url": format!("data:image/jpeg;base64,{b64}")}},
            ],
        }],
        "max_tokens": MAX_TOKENS,
        "temperature": 0,
        "stream": false,
    });

    let mut last_err: Option<Error> = None;
    for attempt in 0..=RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(BACKOFF_SECS << (attempt - 1))).await;
        }
        let mut req = client.post(&url).json(&body);
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        };
        let status = resp.status();
        if status.as_u16() == 429 || status.is_server_error() {
            let text = resp.text().await.unwrap_or_default();
            last_err = Some(Error::Ocr(format!(
                "vision api {status}: {}",
                &text[..text.len().min(200)]
            )));
            continue;
        }
        if !status.is_success() {
            // 4xx(除 429)是配置错误,重试无意义,直接抛给调用方
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Ocr(format!(
                "vision api {status}: {}",
                &text[..text.len().min(300)]
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if content.trim().is_empty() {
            return Err(Error::Ocr("vision api 返回空内容".into()));
        }
        return Ok(content);
    }
    Err(last_err.unwrap_or_else(|| Error::Ocr("vision api 重试耗尽".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_splits_insight_and_entities() {
        let (i, e) = parse_reply("用户在写代码。\nVSCode, main.rs, GitHub");
        assert_eq!(i, "用户在写代码。");
        assert_eq!(e, "VSCode, main.rs, GitHub");
        // 只有一行 → 实体为空
        let (i, e) = parse_reply("只有一句话");
        assert_eq!(i, "只有一句话");
        assert_eq!(e, "");
        // 多行实体拼接 + 空行剔除
        let (_, e) = parse_reply("行1\n\nA, B\nC");
        assert_eq!(e, "A, B, C");
    }

    #[test]
    fn synthetic_image_encodes() {
        let jpeg = synthetic_image().unwrap();
        assert!(jpeg.len() > 500);
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]); // JPEG magic
    }
}
