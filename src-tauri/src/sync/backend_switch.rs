//! Switching the sync backend ([`CloudBackend`](crate::sync::cloud::CloudBackend):
//! Google Drive or a WebDAV server; ADR-0011 §2): saving the new backend's
//! credentials, deleting the old backend's, and marking every file of this device
//! for upload again, all in one transaction.

use rusqlite::types::Value;
use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::repo::outbox::enqueue_every_file;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::local_key::{aes_encrypt, derive_master_key};
use crate::sync::webdav::account_hash::{account_hash, server_host};

/// The backend to switch to, with its credentials.
pub(crate) enum NewBackend<'a> {
    // Google sign-in switches with it next (ADR-0011 follow-up 6).
    #[allow(dead_code)]
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
    let statements = credential_statements(to)?;
    let self_id = self_id.to_string();
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            execute_all(&tx, &statements).db()?;
            enqueue_every_file(&tx, &self_id).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The account connected now, with a new password, token or spelling: only the
/// credentials change. The backend stays, so nothing is uploaded again and no
/// restart is needed.
pub(crate) async fn update_credentials(pool: &DbPool, to: NewBackend<'_>) -> Result<()> {
    let statements = credential_statements(to)?;
    pool.0
        .call(move |conn| {
            let tx = conn.transaction().db()?;
            execute_all(&tx, &statements).db()?;
            tx.commit().db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

type Statement = (&'static str, Vec<Value>);

/// The SQL that saves the new backend's credentials and deletes the other
/// backend's. Passwords and tokens are encrypted here, before the transaction.
fn credential_statements(to: NewBackend<'_>) -> Result<Vec<Statement>> {
    let key = derive_master_key()?;
    Ok(match to {
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
    })
}

fn execute_all(tx: &rusqlite::Transaction, statements: &[Statement]) -> rusqlite::Result<()> {
    for (sql, params) in statements {
        tx.execute(sql, rusqlite::params_from_iter(params))?;
    }
    Ok(())
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

/// What switching the cloud backend takes (ADR-0011 §1).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConnectAction {
    /// This account is the one connected now: only update its credentials.
    UpdateCredentials,
    /// Keep this database and switch backends in it: it recorded this account on
    /// this backend, or no account there yet.
    UseThisDatabase,
    /// A database that belongs to no account yet: claim it for this account, then
    /// switch backends.
    ClaimThisDatabase,
    /// Switch to the new account's own database: this database used another
    /// account on this backend.
    SwitchDatabase,
}

/// Works out what switching to the account in `to` takes, without writing
/// anything. `active_uid` is the account this database belongs to; `None` if it
/// belongs to no account yet.
pub(crate) async fn decide_connect_action(
    pool: &DbPool,
    active_uid: Option<&str>,
    to: &NewBackend<'_>,
) -> Result<ConnectAction> {
    let record = match to {
        NewBackend::Drive { uid, .. } => drive_record(pool, uid).await?,
        NewBackend::WebDav {
            server_url, user, ..
        } => webdav_record(pool, server_url, user).await?,
    };
    Ok(match (record, active_uid) {
        (AccountRecord::ConnectedNow, _) => ConnectAction::UpdateCredentials,
        (AccountRecord::UsedBefore, _) => ConnectAction::UseThisDatabase,
        (AccountRecord::UsedAnother, _) => ConnectAction::SwitchDatabase,
        // Never used this backend: a database with no account yet is claimed, one
        // that has an account is kept
        (AccountRecord::NeverUsed, None) => ConnectAction::ClaimThisDatabase,
        (AccountRecord::NeverUsed, Some(_)) => ConnectAction::UseThisDatabase,
    })
}

/// What this database recorded about the accounts on this backend, compared with
/// the new account.
enum AccountRecord {
    /// It is the account connected now.
    ConnectedNow,
    /// It was used before and is not connected now.
    UsedBefore,
    /// Another account was used.
    UsedAnother,
    /// No account was ever recorded.
    NeverUsed,
}

/// Drive: compares the `drive_account` this database recorded with `uid`, the
/// Google account just signed in.
async fn drive_record(pool: &DbPool, uid: &str) -> Result<AccountRecord> {
    let (backend, drive_account): (Option<String>, Option<String>) = pool
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT backend, drive_account FROM auth_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .db()
        })
        .await?;
    Ok(match drive_account {
        None => AccountRecord::NeverUsed,
        Some(account) if account != uid => AccountRecord::UsedAnother,
        Some(_) if backend.as_deref() == Some("drive") => AccountRecord::ConnectedNow,
        Some(_) => AccountRecord::UsedBefore,
    })
}

/// WebDAV: compares the account connected now, and the one `webdav_accounts`
/// recorded for this WebDAV provider (by its host), with the account just entered.
async fn webdav_record(pool: &DbPool, server_url: &str, user: &str) -> Result<AccountRecord> {
    let host = server_host(server_url)?;
    let (current, recorded) = pool
        .0
        .call(move |conn| {
            let current: Option<(String, String)> = conn
                .query_row(
                    "SELECT webdav_url, webdav_user FROM auth_state
                      WHERE id = 1 AND backend = 'webdav'
                        AND webdav_url IS NOT NULL AND webdav_user IS NOT NULL",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .db()?;
            let recorded: Option<(String, String)> = conn
                .query_row(
                    "SELECT url, user FROM webdav_accounts WHERE host = ?1",
                    [&host],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .db()?;
            Ok((current, recorded))
        })
        .await?;

    // Recognize the same account however it is spelled: account hashes are
    // computed after normalizing
    let new_account = account_hash(server_url, user)?;
    let same_as_new = |(url, user): &(String, String)| -> Result<bool> {
        Ok(account_hash(url, user)? == new_account)
    };
    if let Some(current) = &current {
        if same_as_new(current)? {
            return Ok(AccountRecord::ConnectedNow);
        }
    }
    Ok(match &recorded {
        None => AccountRecord::NeverUsed,
        Some(recorded) if same_as_new(recorded)? => AccountRecord::UsedBefore,
        Some(_) => AccountRecord::UsedAnother,
    })
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

    /// 同一个账号只是换了密码和写法：凭证更新了，outbox 里什么都没多，不会重新上传。
    #[tokio::test]
    async fn updating_credentials_uploads_nothing_again() {
        let pool = signed_in_to_drive().await;
        switch_backend(&pool, "me", to_nutstore()).await.unwrap();
        pool.0
            .call(|conn| {
                conn.execute("DELETE FROM sync_outbox", []).db()?;
                Ok(())
            })
            .await
            .unwrap();

        let new_password = NewBackend::WebDav {
            server_url: URL,
            user: "You@Example.com",
            password: "new-password",
        };
        update_credentials(&pool, new_password).await.unwrap();

        let auth = query(
            &pool,
            "SELECT webdav_user, webdav_password_enc FROM auth_state",
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .await
        .remove(0);
        assert_eq!(auth.0, "You@Example.com");
        let password = aes_decrypt(&derive_master_key().unwrap(), &auth.1).unwrap();
        assert_eq!(password, b"new-password");
        assert!(outbox(&pool).await.is_empty());
    }

    /// 先执行 `sql` 摆好库的状态，再判断换到 `to` 要做哪件事。
    async fn action_after(
        sql: &'static str,
        active_uid: Option<&str>,
        to: NewBackend<'_>,
    ) -> ConnectAction {
        let pool = fresh_test_pool().await;
        pool.0
            .call(move |conn| {
                conn.execute_batch(sql).db()?;
                Ok(())
            })
            .await
            .unwrap();
        decide_connect_action(&pool, active_uid, &to).await.unwrap()
    }

    fn webdav(server_url: &'static str, user: &'static str) -> NewBackend<'static> {
        NewBackend::WebDav {
            server_url,
            user,
            password: "",
        }
    }

    fn drive(uid: &'static str) -> NewBackend<'static> {
        NewBackend::Drive {
            uid,
            email: "",
            refresh_token: "",
            access_token: "",
            expires_at: "",
        }
    }

    /// 换到 Google 账号时同样是这四种情况，比的是 `drive_account`。
    #[tokio::test]
    async fn drive_action_follows_what_this_database_recorded() {
        use ConnectAction::*;
        let on_drive = "UPDATE auth_state SET backend = 'drive', drive_account = 'g-1';";
        let on_webdav = "UPDATE auth_state SET backend = 'webdav', drive_account = 'g-1';";

        // 现在用的就是 Drive g-1，只是重新登录
        assert_eq!(
            action_after(on_drive, Some("g-1"), drive("g-1")).await,
            UpdateCredentials
        );
        // 记着 g-1，现在在用 WebDAV
        assert_eq!(
            action_after(on_webdav, Some("g-1"), drive("g-1")).await,
            UseThisDatabase
        );
        // 记着的是 g-1，这次登的是 g-2
        assert_eq!(
            action_after(on_drive, Some("g-1"), drive("g-2")).await,
            SwitchDatabase
        );
        // 从没用过 Drive：有主人的库就沿用，还没归属账号的库就认领
        assert_eq!(
            action_after("", Some("webdav-2538daca8be4c388"), drive("g-1")).await,
            UseThisDatabase
        );
        assert_eq!(
            action_after("", None, drive("g-1")).await,
            ClaimThisDatabase
        );
    }

    /// 四种情况各举一例（ADR-0011 §1）。
    #[tokio::test]
    async fn webdav_action_follows_what_this_database_recorded() {
        use ConnectAction::*;
        let on_nutstore = "UPDATE auth_state SET backend = 'webdav',
                             webdav_url = 'https://dav.jianguoyun.com/dav/',
                             webdav_user = 'you@example.com';
                           INSERT INTO webdav_accounts VALUES
                             ('dav.jianguoyun.com', 'https://dav.jianguoyun.com/dav/',
                              'you@example.com');";
        let back_on_drive = "UPDATE auth_state SET backend = 'drive';
                             INSERT INTO webdav_accounts VALUES
                               ('dav.jianguoyun.com', 'https://dav.jianguoyun.com/dav/',
                                'you@example.com');";

        // 当前连着的就是这个账号，只是写法不同
        let spelled = webdav("HTTPS://DAV.jianguoyun.com/dav", "You@Example.com");
        assert_eq!(
            action_after(on_nutstore, Some("g-1"), spelled).await,
            UpdateCredentials
        );
        // 用过这个账号，现在在 Drive 上
        assert_eq!(
            action_after(back_on_drive, Some("g-1"), webdav(URL, "you@example.com")).await,
            UseThisDatabase
        );
        // 这台服务器上用的是另一个账号：不管现在连着它还是在 Drive 上
        assert_eq!(
            action_after(on_nutstore, Some("g-1"), webdav(URL, "me@example.com")).await,
            SwitchDatabase
        );
        assert_eq!(
            action_after(back_on_drive, Some("g-1"), webdav(URL, "me@example.com")).await,
            SwitchDatabase
        );
        // 这台服务器上从没记过账号
        assert_eq!(
            action_after("", Some("g-1"), webdav(URL, "you@example.com")).await,
            UseThisDatabase
        );
        // 还没归属账号的库第一次连接
        assert_eq!(
            action_after("", None, webdav(URL, "you@example.com")).await,
            ClaimThisDatabase
        );
    }
}
