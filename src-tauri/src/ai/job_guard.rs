//! 防孤儿子进程：用 Windows Job Object 把所有 llama-server child 绑到 Hindsight 父进程上。
//!
//! Tauri 自带的 `RunEvent::Exit` 钩子只覆盖正常退出路径——`Ctrl+C` 杀 dev、panic、
//! taskkill 外部杀的时候 hook 完全不触发，child 就被遗弃，VRAM 一直占着。
//!
//! Job Object 是 OS 内核级别的「进程组」概念：父进程 spawn 出的 child 调
//! `AssignProcessToJobObject` 加入 Job → Job 设了 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//! → 父进程死时 OS 自动 close Job 的最后一个 HANDLE（父进程持有的）→ Job 关闭
//! → 内核同步杀光所有 Job 成员（child 们）。无视父进程怎么死。
//!
//! 用法：
//! 1. 启动时调一次 [`init_global_job`]（lib.rs setup）
//! 2. 每次 spawn child 后立刻调 [`assign_child_pid`]（server.rs）
//!
//! Linux：可以用 `prctl(PR_SET_PDEATHSIG, SIGKILL)` 在 child fork 后；本文件留 no-op。
//! macOS：无原生等价物，依赖 RunEvent::Exit 钩子；本文件留 no-op。

use crate::error::{Error, Result};

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

            // 不能 set——OnceLock 不允许，但这里是首次进所以一定没值
            JOB.set(JobHandle(job))
                .map_err(|_| "Job 已被并发初始化".to_string())?;
            Ok(())
        }
    }

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

#[cfg(target_os = "windows")]
pub fn init_global_job() -> Result<()> {
    win::init().map_err(Error::Other)
}

#[cfg(target_os = "windows")]
pub fn assign_child_pid(pid: u32) -> Result<()> {
    win::assign_pid(pid).map_err(Error::Other)
}

#[cfg(not(target_os = "windows"))]
pub fn init_global_job() -> Result<()> {
    // Linux / macOS：留待后续；当前依赖 RunEvent::Exit 钩子在正常退出时收尸
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn assign_child_pid(_pid: u32) -> Result<()> {
    Ok(())
}
