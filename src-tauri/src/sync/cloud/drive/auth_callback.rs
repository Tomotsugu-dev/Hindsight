//! Local callback page for Google OAuth, displaying the sign-in result in the system language.

/// Replace `&`, `<`, and `>` with HTML entities so the browser displays them as text instead of treating them as tags.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Language-specific text for the page.
struct PageText {
    /// The value for the `<html lang>` attribute.
    html_lang: &'static str,
    signed_in: &'static str,
    sign_in_failed: &'static str,
    close_hint: &'static str,
}

/// Returns the appropriate set of page text based on
/// the language code from [`crate::platform::system_language`].
fn page_text(lang: &str) -> PageText {
    match lang {
        "zh" => PageText {
            html_lang: "zh-CN",
            signed_in: "登录成功",
            sign_in_failed: "登录失败",
            close_hint: "可以关闭此页，回到 Hindsight。",
        },
        "tw" => PageText {
            html_lang: "zh-TW",
            signed_in: "登入成功",
            sign_in_failed: "登入失敗",
            close_hint: "可以關閉此頁，回到 Hindsight。",
        },
        "ja" => PageText {
            html_lang: "ja",
            signed_in: "ログインしました",
            sign_in_failed: "ログインできませんでした",
            close_hint: "このページを閉じて、Hindsight に戻ってください。",
        },
        "pt" => PageText {
            html_lang: "pt-BR",
            signed_in: "Login concluído",
            sign_in_failed: "Não foi possível entrar",
            close_hint: "Você pode fechar esta página e voltar ao Hindsight.",
        },
        "es" => PageText {
            html_lang: "es",
            signed_in: "Sesión iniciada",
            sign_in_failed: "No se pudo iniciar sesión",
            close_hint: "Puedes cerrar esta página y volver a Hindsight.",
        },
        _ => PageText {
            html_lang: "en",
            signed_in: "Signed in",
            sign_in_failed: "Sign-in failed",
            close_hint: "You can close this page and go back to Hindsight.",
        },
    }
}

/// The page displayed in the browser when sign-in is successful.
pub fn success_page() -> String {
    let text = page_text(crate::platform::system_language());
    render(text.html_lang, true, text.signed_in, text.close_hint)
}

/// The page displayed in the browser when sign-in fails;
/// `error` is the error code returned by Google, escaped and displayed as-is.
pub fn failure_page(error: &str) -> String {
    let text = page_text(crate::platform::system_language());
    render(
        text.html_lang,
        false,
        text.sign_in_failed,
        &html_escape(error),
    )
}

/// Renders the callback page HTML.
/// This page is displayed in the browser when OAuth redirects back to the loopback URL.
fn render(html_lang: &str, success: bool, title: &str, message: &str) -> String {
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
<html lang="{html_lang}">
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
        html_lang = html_lang,
        title = title,
        icon_color = icon_color,
        icon_bg = icon_bg,
        icon_svg = icon_svg,
        message = message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 六种语言各有自己的一套文字；认不出的语言代码用英文。
    #[test]
    fn page_text_follows_the_language_code() {
        for (lang, html_lang) in [
            ("zh", "zh-CN"),
            ("tw", "zh-TW"),
            ("ja", "ja"),
            ("pt", "pt-BR"),
            ("es", "es"),
            ("en", "en"),
        ] {
            assert_eq!(page_text(lang).html_lang, html_lang);
        }
        assert_eq!(page_text("fr").signed_in, "Signed in");
    }

    /// 失败页里的错误码要转义：Google 回传的内容不能当 HTML 插进页面。
    #[test]
    fn failure_page_escapes_the_error() {
        let html = failure_page("<script>alert(1)</script>");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>"));
    }
}
