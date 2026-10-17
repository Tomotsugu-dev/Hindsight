//! 插件测试共用：临时文件夹、造 zip。

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

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

pub fn manifest_json(version: &str) -> String {
    serde_json::json!({
        "id": "hindsight-focus",
        "name": "Focus",
        "version": version,
        "permissions": ["stats.read", "focus.changed", "focus.category"],
        "ui_location": "none",
        "runs_in_background": true
    })
    .to_string()
}

pub fn write_zip(path: &Path, files: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    for (name, body) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
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

/// 文件夹里的名字，排好序。
pub fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
