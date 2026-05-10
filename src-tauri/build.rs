fn main() {
    tauri_build::build();

    // Phase 1C 之前的 copy_onnxruntime_dylib 已废弃——onnxruntime dylib 现在
    // 走运行期 lazy-download（[`crate::ai::embedding_runtime`]），落到
    // `<data_root>/ai/runtime/`。Dev 第一次跑 AI 总结时下，跟 prod 行为一致。
    // 旧的 `src-tauri/resources/runtime/` 不再使用；相关 fetch 脚本仅作离线
    // dev 兜底（具体见脚本头注释）。
}
