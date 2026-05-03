use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct BootstrapFile {
    #[serde(default)]
    data_path: Option<String>,
}

/// 启动级配置文件位置：%APPDATA%/Hindsight/bootstrap.json （Windows）
/// 它存放的是"DB 应该开在哪里"这种 chicken-and-egg 的信息——在打开 DB 之前就要读到。
fn config_file() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("Hindsight")
            .join("bootstrap.json"),
    )
}

/// 系统默认数据目录：%APPDATA%/Hindsight 等
fn default_data_root() -> PathBuf {
    dirs::data_dir()
        .map(|p| p.join("Hindsight"))
        .unwrap_or_else(|| PathBuf::from("hindsight-data"))
}

/// 当前生效的数据根：优先 bootstrap 里的自定义值，没有则系统默认。
pub fn data_root() -> PathBuf {
    if let Some(path) = read_custom_data_path() {
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    default_data_root()
}

fn read_custom_data_path() -> Option<PathBuf> {
    let cfg = config_file()?;
    let s = fs::read_to_string(&cfg).ok()?;
    let b: BootstrapFile = serde_json::from_str(&s).ok()?;
    b.data_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
}

/// 写入新的数据根（不会自动迁移已有数据，下次启动生效）。
pub fn set_data_root(path: &str) -> io::Result<()> {
    let cfg = config_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录"))?;
    if let Some(parent) = cfg.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = BootstrapFile {
        data_path: Some(path.to_string()),
    };
    let s = serde_json::to_string_pretty(&body)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    fs::write(&cfg, s)
}
