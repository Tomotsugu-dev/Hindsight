//! Command line arguments for starting `llama-server`.

use std::path::Path;

use tokio::process::Command;

use crate::ai::platform;

/// Default context size per slot, in tokens.
///
/// Large enough for long daily report and chat input, without using too much memory.
pub const DEFAULT_CTX_SIZE: u32 = 8192;

/// Optional settings for starting `llama-server`. `None` keeps the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartOverrides {
    /// Sets both `--batch-size` and `--ubatch-size`. `None` keeps the llama.cpp default.
    ///
    /// A larger value reads the prompt faster, but uses more video memory at peak.
    pub batch_size: Option<u32>,
    /// `-np`: how many requests the engine serves at the same time. `None` means 1.
    ///
    /// More slots help only when requests arrive at the same time.
    pub parallel_slots: Option<u32>,
    /// Context limit **per slot**. `None` means [`DEFAULT_CTX_SIZE`].
    ///
    /// `llama-server` gets `--ctx-size` = `ctx_size × parallel_slots`, so every slot gets the
    /// full amount. The KV cache for all slots is allocated once, at start.
    pub ctx_size: Option<u32>,
}

/// Builds the command that starts `llama-server` with `model` on the local `port`.
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
    // TODO: Use `--fit`, which llama-server turns on by default. Stop passing -ngl,
    // --ctx-size and -np, so it sizes them to the device memory. The guard must then ask
    // the engine for its real context before reuse. Upgrade llama.cpp first (TODO.md step 6).
    if platform::has_gpu_backend(platform::detect()) {
        cmd.arg("-ngl").arg("all");
    }
    // Use one value for both. The defaults (2048 and 512) differ, which makes memory use
    // harder to predict.
    if let Some(batch_size) = overrides.batch_size {
        let batch_size = batch_size.max(32); // llama-server rejects very small batches
        cmd.arg("--batch-size").arg(batch_size.to_string());
        cmd.arg("--ubatch-size").arg(batch_size.to_string());
    }
    // llama-server splits `--ctx-size` evenly across the `-np` slots, so pass
    // "per-slot size × slots". Otherwise each slot gets less, and long prompts fail with 400.
    let parallel_slots = overrides.parallel_slots.unwrap_or(1).max(1);
    let per_slot_ctx = overrides.ctx_size.unwrap_or(DEFAULT_CTX_SIZE);
    cmd.arg("--ctx-size")
        .arg(per_slot_ctx.saturating_mul(parallel_slots).to_string());
    // Always pass -np, even 1. Without it, llama-server picks the slot count itself and
    // uses more KV cache.
    cmd.arg("-np").arg(parallel_slots.to_string());
    // Turn off the prompt cache (up to 8 GB by default). Report segments differ a lot, so it
    // almost never hits.
    cmd.arg("--cache-ram").arg("0");
    // Turn off context checkpoints. With several slots and long sessions they can grow to
    // several GB and crash the server in the middle of a reply.
    cmd.arg("--ctx-checkpoints").arg("0");
    // Capture the output to explain a failed start, and keep it out of the app's terminal.
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());

    // Do not open a console window on Windows. tokio's `Command` has `creation_flags` built in.
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

    // 引擎能用显卡就把层全放进显存，不能用就不传。
    #[test]
    fn gpu_layers_are_all_or_not_passed() {
        let args = command_args(&EngineStartOverrides::default());
        let want = platform::has_gpu_backend(platform::detect()).then(|| "all".to_string());
        assert_eq!(flag_value(&args, "-ngl"), want);
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
