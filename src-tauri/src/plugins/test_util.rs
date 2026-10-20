//! 插件测试共用：临时文件夹、造 zip。

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::plugins::install;
use crate::plugins::market::{InstallKind, OnlinePlugin, OnlineVersion};
use crate::repo::plugins::{Plugin, UiSettings};
use crate::storage::DbPool;

/// 一个测试一个空文件夹，放 zip 和插件库，测完删掉。
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "hindsight-plugins-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn plugins(&self) -> PathBuf {
        self.0.join("plugins")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// plugins 表里的一行，版本 0.1.0，没有权限，没有磁盘容量。
pub fn plugin_row(id: &str, enabled: bool, runs_in_background: bool) -> Plugin {
    Plugin {
        id: id.into(),
        name: id.into(),
        version: "0.1.0".into(),
        enabled,
        permissions: vec![],
        storage_mb: None,
        ui: UiSettings {
            has_page: true,
            opens_in: Some("main".into()),
            window_x: None,
            window_y: None,
            window_width: 900,
            window_height: 640,
        },
        runs_in_background,
        installed_at: "2026-10-18T00:00:00+00:00".into(),
    }
}

/// 开着的插件，带这些权限。
pub fn plugin_with_permissions(id: &str, permissions: &[&str]) -> Plugin {
    let mut row = plugin_row(id, true, true);
    row.permissions = permissions.iter().map(|p| p.to_string()).collect();
    row
}

pub fn manifest_json(version: &str) -> String {
    serde_json::json!({
        "id": "hindsight-focus",
        "name": "Focus",
        "version": version,
        "permissions": ["stats.read", "stats.today", "focus.changed", "focus.category"],
        "runs_in_background": true
    })
    .to_string()
}

pub fn write_zip(path: &Path, files: &[(&str, &str)]) {
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|&(name, body)| (name, body.as_bytes()))
        .collect();
    write_zip_bytes(path, &files);
}

/// 和 [`write_zip`] 一样，但内容是字节，用来放 PNG 这类不是文本的文件。
pub fn write_zip_bytes(path: &Path, files: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    for (name, body) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap();
}

/// 专注插件的 zip，background.js 的内容就是版本号，方便看装的是哪一版。
pub fn focus_zip(dir: &TempDir, version: &str) -> PathBuf {
    let path = dir.0.join(format!("focus-{version}.zip"));
    let manifest = manifest_json(version);
    write_zip(
        &path,
        &[("manifest.json", &manifest), ("background.js", version)],
    );
    path
}

/// 装上专注插件的这个版本，不给权限，也不给磁盘容量。
pub async fn install_focus(pool: &DbPool, dir: &TempDir, version: &str) {
    install::install(pool, &dir.plugins(), &focus_zip(dir, version), vec![], None)
        .await
        .unwrap();
}

/// 线上列表里的一个插件，只有 id 和各个版本号要紧。
pub fn online_plugin(id: &str, versions: &[&str]) -> OnlinePlugin {
    OnlinePlugin {
        id: id.into(),
        name: id.into(),
        author: "someone".into(),
        description: String::new(),
        repo: format!("someone/{id}"),
        versions: versions
            .iter()
            .map(|version| OnlineVersion {
                version: version.to_string(),
                url: format!("https://example.com/{id}-{version}.zip"),
                sha256: String::new(),
                permissions: vec!["stats.read".into()],
                storage_mb: None,
                kind: InstallKind::default(),
            })
            .collect(),
        icon_url: None,
        official: false,
    }
}

/// 在本机起一个只会回 `body` 的 HTTP 服务，返回下载地址。用来测下载，不连外网。
pub async fn serve(body: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/plugin.zip", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            // GET 没有请求体，读到空行就是整个请求
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
        }
    });
    url
}

/// 文件夹里的名字，排好序。
pub fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
