//! Switching backends (ADR-0011 §2): saving the new backend and its credentials,
//! deleting the old backend's credentials, and marking every file of this device
//! for upload again, all in one transaction.

// Removed once the connect command calls `switch_backend` (ADR-0011 follow-up 5).
#![allow(dead_code)]

use rusqlite::types::Value;

use crate::error::Result;
use crate::repo::outbox::enqueue_every_file;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::local_key::{aes_encrypt, derive_master_key};
use crate::sync::webdav::account_hash::server_host;

/// The backend to switch to, with its credentials.
pub(crate) enum NewBackend<'a> {
    Drive {
        uid: &'a str,
        email: &'a str,
        refresh_token: &'a str,
        access_token: &'a str,
        expires_at: &'a str,
    },
    WebDav {
        server_url: &'a str,
        user: &'a str,
        password: &'a str,
    },
}

/// Switches to another backend. Call it while sync is paused, and restart the
/// app as soon as it returns.
pub(crate) async fn switch_backend(pool: &DbPool, self_id: &str, to: NewBackend<'_>) -> Result<()> {
    let key = derive_master_key()?;
    let statements: Vec<(&str, Vec<Value>)> = match to {
        NewBackend::Drive {
            uid,
            email,
            refresh_token,
            access_token,
            expires_at,
        } => vec![(
            "UPDATE auth_state
                SET backend = 'drive', drive_account = ?1,
                    uid = ?1, email = ?2, refresh_token_enc = ?3,
                    access_token = ?4, expires_at = ?5,
                    webdav_password_enc = NULL
              WHERE id = 1",
            vec![
                text(uid),
                text(email),
                Value::Blob(aes_encrypt(&key, refresh_token.as_bytes())?),
                text(access_token),
                text(expires_at),
            ],
        )],
        NewBackend::WebDav {
            server_url,
            user,
            password,
        } => vec![
            (
                "UPDATE auth_state
                    SET backend = 'webdav', webdav_url = ?1, webdav_user = ?2,
                        webdav_password_enc = ?3,
                        uid = NULL, email = NULL,
                        refresh_token_enc = NULL, access_token = NULL, expires_at = NULL
                  WHERE id = 1",
                vec![
                    text(server_url),
                    text(user),
                    Value::Blob(aes_encrypt(&key, password.as_bytes())?),
                ],
            ),
            (
                "INSERT INTO webdav_accounts(host, url, user) VALUES(?1, ?2, ?3)
                 ON CONFLICT(host) DO UPDATE SET url = excluded.url, user = excluded.user",
                vec![
                    Value::Text(server_host(server_url)?),
                    text(server_url),
                    text(user),
                ],
            ),
        ],
    };
    let self_id = self_id.to_string();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            for (sql, params) in &statements {
                tx.execute(sql, rusqlite::params_from_iter(params)).db()?;
            }
            enqueue_every_file(&tx, &self_id).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::sync::local_key::aes_decrypt;

    const URL: &str = "https://dav.jianguoyun.com/dav/";

    /// 登录着 Google 的库：本机两天的活动，对端一天的活动，outbox 是空的。
    async fn signed_in_to_drive() -> DbPool {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute_batch(
                    "UPDATE auth_state
                        SET uid = 'g-1', email = 'a@example.com', refresh_token_enc = x'00',
                            access_token = 'token', expires_at = '2099-01-01T00:00:00Z',
                            backend = 'drive', drive_account = 'g-1'
                      WHERE id = 1;
                     INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                                            local_hour, process_name, category_id, device_id)
                     VALUES ('2026-09-01T10:00:00Z', '2026-09-01T10:01:00Z', 60, '2026-09-01',
                             10, 'Code', 'other', 'me'),
                            ('2026-09-01T11:00:00Z', '2026-09-01T11:01:00Z', 60, '2026-09-01',
                             11, 'Code', 'other', 'me'),
                            ('2026-09-02T10:00:00Z', '2026-09-02T10:01:00Z', 60, '2026-09-02',
                             10, 'Code', 'other', 'me'),
                            ('2026-09-03T10:00:00Z', '2026-09-03T10:01:00Z', 60, '2026-09-03',
                             10, 'Code', 'other', 'peer');
                     DELETE FROM sync_outbox;",
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        pool
    }

    async fn query<T: Send + 'static>(
        pool: &DbPool,
        sql: &'static str,
        row: fn(&rusqlite::Row) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        pool.0
            .call(move |conn| {
                let mut stmt = conn.prepare(sql).db()?;
                let rows = stmt
                    .query_map([], row)
                    .db()?
                    .collect::<rusqlite::Result<Vec<T>>>()
                    .db()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    fn to_nutstore() -> NewBackend<'static> {
        NewBackend::WebDav {
            server_url: URL,
            user: "you@example.com",
            password: "app-password",
        }
    }

    /// outbox 里的 (entity, payload)，排好序。
    async fn outbox(pool: &DbPool) -> Vec<(String, String)> {
        let mut rows = query(pool, "SELECT entity, payload FROM sync_outbox", |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .await;
        rows.sort();
        rows
    }

    /// 本机的两天加五个整表文件；对端那天不在里面。
    fn every_file_of_me() -> Vec<(String, String)> {
        [
            ("activity", r#"{"localDate":"2026-09-01"}"#),
            ("activity", r#"{"localDate":"2026-09-02"}"#),
            ("app_group", "{}"),
            ("app_group_member", "{}"),
            ("app_icon", "{}"),
            ("category", "{}"),
            ("device", "{}"),
        ]
        .map(|(e, p)| (e.to_string(), p.to_string()))
        .to_vec()
    }

    /// 三件事都做了：存下服务器和加密的密码、清掉 Google 的登录信息、记下这台服务器上
    /// 用的账号、本机的两天加五个整表文件进 outbox。对端那天不进。
    #[tokio::test]
    async fn switching_to_webdav_saves_the_server_and_marks_every_file() {
        let pool = signed_in_to_drive().await;
        switch_backend(&pool, "me", to_nutstore()).await.unwrap();

        let auth = query(
            &pool,
            "SELECT backend, webdav_url, webdav_user, webdav_password_enc, drive_account,
                    uid IS NULL AND email IS NULL AND refresh_token_enc IS NULL
                    AND access_token IS NULL AND expires_at IS NULL
               FROM auth_state",
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            },
        )
        .await
        .remove(0);
        assert_eq!(auth.0, "webdav");
        assert_eq!(auth.1, URL);
        assert_eq!(auth.2, "you@example.com");
        let password = aes_decrypt(&derive_master_key().unwrap(), &auth.3).unwrap();
        assert_eq!(password, b"app-password");
        assert_eq!(auth.4, "g-1", "drive_account 留着");
        assert!(auth.5, "Google 的登录信息清掉了");

        let accounts = query(&pool, "SELECT host, url, user FROM webdav_accounts", |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .await;
        assert_eq!(
            accounts,
            vec![(
                "dav.jianguoyun.com".to_string(),
                URL.to_string(),
                "you@example.com".to_string()
            )]
        );

        assert_eq!(outbox(&pool).await, every_file_of_me());
    }

    /// 从 WebDAV 换回 Drive：存下 Google 的登录信息，refresh token 加密；删掉 WebDAV 的
    /// 密码，地址和用户名留着；outbox 照样重填。
    #[tokio::test]
    async fn switching_back_to_drive_saves_the_token_and_drops_the_password() {
        let pool = signed_in_to_drive().await;
        switch_backend(&pool, "me", to_nutstore()).await.unwrap();
        pool.0
            .call(|conn| {
                conn.execute("DELETE FROM sync_outbox", []).db()?;
                Ok(())
            })
            .await
            .unwrap();

        let to_drive = NewBackend::Drive {
            uid: "g-1",
            email: "a@example.com",
            refresh_token: "refresh",
            access_token: "access",
            expires_at: "2099-01-01T00:00:00Z",
        };
        switch_backend(&pool, "me", to_drive).await.unwrap();

        let auth = query(
            &pool,
            "SELECT backend, drive_account, uid, refresh_token_enc, access_token,
                    webdav_url, webdav_password_enc IS NULL
               FROM auth_state",
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            },
        )
        .await
        .remove(0);
        assert_eq!(auth.0, "drive");
        assert_eq!(auth.1, "g-1");
        assert_eq!(auth.2, "g-1");
        let refresh = aes_decrypt(&derive_master_key().unwrap(), &auth.3).unwrap();
        assert_eq!(refresh, b"refresh");
        assert_eq!(auth.4, "access");
        assert_eq!(auth.5, URL, "WebDAV 的地址留着");
        assert!(auth.6, "WebDAV 的密码删掉了");
        assert_eq!(outbox(&pool).await, every_file_of_me());
    }

    /// 中途失败整个回滚：outbox 表没了，第三件事失败，前两件也不留下。
    #[tokio::test]
    async fn a_failed_switch_changes_nothing() {
        let pool = signed_in_to_drive().await;
        pool.0
            .call(|conn| {
                conn.execute("DROP TABLE sync_outbox", []).db()?;
                Ok(())
            })
            .await
            .unwrap();

        assert!(switch_backend(&pool, "me", to_nutstore()).await.is_err());

        let auth = query(
            &pool,
            "SELECT backend, uid, webdav_url IS NULL FROM auth_state",
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .await;
        assert_eq!(auth, vec![("drive".to_string(), "g-1".to_string(), true)]);
        let accounts = query(&pool, "SELECT COUNT(*) FROM webdav_accounts", |r| {
            r.get::<_, i64>(0)
        })
        .await;
        assert_eq!(accounts, vec![0]);
    }
}
