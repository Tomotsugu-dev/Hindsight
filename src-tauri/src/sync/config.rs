//! 云同步凭证：BYO（Bring Your Own）模式 —— 每个用户在设置 UI 里填自己的
//! Firebase 项目凭证。值存在 `settings_store`（本地 SQLite）。
//!
//! 用户操作流程：
//! 1. 在 Firebase Console 建一个自己的项目（免费额度独占）
//! 2. 启用 Authentication → Google sign-in provider
//! 3. 拿 Web API Key（Project Settings）+ OAuth 2.0 Client ID/Secret（Google Cloud Console，
//!    Application type = Desktop app）
//! 4. 设置 → 云同步 → 填进三个字段
//! 5. 点登录 → 用自己的 Google 账号授权 → 数据进自己的 Firebase 项目
//!
//! 凭证存储：明文落本地 DB（client_secret 对桌面应用不算真正的 secret，且这是用户
//! 自己的 secret，落自己机器上不涉及跨用户泄露）。Firestore 访问权由用户自己的
//! Security Rules 控制 + per-user refresh_token 仍走 keyring + AES-GCM。

use crate::repo::settings::Settings;

#[derive(Debug, Clone)]
pub struct FullConfig {
    pub google_client_id: String,
    pub google_client_secret: String,
    pub firebase_api_key: String,
}

impl FullConfig {
    /// 从 Settings 抽取凭证。三件套必须都非空，否则返回 None（UI 应提示"未配置"）。
    pub fn from_settings(s: &Settings) -> Option<Self> {
        let id = s.firebase_client_id.trim();
        let secret = s.firebase_client_secret.trim();
        let key = s.firebase_api_key.trim();
        if id.is_empty() || secret.is_empty() || key.is_empty() {
            return None;
        }
        Some(FullConfig {
            google_client_id: id.to_string(),
            google_client_secret: secret.to_string(),
            firebase_api_key: key.to_string(),
        })
    }
}
