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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::TEST_SELF_ID;

    /// `HINDSIGHT_DATA_DIR` 是进程级全局；会改它的测试必须串行，否则
    /// `device_file()` 在并发测试间读到彼此的临时目录，产生顺序相关的随机红。
    /// 锁放在 test_util 共享——settings.rs 的损坏 JSON 测试也会改这个 env var，
    /// 模块各自设 Mutex 锁不住彼此。
    use crate::repo::test_util::lock_data_dir_env as lock_env;

    fn unique_tmp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hindsight-device-test-{tag}-{}", Uuid::new_v4()))
    }

    /// 改完 env var 必须恢复原值：泄漏到后续测试会把 device.json 引到不存在的临时目录。
    struct EnvGuard {
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(val: &str) -> Self {
            let prev = std::env::var("HINDSIGHT_DATA_DIR").ok();
            std::env::set_var("HINDSIGHT_DATA_DIR", val);
            Self { prev }
        }

        fn unset() -> Self {
            let prev = std::env::var("HINDSIGHT_DATA_DIR").ok();
            std::env::remove_var("HINDSIGHT_DATA_DIR");
            Self { prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HINDSIGHT_DATA_DIR", v),
                None => std::env::remove_var("HINDSIGHT_DATA_DIR"),
            }
        }
    }

    /// 升级版本新增字段（color/icon/os/created_at）后，老安装的 device.json 缺这些字段。
    /// 如果解析因此失败，ensure_loaded 会走"损坏重生成"分支换掉 device_id——用户的
    /// 云端同步身份直接断裂，Drive 上出现一台"新设备"。缺字段必须能解析且 id 原样保留。
    #[test]
    fn old_device_json_without_new_fields_still_parses() {
        let json =
            r#"{"device_id":"11111111-2222-3333-4444-555555555555","display_name":"旧安装"}"#;
        let m: DeviceMeta = serde_json::from_str(json).expect("老格式 device.json 必须能解析");
        assert_eq!(m.device_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(m.display_name, "旧安装");
        // 缺省出来的颜色/图标要直接可渲染（非空、颜色是 # 开头 hex），否则 UI 出空样式
        assert!(
            m.color.starts_with('#') && m.color.len() >= 4,
            "缺省 color 应是可用的 hex 颜色，实际: {:?}",
            m.color
        );
        assert!(!m.icon.is_empty(), "缺省 icon 不能为空");
        // os / created_at 是纯信息字段，缺省为空串即可，不应导致整体解析失败
        assert_eq!(m.os, "");
        assert_eq!(m.created_at, "");
    }

    /// 断电/半截写坏的 device.json 必须解析失败而不是产出脏 meta——ensure_loaded
    /// 正是靠这个 Err 才能触发重生成分支。缺 device_id 的 JSON 也必须失败：
    /// 该字段没有 serde default，一旦悄悄放行就会以空身份继续跑、污染云端文件名。
    #[test]
    fn corrupt_or_idless_device_json_fails_parse() {
        assert!(
            serde_json::from_str::<DeviceMeta>("{\"device_id\": \"abc").is_err(),
            "半截 JSON 必须报错"
        );
        assert!(
            serde_json::from_str::<DeviceMeta>(r#"{"display_name":"x"}"#).is_err(),
            "缺 device_id 必须报错，它没有 default"
        );
    }

    /// 两次安装/两台机器生成同一个 id，等价于云端串号：Drive 上同名文件互相覆盖。
    /// 所以新生成的 id 必须是合法 UUID（云端以它命名文件）且每次都不同。
    #[test]
    fn generate_default_yields_unique_valid_uuid() {
        let a = generate_default();
        let b = generate_default();
        Uuid::parse_str(&a.device_id).expect("device_id 必须是合法 UUID");
        Uuid::parse_str(&b.device_id).expect("device_id 必须是合法 UUID");
        assert_ne!(a.device_id, b.device_id, "两次生成必须得到不同身份");
        // created_at 参与展示与排序，必须是可解析的 RFC3339
        chrono::DateTime::parse_from_rfc3339(&a.created_at)
            .expect("created_at 必须是 RFC3339 时间戳");
        assert!(!a.display_name.is_empty(), "新设备必须有默认展示名");
        assert!(!a.os.is_empty(), "os 字段用于跨端图标区分，不能为空");
    }

    /// 首次启动时 config_dir/Hindsight 目录还不存在，write_atomic 必须自己建目录；
    /// 中间 tmp 文件残留会在人工排查时被误当正式文件。更新场景下目标文件已存在，
    /// rename 必须原地替换而不是报错（否则改名操作直接失败）。
    #[test]
    fn write_atomic_roundtrip_creates_parents_and_cleans_tmp() {
        let root = unique_tmp_dir("atomic");
        let dir = root.join("deep").join("nested");
        let path = dir.join("device.json");

        let m = generate_default();
        write_atomic(&path, &m).expect("父目录不存在时应自动创建");

        let parsed: DeviceMeta =
            serde_json::from_str(&fs::read_to_string(&path).expect("写完必须能读回"))
                .expect("落盘内容必须能解析回 DeviceMeta");
        assert_eq!(parsed.device_id, m.device_id);
        assert_eq!(parsed.display_name, m.display_name);
        assert_eq!(parsed.created_at, m.created_at);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "中间 tmp 文件必须被 rename 掉，不能残留"
        );

        // 覆盖写：device.json 已存在时必须替换成功且身份不变
        let mut m2 = m.clone();
        m2.display_name = "改名后".into();
        write_atomic(&path, &m2).expect("目标已存在时应原地替换");
        let parsed2: DeviceMeta =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed2.display_name, "改名后");
        assert_eq!(parsed2.device_id, m.device_id, "覆盖写不能动身份");

        let _ = fs::remove_dir_all(&root);
    }

    /// 双实例同机测试完全依赖 HINDSIGHT_DATA_DIR 把 device.json 隔离到各自数据目录；
    /// 覆盖失效则两实例共用一个 UUID，push 到 Drive 撞同名文件互相覆盖（等价于没 sync）。
    /// 空白 env 值（shell 里 export 了空串/空格）必须回落系统目录，而不是把文件写到
    /// 空路径或当前工作目录。
    #[test]
    fn device_file_honors_env_override_and_ignores_blank() {
        let _l = lock_env();

        // 正常覆盖：device.json 必须落在 env 指定目录下
        {
            let dir = unique_tmp_dir("envfile");
            let _g = EnvGuard::set(dir.to_str().unwrap());
            assert_eq!(device_file(), Some(dir.join("device.json")));
        }

        // 带首尾空格（复制命令时手滑）：应按 trim 后的路径处理
        {
            let dir = unique_tmp_dir("envtrim");
            let padded = format!("  {}  ", dir.display());
            let _g = EnvGuard::set(&padded);
            assert_eq!(device_file(), Some(dir.join("device.json")));
        }

        let expected_tail = Path::new("Hindsight").join("device.json");

        // 纯空白：等价于没设，回落系统配置目录
        {
            let _g = EnvGuard::set("   ");
            let p = device_file().expect("空白 env 应回落系统配置目录而不是 None");
            assert!(
                p.ends_with(&expected_tail),
                "回落路径应以 Hindsight/device.json 结尾，实际: {}",
                p.display()
            );
        }

        // 未设：同样回落系统配置目录
        {
            let _g = EnvGuard::unset();
            let p = device_file().expect("未设 env 时应使用系统配置目录");
            assert!(
                p.ends_with(&expected_tail),
                "回落路径应以 Hindsight/device.json 结尾，实际: {}",
                p.display()
            );
        }
    }

    /// 进程内所有测试共享一份 SELF_META；如果重复 init 会换 id，先跑的测试插的
    /// fixture 行（按 device_id 过滤）在后跑的测试里全部消失——表现为顺序相关的随机红。
    /// 所以第二次哪怕传入不同 id，也必须原样返回第一次的身份。
    #[test]
    fn init_for_tests_is_idempotent_and_pins_first_id() {
        let first = init_for_tests(TEST_SELF_ID);
        assert_eq!(first.device_id, TEST_SELF_ID);

        let second = init_for_tests("some-other-device-id");
        assert_eq!(
            second.device_id, TEST_SELF_ID,
            "重复 init 不能换 id，否则测试间 fixture 过滤全失效"
        );
        // 必须是同一份 meta（&'static 指向同一实例），不只是 id 碰巧相等
        assert!(std::ptr::eq(first, second));
    }

    /// self_id 是所有 repo 查询按设备过滤的锚点；init 之后拿不到、或和 self_meta
    /// 不一致，本机数据就会被当成远端设备的数据处理（读多写错都可能发生）。
    #[test]
    fn self_id_and_self_meta_consistent_after_init() {
        init_for_tests(TEST_SELF_ID);
        let id = self_id().expect("init 后 self_id 必须可用");
        assert_eq!(id, TEST_SELF_ID);
        let meta = self_meta().expect("init 后 self_meta 必须可用");
        assert_eq!(meta.device_id, id, "self_meta 与 self_id 必须指向同一身份");
    }

    /// 身份已加载后再调 ensure_loaded 必须是纯早退：既不能换 id（每次启动身份漂移
    /// = 云端历史全断），也不能落盘写新 device.json（写了说明错误走了生成分支）。
    #[test]
    fn ensure_loaded_is_noop_after_identity_set() {
        let _l = lock_env();
        init_for_tests(TEST_SELF_ID);

        let dir = unique_tmp_dir("ensure-noop");
        let _g = EnvGuard::set(dir.to_str().unwrap());

        let m = ensure_loaded().expect("已初始化时 ensure_loaded 不该失败");
        assert_eq!(m.device_id, TEST_SELF_ID, "早退分支不能返回新生成的身份");
        assert!(
            !dir.join("device.json").exists(),
            "早退分支不该往磁盘写任何东西"
        );
        // 再调一次仍是同一身份（幂等）
        assert_eq!(ensure_loaded().unwrap().device_id, TEST_SELF_ID);
    }

    /// 用户改名/改图标绝不能动身份字段：device_id 一变，云端会把本机当成一台新设备，
    /// 历史数据全部"属于别人"。None 的字段必须保留旧值（前端只传要改的字段）；
    /// 落盘的 device.json 是下次冷启动的身份来源，必须和返回值一致；而进程内
    /// SELF_META 按文档语义保持旧值直到冷启动。
    #[test]
    fn update_self_keeps_identity_and_persists_to_disk() {
        let _l = lock_env();
        init_for_tests(TEST_SELF_ID);

        let dir = unique_tmp_dir("update-self");
        let _g = EnvGuard::set(dir.to_str().unwrap());

        let before = self_meta().expect("init 后必有 meta").clone();
        let next = update_self(Some("改过的名字".into()), None, Some("Laptop".into()))
            .expect("已初始化时 update_self 应成功");

        // 身份三件套不许动
        assert_eq!(next.device_id, before.device_id);
        assert_eq!(next.os, before.os);
        assert_eq!(next.created_at, before.created_at);
        // Some 的字段生效，None 的字段保留旧值
        assert_eq!(next.display_name, "改过的名字");
        assert_eq!(next.icon, "Laptop");
        assert_eq!(next.color, before.color, "未传 color 必须保留旧值");

        // 落盘内容 = 下次冷启动读到的身份，必须与返回值一致
        let on_disk: DeviceMeta = serde_json::from_str(
            &fs::read_to_string(dir.join("device.json")).expect("update_self 必须写盘"),
        )
        .expect("落盘内容必须可解析");
        assert_eq!(on_disk.device_id, next.device_id);
        assert_eq!(on_disk.display_name, "改过的名字");
        assert_eq!(on_disk.icon, "Laptop");

        // 进程内 meta 保持旧值（文档行为：冷启动后才反映；UI 走 devices 表所以无感）
        assert_eq!(
            self_meta().unwrap().display_name,
            before.display_name,
            "OnceLock 不支持原地替换，进程内不应看到新名字"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// 所有 repo 测试的 fixture 行都拿 TEST_SELF_ID 当"本机"；fresh_test_pool 若不再
    /// 保证 self_id() == TEST_SELF_ID，那些按 device_id 过滤的测试会静默变成
    /// "过滤出 0 行也算过"。这里把该跨模块契约钉死。
    #[tokio::test]
    async fn fresh_test_pool_pins_self_id_to_test_constant() {
        let _pool = crate::repo::test_util::fresh_test_pool().await;
        assert_eq!(self_id().expect("fixture 后必可用"), TEST_SELF_ID);
    }
}
