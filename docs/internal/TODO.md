 # TODO List

## 2026-05-05

- [x] 测试微软的 MSI 包是否能正确安装和卸载（包括选择安装路径，卸载时选择是否删除用户数据）
- [x] 虚拟机中测试

## 2026-05-06

### llama-server 孤儿子进程防护：跨平台收口

Windows 已经用 Job Object（`KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`）
根治。Linux / macOS 还有缝隙，等需要时补。

- [ ] **Windows**：补「`AssignProcessToJobObject` 失败时立刻 child.kill()」兜底
  （[`ai/server.rs:start_with_overrides`](../../src-tauri/src/ai/server.rs)
  现在只 log warn，理论上 silent leak）
- [ ] **Windows**：spawn 走 `CREATE_SUSPENDED` → assign → ResumeThread，
  彻底关掉 spawn 与 assign 之间~毫秒级的 race window
  （需绕过 `tokio::process::Command`，改用 `std::process::Command` +
  `CommandExt::creation_flags` + 手动 `ResumeThread`）
- [ ] **Linux**：在 child fork 后调 `prctl(PR_SET_PDEATHSIG, SIGKILL)`
  （`std::os::unix::process::CommandExt::pre_exec` 闭包里调 libc::prctl，
  父死即收 SIGKILL；唯一一行根治方案）
- [ ] **macOS**：当前仅靠 `RunEvent::Exit` 钩子覆盖 ~99% 路径
  （Cmd+Q / 红绿灯 / 终端 Ctrl+C 都进 hook）。
  Force Quit / SIGKILL / SIGSEGV / SIGABRT 漏。彻底根治需要 watchdog
  子进程轮询 `getppid()` 是否变 1，代价高。
  - 当前评估：macOS 用户少 + Apple Silicon unified memory（孤儿吃的是 RAM，
    重登注销即回收，不像 Windows 那样需要手动 taskkill），暂不做
  - 文档里给「孤儿处理一句话指引」：`pkill llama-server`
