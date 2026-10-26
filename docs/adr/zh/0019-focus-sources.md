# ADR-0019 · 焦点窗口通过统一接口读取：轮询为主，留出叫醒口子

- **日期**：2026-10-26
- **状态**：**已接受**

采集服务原先通过 `capture/window.rs` 读焦点：一个函数里用 `#[cfg]` 分平台，macOS
专用的标题补取还漏到了 `service.rs`。现在所有系统都放到 `FocusSource` 接口后面，
一个系统一个文件；继续轮询，另加一个只能"叫醒"循环的 `watch` 口子。

## 流程

```mermaid
sequenceDiagram
    participant Loop as tick 循环（service.rs）
    participant Source as FocusSource
    Note over Loop,Source: 现在：所有来源
    loop 每 5 秒
        Loop->>Source: focused_window()
        Source-->>Loop: WindowInfo
        opt 标题为空
            Loop->>Source: recover_title(pid)（spawn_blocking）
        end
    end
    Note over Loop,Source: 以后：能推送的来源
    Source-)Loop: 焦点变化时 waker.wake()
    Loop->>Source: 立刻 focused_window()
```

## 背景

- Windows、macOS 编译时选一种实现就够了；Linux 必须运行时选（同一个安装包可能装在
  GNOME、KDE、Hyprland 或 X11 会话上），旧结构里没地方放这层判断。
- tick 不只是读焦点：挂机、睡眠间隔、工作时段、隐私、截图、会话切分都在里面，每个
  tick 需要一份一致的快照。
- 轮询最多晚 5 秒发现切换，停留不到 5 秒的窗口会漏掉。事件能解决这点，但事件会丢
  （扩展被禁用、睡眠、崩溃）、会成串到来（Alt-Tab、每秒都在变的标题），有些会话根本没有事件。

## 决定

我们**只通过 `FocusSource` 读焦点，一个系统一个文件，继续轮询，`watch` 只用来叫醒**。

| 路径 | 内容 |
| --- | --- |
| `capture/focus/mod.rs` | `WindowInfo`、`FocusSource`、`FocusWaker`、`WatchGuard`、和平台无关的小工具（`basename`、`resolve_exe_path`） |
| `windows.rs` / `macos.rs` / `linux/` | 每个系统一个来源；Linux 是文件夹，因为它的来源取决于桌面（目前只有 `linux/x11.rs`） |
| `pick.rs` | macOS 的主窗口挑选，不依赖平台，所以在所有平台上测试 |

各系统即使都调用 xcap，也各写一份，不共用：xcap 在每个系统底下的机制不同，以后也会各自改动（比如 Windows 改用 `GetForegroundWindow`，Linux 的 X11 路径在 Wayland 下不用）。共用文件会重新长出 `#[cfg]`。

不变式：

- 事件只负责叫醒，不带数据；数据永远从 `focused_window` 拉。
- 轮询永远保留：它纠正丢掉的事件，也驱动和时间有关的检查。
- tick 里的逻辑只看墙钟时间，不数 tick 次数：有了叫醒，tick 间隔不再固定。
- tick 运行期间的多次叫醒合并成一次（tokio `Notify` 只保留一个待处理标记）。
- 监听守卫由 tick 任务持有，停止采集即停止监听。

## 备选方案

- **统一接口 + 轮询 + 叫醒口子（选定）**。
- **来源直接推送焦点数据**：tick 会有两条入口，挂机、隐私、截图逻辑都要处理两遍，事件和 tick 之间还有竞争。
- **`watch` 返回事件通道**：每个事件都排队，要自己写合并逻辑和通道满的处理，而事件本身不带任何数据。
- **顶层按系统分文件夹（`src/linux/…`）**：一个功能散在多个文件夹；项目已经按功能分（`browser_url/`、`icons/`）。

## 影响

- **好处**：新增一个桌面 = 新增一个文件，再在它所在系统的 `detect` 里加一行；`service.rs` 里不再有焦点相关的 `#[cfg]`；可以用假的来源测采集服务。
- **代价**：Windows 和 Linux 上标题为空时，多一次返回 None 的 `spawn_blocking`。

## 数据、兼容性、安全与隐私

无影响：行为不变，不改存储的数据。

## 验证

- Linux：`cargo clippy --all-targets -- -D warnings`（只有 `main` 上已有的 lint）、`cargo test --lib`。
- `FocusWaker` 和 `WatchGuard` 的单测：叫醒不丢、多次合并成一次、没有叫醒就继续等、守卫丢弃时只停一次。
- Windows、macOS：CI 的 clippy 和测试；Mac 真机上手动跑被 ignore 的 `sck_recover_title_smoke`。

## 后续工作

- GNOME Wayland：用 GNOME Shell 扩展经 D-Bus 提供焦点的来源（另写 ADR）；用它的 `FocusChanged` 信号实现 `watch`，然后去掉 `FocusWaker::wake` 和 `WatchGuard::new` 上的 `allow(dead_code)`。
- 第一个来源实现 `watch` 之前：加去抖和两次 tick 的最小间隔，限制快速切换时的连拍截图，定一个最短会话时长。
- Linux 需要挂机检测时，用同样的结构拆 `platform.rs`。
