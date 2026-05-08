//! 防孤儿子进程：把 llama-server child 绑到 Hindsight 父进程的生命周期上。
//!
//! - **Windows**：Job Object（[`win`] 子模块）—— 父进程持有 Job HANDLE，进程死
//!   （panic / Ctrl+C / taskkill）→ OS 内核 close 最后一个 HANDLE → Job 关闭
//!   → 内核同步杀光所有 Job 成员。覆盖所有死法。
//! - **Linux**：[`prepare_command`] 在 child fork 后 / exec 前调
//!   - `setpgid(0, 0)`：让 child 自己当 process group leader（独立 pgid）
//!   - `prctl(PR_SET_PDEATHSIG, SIGKILL)`：父进程死 → 内核给本进程 SIGKILL。覆盖所有死法。
//! - **macOS**：[`prepare_command`] 仅 `setpgid(0, 0)`。无 PR_SET_PDEATHSIG 等价物，
//!   只能让 graceful exit（`RunEvent::Exit` 钩子调 `supervisor.stop()`）能 kill 干净；
//!   Hindsight 被 SIGKILL 时 child 仍被遗留 → [`init_state`] 返回 [`JobInitState::Degraded`]。
//!
//! 用法（已嵌入 server.rs）：
//! 1. 启动时调一次 [`init_global_job`]（在 `lib.rs::run` 里）
//! 2. spawn llama-server 之前调 [`prepare_command(&mut cmd)`]（Linux/macOS 装 pre_exec 钩子）
//! 3. spawn 之后调 [`assign_child_pid(pid)`]（Windows 把 child 加进 Job）
//!
//! [`init_state`] 返回当前保护状态，给前端 `engine_status` 命令做"保护降级"提示。

use crate::error::Result;

/// 子进程保护当前状态。`engine_status` 命令通过 [`init_state`] 拿到，
/// 让前端能告诉用户"保护降级"或"保护正常"。
#[derive(Debug, Clone)]
pub enum JobInitState {
    /// 还没调 [`init_global_job`]
    NotInitialized,
    /// 保护正常工作。仅 Windows / Linux 路径会构造此变体；macOS 永远走
    /// `Degraded` 分支（缺 PR_SET_PDEATHSIG 等价物），所以编译到 macOS 时
    /// 此变体在 lib 里"never constructed"——但 [`commands::ai_engine`] 的
    /// 全平台 match 仍要枚举它，因此不能 cfg-gate 掉，只能 allow。
    #[allow(dead_code)]
    Ok,
    /// 保护降级；附原因（macOS 缺 prctl 时是预期降级；Windows Job 创建失败是异常降级）
    Degraded(String),
}

/// 全局保护状态。`OnceLock` 但用 `set_or_replace` 模式：每次状态变化都覆盖。
/// 因为只有 supervisor 启动 / spawn 路径会写，竞争极少。
static JOB_STATE: std::sync::Mutex<Option<JobInitState>> = std::sync::Mutex::new(None);

fn store_state(s: JobInitState) {
    if let Ok(mut g) = JOB_STATE.lock() {
        *g = Some(s);
    }
}

/// 读当前保护状态。从未初始化时返回 [`JobInitState::NotInitialized`]。
pub fn init_state() -> JobInitState {
    JOB_STATE
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or(JobInitState::NotInitialized)
}

// ───────────────────────────── Windows: Job Object ─────────────────────────────

#[cfg(target_os = "windows")]
mod win {
    use std::ptr::null_mut;
    use std::sync::OnceLock;

    use winapi::ctypes::c_void;
    use winapi::shared::minwindef::FALSE;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::jobapi2::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    };
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::{
        JobObjectExtendedLimitInformation, HANDLE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// 全局 Job 句柄。`HANDLE` 是 `*mut c_void`，自带不实现 Send；包一层声明 unsafe Send。
    /// HANDLE 本身在内核里是引用计数线程安全的，跨线程读取没问题。
    struct JobHandle(HANDLE);
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    /// 进程生命周期内只持有一份 Job HANDLE；进程死 → OS 自动 close → Job 关闭 → child 全死
    static JOB: OnceLock<JobHandle> = OnceLock::new();

    /// 创建 Job Object 并设 KILL_ON_JOB_CLOSE。幂等：已初始化时立刻返回 Ok。
    pub fn init() -> Result<(), String> {
        if JOB.get().is_some() {
            return Ok(());
        }
        unsafe {
            // 匿名 Job——只有我们这一个进程持有 HANDLE
            let job = CreateJobObjectW(null_mut(), null_mut());
            if job.is_null() {
                let err = std::io::Error::last_os_error();
                return Err(format!("CreateJobObjectW: {err}"));
            }

            // KILL_ON_JOB_CLOSE：Job 的最后一个 HANDLE 关闭时，所有成员进程被内核杀
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(format!("SetInformationJobObject: {err}"));
            }

            // 注意：故意不把 Hindsight 自己加进 Job——仅靠 HANDLE 生命周期触发 close。
            // 加进去会让本进程也成为 Job 成员，行为依然正确，但 cargo tauri dev
            // 调试期间 panic 重启等场景多一重耦合，没必要。

            JOB.set(JobHandle(job))
                .map_err(|_| "Job 已被并发初始化".to_string())?;
            Ok(())
        }
    }

    /// 把指定 pid 加入全局 Job。Job 未初始化时返回 Err。
    pub fn assign_pid(pid: u32) -> Result<(), String> {
        let job = match JOB.get() {
            Some(j) => j.0,
            None => return Err("Job 未初始化（先调 init_global_job）".to_string()),
        };
        unsafe {
            // PROCESS_SET_QUOTA + PROCESS_TERMINATE 是 AssignProcessToJobObject 的最低权限
            let h = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
            if h.is_null() {
                let err = std::io::Error::last_os_error();
                return Err(format!("OpenProcess pid={pid}: {err}"));
            }
            let ok = AssignProcessToJobObject(job, h);
            // 不管成功失败都要 close 我们这边的 HANDLE（child 进程的引用由 Job 维持）
            let last_err = if ok == 0 {
                Some(std::io::Error::last_os_error())
            } else {
                None
            };
            CloseHandle(h);
            if let Some(err) = last_err {
                return Err(format!("AssignProcessToJobObject pid={pid}: {err}"));
            }
            Ok(())
        }
    }
}

/// 启动期调一次：初始化全局子进程保护。失败时把状态写为 `Degraded(reason)`
/// 让前端 `engine_status` 命令能给用户提示。
#[cfg(target_os = "windows")]
pub fn init_global_job() -> Result<()> {
    match win::init() {
        Ok(()) => {
            store_state(JobInitState::Ok);
            Ok(())
        }
        Err(e) => {
            store_state(JobInitState::Degraded(format!(
                "Windows Job 初始化失败：{e}（Hindsight 异常退出可能遗留 llama-server 子进程）"
            )));
            // 用全路径 `crate::error::Error::Other` 而非 `use Error;`：
            // Error 仅在 Windows 路径用，写 use 会让 macOS / Linux 编译报 unused_import
            Err(crate::error::Error::Other(e))
        }
    }
}

/// 在 spawn child 之前调用，给 Command 装 pre_exec 钩子（unix）或 no-op（windows）。
/// Windows 走 post-spawn 的 [`assign_child_pid`] 路径。
#[cfg(target_os = "windows")]
pub fn prepare_command(_cmd: &mut tokio::process::Command) -> Result<()> {
    // Windows 走 post-spawn AssignProcessToJobObject 路径，spawn 前不需要装钩子
    Ok(())
}

/// 在 spawn 之后立刻调用，把 child pid 加入全局 Job（windows）或 no-op（unix，pre_exec 已处理）。
/// 失败时把状态切到 Degraded 让前端能感知。
#[cfg(target_os = "windows")]
pub fn assign_child_pid(pid: u32) -> Result<()> {
    win::assign_pid(pid).map_err(|e| {
        // 单进程级别失败：Job 本身可能仍正常工作，但本 child 漏接管。
        // 让前端拿到 Degraded 状态去提示——比静默更安全
        store_state(JobInitState::Degraded(format!(
            "AssignProcessToJobObject pid={pid} 失败：{e}（该子进程未被 Job 接管）"
        )));
        crate::error::Error::Other(e)
    })
}

// ───────────────────────────── Linux: setpgid + PDEATHSIG ─────────────────────────────

/// Linux 实现：pre_exec 时装 setpgid + PDEATHSIG，无需全局 init syscall。
#[cfg(target_os = "linux")]
pub fn init_global_job() -> Result<()> {
    // pre_exec 在 spawn 时装钩子，全局 init 阶段无需做 syscall；状态写 Ok
    store_state(JobInitState::Ok);
    Ok(())
}

/// Linux 实现：装 pre_exec 钩子 setpgid + PR_SET_PDEATHSIG=SIGKILL。
/// 父进程死时内核同步杀子；graceful exit 时 supervisor 也能 killpg 收尸。
#[cfg(target_os = "linux")]
pub fn prepare_command(cmd: &mut tokio::process::Command) -> Result<()> {
    // SAFETY: pre_exec 闭包在 fork 后 exec 前的 child 里跑，必须 async-signal-safe；
    // setpgid 与 prctl 都是 async-signal-safe 的 syscall（POSIX 与 Linux 文档均明确）。
    // 注：tokio::process::Command 的 `pre_exec` 是 inherent method，无需 `use std::os::unix::process::CommandExt`。
    unsafe {
        cmd.pre_exec(|| {
            // 让 child 成为自己 pgid 的 leader——Hindsight 在 graceful exit 时
            // 即使丢失 child handle 也能用 killpg(child_pid, SIGKILL) 收尸
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // 父进程死 → 内核给本进程 SIGKILL。无视父进程死法（panic / SIGKILL / SEGV）
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

/// Linux 实现：no-op，pre_exec 已搞定保护。
#[cfg(target_os = "linux")]
pub fn assign_child_pid(_pid: u32) -> Result<()> {
    Ok(())
}

// ───────────────────────────── macOS: 仅 setpgid（无 PR_SET_PDEATHSIG 等价） ─────────────────────────────

/// macOS 实现：缺 prctl 等价物，预期降级；初始化即把状态写为 Degraded 让 UI 提示。
#[cfg(target_os = "macos")]
pub fn init_global_job() -> Result<()> {
    // macOS 缺乏 Linux 的 PR_SET_PDEATHSIG / Windows 的 Job Object 等价物：
    // Hindsight 被 SIGKILL（强杀 / panic 不调 Drop）时 llama-server 子进程会被遗留。
    // 状态明确标 Degraded 让前端给用户提示。
    store_state(JobInitState::Degraded(
        "macOS 不支持 PR_SET_PDEATHSIG，Hindsight 被强制杀死时 llama-server 子进程会被遗留；\
         正常退出（关窗 / Cmd+Q）路径上由 RunEvent::Exit 钩子收尸"
            .to_string(),
    ));
    Ok(())
}

/// macOS 实现：仅 setpgid，让 supervisor.stop() 能 killpg 收尸。
#[cfg(target_os = "macos")]
pub fn prepare_command(cmd: &mut tokio::process::Command) -> Result<()> {
    // SAFETY: setpgid 是 async-signal-safe 的 syscall。
    // 注：tokio::process::Command 的 `pre_exec` 是 inherent method，无需 `use std::os::unix::process::CommandExt`。
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

/// macOS 实现：no-op。
#[cfg(target_os = "macos")]
pub fn assign_child_pid(_pid: u32) -> Result<()> {
    Ok(())
}

// ───────────────────────────── 其它 unix（FreeBSD 等） ─────────────────────────────

/// 其它 unix（FreeBSD / OpenBSD 等）：未实现，状态写 Degraded。
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn init_global_job() -> Result<()> {
    store_state(JobInitState::Degraded(
        "当前平台未实现子进程保护；Hindsight 异常退出可能遗留 llama-server 子进程".to_string(),
    ));
    Ok(())
}

/// 其它 unix：no-op。
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn prepare_command(_cmd: &mut tokio::process::Command) -> Result<()> {
    Ok(())
}

/// 其它 unix：no-op。
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn assign_child_pid(_pid: u32) -> Result<()> {
    Ok(())
}
