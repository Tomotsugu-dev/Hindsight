//! llama-server 的命令行参数。

use std::path::Path;

use tokio::process::Command;

use crate::ai::platform::{self, Platform};

/// llama-server 上下文窗口大小。
///
/// Vision 模型每张图被分成多个 patch，token 数比纯文本对话大很多：
/// 768 px max_dim 下一张图 ~256 tokens × 12 张 = ~3000 tokens，加 system / user
/// prompt 和输出空间 (max_tokens=768)，4K 容易超。8K 给足余量；GPU 上 KV cache
/// 多用一倍 RAM 不痛——5090/Apple Silicon 都装得下，CPU fallback 也可接受。
pub const DEFAULT_CTX_SIZE: u32 = 8192;

/// 启动 llama-server 时可选的命令行调参。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartOverrides {
    /// 同时设 `--batch-size N --ubatch-size N`。`None` = 用 llama.cpp 默认（512）。
    /// 加大能提升 prompt eval 速度，代价是 KV cache 高水位 ↑（5090 32GB 装得下 4096）。
    pub batch_size: Option<u32>,
    /// `-np N`：llama-server 同时处理的并行槽位数；`None` = 1（单槽）。
    /// 配合 `summary.rs` 的 `buffer_unordered` 才能真正并发——只设 -np 不并发调用没用。
    pub parallel_slots: Option<u32>,
    /// **每个 slot** 的 ctx 上限。`None` = [`DEFAULT_CTX_SIZE`]（8K）。
    /// 实际传给 llama-server 的 `--ctx-size` 是 `ctx_size × parallel_slots`，
    /// 让每个 slot 都拿到 user 选的 budget；llama.cpp 启动时按这个总量
    /// 一次性 mmap KV cache（VRAM / RAM 直接吃掉，不是按需增长）。
    pub ctx_size: Option<u32>,
}

/// 根据 platform 决定 `-ngl`（GPU layer 数）：CUDA / Metal binary 全 offload，
/// CPU binary 不传。
fn gpu_layers_for(p: Platform) -> Option<u32> {
    match p {
        Platform::WindowsX64Cuda12 | Platform::WindowsX64Cuda13 => Some(99),
        Platform::MacOSArm64Metal => Some(99),
        // CPU binary 不支持 -ngl
        Platform::WindowsX64Cpu | Platform::MacOSX64 | Platform::LinuxX64Cpu => None,
    }
}

/// 拼出在 `port` 上加载 `model` 的 llama-server 命令。
pub(super) fn build_command(
    bin: &Path,
    port: u16,
    model: &Path,
    overrides: &EngineStartOverrides,
) -> Command {
    let mut cmd = Command::new(bin);
    cmd.arg("--host").arg("127.0.0.1");
    cmd.arg("--port").arg(port.to_string());
    cmd.arg("-m").arg(model);
    if let Some(n) = gpu_layers_for(platform::detect()) {
        cmd.arg("-ngl").arg(n.to_string());
    }
    // batch / ubatch：同时设两值保持 logical=physical batch 一致，
    // 用户能预测显存占用；不像 llama.cpp 默认 batch=2048 / ubatch=512 拉开两倍。
    if let Some(b) = overrides.batch_size {
        let b = b.max(32); // llama-server 拒绝过小的 batch；32 是安全下限
        cmd.arg("--batch-size").arg(b.to_string());
        cmd.arg("--ubatch-size").arg(b.to_string());
    }
    // ctx-size + parallel slots 协同：llama-server 把 --ctx-size 平均分给
    // np 个 slot（per-slot = ctx_size / np）。所以这里实际传的 --ctx-size
    // 必须是 (per-slot 上限 × np)，让每个 slot 都拿到用户选的 budget。
    // 否则 -np 4 时 user 看到的 8K base 会缩成每槽 2048，长 prompt 直接 400。
    let np = overrides.parallel_slots.unwrap_or(1).max(1);
    let per_slot_ctx = overrides.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    cmd.arg("--ctx-size")
        .arg(per_slot_ctx.saturating_mul(np).to_string());
    // 总是显式传 -np，哪怕是 1。不传时 llama-server auto-fallback 到 4 slot，
    // 让用户的"段总结 single slot"配置失效——summary phase 也起 4 槽，浪费 KV 预算 +
    // 让 checkpoint 池跨 slot 翻 4 倍。
    cmd.arg("-np").arg(np.to_string());
    // 关掉 llama.cpp 新版默认开的 prompt cache：8 GB 上限，纯吃显存（日报场景每段
    // prompt 都不同，命中率几乎为 0）。
    cmd.arg("--cache-ram").arg("0");
    // 关掉 context checkpoints：每个 50 MiB × max 32 个 × N slots 持续累积——大模型 +
    // 多 slot 时能堆出 GB 级开销，导致段总结生成途中 server 崩
    // （"connection closed before message completed"）。我们没复用 prompt prefix 的
    // 场景，关掉无副作用。
    cmd.arg("--ctx-checkpoints").arg("0");
    // 截 stderr/stdout 用于失败时回放给用户；不让它们污染 Hindsight 自己的终端
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());

    // Windows 下抑制控制台窗口闪一下
    // tokio::process::Command 自带 creation_flags 方法（inherent on Windows），
    // 不用 import std::os::windows::process::CommandExt
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(o: &EngineStartOverrides) -> Vec<String> {
        build_command(Path::new("llama-server"), 4242, Path::new("m.gguf"), o)
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn value_of(args: &[String], flag: &str) -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).cloned()
    }

    /// 每槽都拿到要的上下文：传给引擎的是「每槽 × 槽数」。没设就是 8K、1 槽。
    #[test]
    fn ctx_size_is_per_slot_times_slots() {
        let a = args(&EngineStartOverrides::default());
        assert_eq!(value_of(&a, "--ctx-size").as_deref(), Some("8192"));
        assert_eq!(value_of(&a, "-np").as_deref(), Some("1"));

        let a = args(&EngineStartOverrides {
            ctx_size: Some(16384),
            parallel_slots: Some(2),
            batch_size: None,
        });
        assert_eq!(value_of(&a, "--ctx-size").as_deref(), Some("32768"));
        assert_eq!(value_of(&a, "-np").as_deref(), Some("2"));
    }

    /// 批大小同时设 batch 和 ubatch，太小的抬到 32。
    #[test]
    fn batch_size_sets_both_and_has_a_floor() {
        let a = args(&EngineStartOverrides {
            batch_size: Some(8),
            ..Default::default()
        });
        assert_eq!(value_of(&a, "--batch-size").as_deref(), Some("32"));
        assert_eq!(value_of(&a, "--ubatch-size").as_deref(), Some("32"));
        assert_eq!(value_of(&a, "-m").as_deref(), Some("m.gguf"));
    }
}
