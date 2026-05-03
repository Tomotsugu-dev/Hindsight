use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMeta {
    pub device_id: String,
    pub display_name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub created_at: String,
}

fn default_color() -> String {
    "#60a5fa".into()
}

fn default_icon() -> String {
    "Monitor".into()
}

/// 启动级身份：在 bootstrap.json 同级目录存 device.json，与数据 DB 物理分离。
/// 把 DB 拷到另一台机器时不会带走 device_id —— device_id 必须随安装走，不随数据走。
fn device_file() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("Hindsight")
            .join("device.json"),
    )
}

static SELF_META: OnceLock<DeviceMeta> = OnceLock::new();

/// 启动时调用一次：读 device.json，没有就生成新的并落盘。之后用 self_meta() / self_id() 拿。
pub fn ensure_loaded() -> io::Result<&'static DeviceMeta> {
    if let Some(m) = SELF_META.get() {
        return Ok(m);
    }

    let path = device_file().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录")
    })?;

    let meta = match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<DeviceMeta>(&s) {
            Ok(m) if !m.device_id.trim().is_empty() => m,
            _ => {
                // 文件存在但内容损坏 —— 重生成，覆盖
                let m = generate_default();
                write_atomic(&path, &m)?;
                m
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let m = generate_default();
            write_atomic(&path, &m)?;
            m
        }
        Err(e) => return Err(e),
    };

    let _ = SELF_META.set(meta);
    Ok(SELF_META.get().expect("just set"))
}

fn generate_default() -> DeviceMeta {
    DeviceMeta {
        device_id: Uuid::new_v4().to_string(),
        display_name: "本机".into(),
        color: default_color(),
        icon: default_icon(),
        os: std::env::consts::OS.into(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn write_atomic(path: &PathBuf, meta: &DeviceMeta) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let s = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    fs::write(&tmp, s)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 获取当前设备的 UUID。必须在 `ensure_loaded()` 之后调用。
pub fn self_id() -> &'static str {
    &SELF_META
        .get()
        .expect("device::ensure_loaded() not called")
        .device_id
}

/// 获取当前设备完整 meta。
#[allow(dead_code)] // 公开 API，外部可调用
pub fn self_meta() -> &'static DeviceMeta {
    SELF_META
        .get()
        .expect("device::ensure_loaded() not called")
}

/// 用户改名 / 改颜色 / 改图标后写回 device.json。
pub fn update_self(name: Option<String>, color: Option<String>, icon: Option<String>) -> io::Result<DeviceMeta> {
    let current = SELF_META
        .get()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "device 尚未初始化"))?;

    let next = DeviceMeta {
        device_id: current.device_id.clone(),
        display_name: name.unwrap_or_else(|| current.display_name.clone()),
        color: color.unwrap_or_else(|| current.color.clone()),
        icon: icon.unwrap_or_else(|| current.icon.clone()),
        os: current.os.clone(),
        created_at: current.created_at.clone(),
    };

    let path = device_file().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录")
    })?;
    write_atomic(&path, &next)?;

    // OnceLock 不支持原地替换；这里只更新文件，进程内的 self_meta 直到下次冷启动才反映新值。
    // 但 devices 表里我们会同时同步写一行，UI 拿的是 devices 表，体验上不受影响。
    Ok(next)
}
