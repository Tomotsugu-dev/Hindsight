//! `llama-server` 启动命令行参数拼装。

use std::path::Path;

use tokio::process::Command;

use crate::ai::platform::{self, Platform};

/// `llama-server` 默认单槽上下文窗口大小（8K tokens）。
/// 兼顾日报/对话较长输入与本机显存/内存占用。
pub const DEFAULT_CTX_SIZE: u32 = 8192;

/// 启动 `llama-server` 时的可选性能与上下文覆盖参数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartOverrides {
    /// 同时设置 `--batch-size` 与 `--ubatch-size`；`None` 时沿用 `llama.cpp` 默认值。
    /// 调大可加快 prompt 评估速度，但会提高显存峰值。
    pub batch_size: Option<u32>,
    /// `-np` 并行槽位数；`None` 时固定为 1（单槽），需配合 `summary_runner` 并发请求生效。
    pub parallel_slots: Option<u32>,
    /// **单个槽位**的上下文上限；`None` 时采用 [`DEFAULT_CTX_SIZE`]（8K）。
    /// 传给 `llama-server` 的 `--ctx-size` 为 `ctx_size × parallel_slots`，
    /// 以确保每个槽位都能拿到完整预算（KV cache 启动时按总量一次性分配）。
    pub ctx_size: Option<u32>,
}

/// 按当前平台对应的后端能力决定 `-ngl`（GPU offload 层数）；CPU 构建不传该参数。
fn gpu_layers_for(platform: Platform) -> Option<u32> {
    match platform {
        Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13 | Platform::MacOSArm64Metal => {
            Some(99)
        }
        Platform::WindowsX64Cpu | Platform::MacOSX64 | Platform::LinuxX64Cpu => None,
    }
}

/// 组装在本地 `port` 上加载 `model` 的 `llama-server` 启动命令。
pub(super) fn build_command(
    binary_path: &Path,
    port: u16,
    model: &Path,
    overrides: &EngineStartOverrides,
) -> Command {
    let mut cmd = Command::new(binary_path);
    cmd.arg("--host").arg("127.0.0.1");
    cmd.arg("--port").arg(port.to_string());
    cmd.arg("-m").arg(model);
    if let Some(layers) = gpu_layers_for(platform::detect()) {
        cmd.arg("-ngl").arg(layers.to_string());
    }
    // 同时设置 logical / physical batch 保持一致，避免默认 batch=2048、ubatch=512 拉大显存波动。
    if let Some(batch_size) = overrides.batch_size {
        let batch_size = batch_size.max(32); // llama-server 不接受过小的 batch，32 为安全下限
        cmd.arg("--batch-size").arg(batch_size.to_string());
        cmd.arg("--ubatch-size").arg(batch_size.to_string());
    }
    // llama-server 会将 `--ctx-size` 均分给 `-np` 个槽位，因此必须传入 `单槽预算 × 槽位数`，
    // 否则多槽模式下每个槽位的有效上下文会被缩小导致长 prompt 报错 400。
    let parallel_slots = overrides.parallel_slots.unwrap_or(1).max(1);
    let per_slot_ctx = overrides.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    cmd.arg("--ctx-size")
        .arg(per_slot_ctx.saturating_mul(parallel_slots).to_string());
    // 即使为单槽也显式传入 `-np 1`，防止 llama-server 自动回退到 4 槽浪费 KV cache。
    cmd.arg("-np").arg(parallel_slots.to_string());
    // 关闭新版默认开启的 prompt cache（上限 8 GB），日报各段输入差异大、命中率极低。
    cmd.arg("--cache-ram").arg("0");
    // 关闭 context checkpoints，避免多槽长会话累积 GB 级内存并在生成途中导致服务崩溃。
    cmd.arg("--ctx-checkpoints").arg("0");
    // 接管子进程 stdout/stderr 供失败诊断回放，避免污染主进程终端。
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());

    // Windows 下隐藏控制台黑框（tokio::process::Command 自带 creation_flags）。
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_args(overrides: &EngineStartOverrides) -> Vec<String> {
        build_command(
            Path::new("llama-server"),
            4242,
            Path::new("m.gguf"),
            overrides,
        )
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
    }

    fn flag_value(args: &[String], flag: &str) -> Option<String> {
        let idx = args.iter().position(|arg| arg == flag)?;
        args.get(idx + 1).cloned()
    }

    // 传给 llama-server 的 --ctx-size 应等于「单槽上下文 × 槽位数」，默认 8K、1 槽。
    #[test]
    fn ctx_size_is_per_slot_times_slots() {
        let args = command_args(&EngineStartOverrides::default());
        assert_eq!(flag_value(&args, "--ctx-size").as_deref(), Some("8192"));
        assert_eq!(flag_value(&args, "-np").as_deref(), Some("1"));

        let args = command_args(&EngineStartOverrides {
            ctx_size: Some(16384),
            parallel_slots: Some(2),
            batch_size: None,
        });
        assert_eq!(flag_value(&args, "--ctx-size").as_deref(), Some("32768"));
        assert_eq!(flag_value(&args, "-np").as_deref(), Some("2"));
    }

    // 批大小同时写入 --batch-size 和 --ubatch-size，低于 32 时抬升到下限 32。
    #[test]
    fn batch_size_sets_both_and_has_a_floor() {
        let args = command_args(&EngineStartOverrides {
            batch_size: Some(8),
            ..Default::default()
        });
        assert_eq!(flag_value(&args, "--batch-size").as_deref(), Some("32"));
        assert_eq!(flag_value(&args, "--ubatch-size").as_deref(), Some("32"));
        assert_eq!(flag_value(&args, "-m").as_deref(), Some("m.gguf"));
    }
}
