fn main() {
    tauri_build::build();

    // Phase 1C 之前的 copy_onnxruntime_dylib 已废弃——onnxruntime dylib 现在
    // 走运行期 lazy-download（[`crate::ai::embedding_runtime`]），落到
    // `<data_root>/ai/runtime/`。Dev 第一次跑 AI 总结时下，跟 prod 行为一致。
    // 旧的 `src-tauri/resources/runtime/` 不再使用；相关 fetch 脚本仅作离线
    // dev 兜底（具体见脚本头注释）。

    add_swift_concurrency_rpath();
}

/// 给 macOS dev 二进制补 `LC_RPATH`，让 `@rpath/libswift_Concurrency.dylib`
/// 解析到系统 dyld shared cache 里的 `/usr/lib/swift/libswift_Concurrency.dylib`。
///
/// 起因：`screencapturekit-rs` build 出的 `libScreenCaptureKitBridge.a` 静态库
/// 引用 `@rpath/libswift_Concurrency.dylib` 但没安排 rustc 埋 `LC_RPATH`，
/// 直跑 `target/debug/hindsight` 会 dyld 报 "Library not loaded"。
///
/// 优先用 `/usr/lib/swift`（dyld 共享缓存路径）—— 二进制本身链接的其它 Swift
/// 标准库 (`libswiftCore.dylib` 等) 都从这里加载，rpath 指向同一目录可以让
/// `libswift_Concurrency.dylib` 也命中同一份，避免 toolchain copy + cache copy
/// 同时加载产生 "Class _TtCs... is implemented in both" 重复类警告。
///
/// 生产 app bundle 由 Tauri 打包流程自带 Swift 运行时，不受影响 —— 这条只为
/// dev 命令行运行兜底，prod 不需要。
#[cfg(target_os = "macos")]
fn add_swift_concurrency_rpath() {
    // dyld 共享缓存路径（macOS 12+ 起 Swift Concurrency 进 OS），文件本身不一定
    // 在磁盘上存在，但 dyld 加载 `@rpath/libswift_Concurrency.dylib` + rpath
    // `/usr/lib/swift` 会查到 cache。
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    // 兜底：万一系统 cache 没收录（异常环境），再加一条 Xcode toolchain 路径。
    // dyld 按 rpath 顺序查找；系统找到就不会走兜底。
    if let Ok(o) = std::process::Command::new("xcrun")
        .args(["--toolchain", "default", "--show-toolchain-path"])
        .output()
    {
        if o.status.success() {
            if let Ok(s) = String::from_utf8(o.stdout) {
                let toolchain = s.trim();
                for sub in ["swift-5.5/macosx", "swift/macosx"] {
                    let dir = format!("{toolchain}/usr/lib/{sub}");
                    let dylib = format!("{dir}/libswift_Concurrency.dylib");
                    if std::path::Path::new(&dylib).exists() {
                        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn add_swift_concurrency_rpath() {}
