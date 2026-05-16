use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::utc_now_rfc3339;

/// 当前设备身份（device.json 里持久化的字段）。安装级别身份，不随数据走：
/// 把 DB 拷到另一台机器时不会带走 device_id。
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

/// 启动级身份：默认走系统 config_dir（`~/Library/Application Support/Hindsight/`），
/// 与数据 DB 物理分离 —— 把 DB 拷到另一台机器时不会带走 device_id。
///
/// 测试场景例外：`HINDSIGHT_DATA_DIR` 被设时，device.json 跟数据走，确保
/// [`docs/internal/local-multi-device-test.md`] 的双进程同机测试里两个实例
/// 各自有独立的 device_id（否则它们共享系统 config_dir 的 device.json 共用同一个
/// UUID，push 到 Drive 上撞同名文件、互相覆盖，等价于完全没 sync）。生产路径
/// 不会设这个 env var，行为完全不变。
fn device_file() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("HINDSIGHT_DATA_DIR") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("device.json"));
        }
    }
    Some(dirs::config_dir()?.join("Hindsight").join("device.json"))
}

static SELF_META: OnceLock<DeviceMeta> = OnceLock::new();

/// 启动时调用一次：读 device.json，没有就生成新的并落盘。之后用 self_meta() / self_id() 拿。
pub fn ensure_loaded() -> io::Result<&'static DeviceMeta> {
    if let Some(m) = SELF_META.get() {
        return Ok(m);
    }

    let path = device_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录"))?;

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
    // OnceLock 刚 set 完立刻 get：上一行 `set` 即使被并发线程抢走，本线程
    // get 拿到的也是已写入的值；invariant 在 OnceLock 类型上由 std 保证
    Ok(SELF_META.get().expect("OnceLock 刚 set，必有值"))
}

fn generate_default() -> DeviceMeta {
    DeviceMeta {
        device_id: Uuid::new_v4().to_string(),
        display_name: "本机".into(),
        color: default_color(),
        icon: default_icon(),
        os: crate::platform::local_os_id().into(),
        created_at: utc_now_rfc3339(),
    }
}

fn write_atomic(path: &Path, meta: &DeviceMeta) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let s = serde_json::to_string_pretty(meta).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(&tmp, s)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 获取当前设备的 UUID。
///
/// 返回 `Err` 当且仅当 [`ensure_loaded`] 未被调用（理论上 `lib.rs::run` 启动期就调过，
/// 所以运行期不该看到这条错误）；改 Result 后任何漏掉 ensure 的代码路径不再 panic。
pub fn self_id() -> crate::error::Result<&'static str> {
    SELF_META
        .get()
        .map(|m| m.device_id.as_str())
        .ok_or_else(|| crate::error::Error::Other("device::ensure_loaded() 未调用".to_string()))
}

/// 获取当前设备完整 meta。同 [`self_id`]：未初始化时返回 `Err`。
#[allow(dead_code)] // 公开 API，外部可调用
pub fn self_meta() -> crate::error::Result<&'static DeviceMeta> {
    SELF_META
        .get()
        .ok_or_else(|| crate::error::Error::Other("device::ensure_loaded() 未调用".to_string()))
}

/// 单元测试入口：把 `SELF_META` 设成一个固定 device_id 让 [`self_id`] 能返回值。
///
/// `OnceLock` 是 set-once：进程内所有 `cargo test` 共享一份 `SELF_META`，所以约定
/// 全部测试用同一个 id（"test-self-device"）。第一个 test 调用 init 时真正写入，
/// 之后的 test 调用 `get_or_init` 返回已存的值——不会 panic 也不会换值，
/// 但测试的 fixture row 也必须用这个固定 id 才能配合 device_id 过滤逻辑。
#[cfg(test)]
pub(crate) fn init_for_tests(id: &str) -> &'static DeviceMeta {
    SELF_META.get_or_init(|| DeviceMeta {
        device_id: id.to_string(),
        display_name: format!("test-{id}"),
        color: default_color(),
        icon: default_icon(),
        os: "test".into(),
        created_at: utc_now_rfc3339(),
    })
}

/// 用户改名 / 改颜色 / 改图标后写回 device.json。
pub fn update_self(
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
) -> io::Result<DeviceMeta> {
    let current = SELF_META
        .get()
        .ok_or_else(|| io::Error::other("device 尚未初始化"))?;

    let next = DeviceMeta {
        device_id: current.device_id.clone(),
        display_name: name.unwrap_or_else(|| current.display_name.clone()),
        color: color.unwrap_or_else(|| current.color.clone()),
        icon: icon.unwrap_or_else(|| current.icon.clone()),
        os: current.os.clone(),
        created_at: current.created_at.clone(),
    };

    let path = device_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到系统配置目录"))?;
    write_atomic(&path, &next)?;

    // OnceLock 不支持原地替换；这里只更新文件，进程内的 self_meta 直到下次冷启动才反映新值。
    // 但 devices 表里我们会同时同步写一行，UI 拿的是 devices 表，体验上不受影响。
    Ok(next)
}
