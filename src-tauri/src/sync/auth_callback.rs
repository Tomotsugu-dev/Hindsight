//! OAuth loopback 回调页 HTML —— 浏览器拿到 code 之后看到的 "登录成功" / "登录失败" 页面。
//!
//! 配色与 `src/styles/tokens.css` 对齐：
//! - accent: #6c5ce7（深紫）
//! - 背景：sidebar 同款 lavender → pink → peach 径向渐变
//! - 字体 Inter / 中文 fallback；过渡曲线 cubic-bezier(0.22, 1, 0.36, 1)
//!
//! 为什么把它从 auth.rs 拆出来：原文件 600+ 行，一半是这页 HTML 字面量，
//! 把 OAuth 流程逻辑挤在底下不容易找。

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn render(success: bool, message: &str) -> String {
    let title = if success { "登录成功" } else { "登录失败" };
    let (icon_color, icon_bg) = if success {
        ("#6c5ce7", "rgba(108, 92, 231, 0.13)")
    } else {
        ("#ef4444", "rgba(239, 68, 68, 0.12)")
    };
    let icon_svg = if success {
        // checkmark
        r#"<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>"#
    } else {
        // alert circle
        r#"<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>"#
    };
    format!(
        r#"<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Hindsight · {title}</title>
<style>
  *,*::before,*::after {{ box-sizing: border-box; }}
  html, body {{ margin: 0; padding: 0; height: 100%; }}
  body {{
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
                 "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif;
    background:
      radial-gradient(120% 80% at 0% 0%, #efe7ff 0%, #ffe9f0 60%, #fff4e6 100%);
    color: #1d1c25;
    min-height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }}
  .card {{
    width: 100%;
    max-width: 580px;
    background: #ffffff;
    border: 1px solid rgba(20, 20, 40, 0.06);
    border-radius: 28px;
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.7) inset,
      0 0 0 1px rgba(255, 255, 255, 0.5) inset,
      0 16px 48px rgba(20, 20, 40, 0.10),
      0 3px 8px rgba(20, 20, 40, 0.04);
    padding: 50px 44px 40px;
    text-align: center;
    animation: rise 360ms cubic-bezier(0.22, 1, 0.36, 1);
  }}
  @keyframes rise {{
    from {{ opacity: 0; transform: translateY(12px); }}
    to   {{ opacity: 1; transform: translateY(0);   }}
  }}
  .badge {{
    width: 80px;
    height: 80px;
    margin: 0 auto 22px;
    border-radius: 22px;
    background: {icon_bg};
    color: {icon_color};
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }}
  h1 {{
    font-size: 26px;
    font-weight: 650;
    color: #1d1c25;
    margin: 0 0 10px;
    letter-spacing: -0.01em;
  }}
  p {{
    margin: 0;
    font-size: 18px;
    line-height: 1.55;
    color: #6b6680;
    word-break: break-word;
  }}
  .brand {{
    margin-top: 32px;
    padding-top: 22px;
    border-top: 1px solid rgba(20, 20, 40, 0.06);
    font-size: 15px;
    color: #9a96aa;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 550;
  }}
</style>
</head>
<body>
  <div class="card">
    <div class="badge">{icon_svg}</div>
    <h1>{title}</h1>
    <p>{message}</p>
    <div class="brand">Hindsight</div>
  </div>
</body>
</html>"#,
        title = title,
        icon_color = icon_color,
        icon_bg = icon_bg,
        icon_svg = icon_svg,
        message = message,
    )
}
