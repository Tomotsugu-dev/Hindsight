use std::path::Path;

fn main() {
    tauri_build::build();

    // Phase 1C：把 onnxruntime DLL 复制到 target/<profile>/，让 ort load-dynamic
    // 在 dev / cargo run 时能找到。Tauri 打 release 包时由 bundle.resources 处理。
    copy_onnxruntime_dylib();
}

/// 把 `resources/runtime/<libname>` 复制到 `target/<profile>/<libname>`。
/// 文件不存在 → 打印 cargo:warning 提示，但**不**让 build 失败——首次 clone 时
/// 用户还没拉过 DLL 也能正常 cargo check / cargo build；只有运行期 ort 真要加载
/// 时才会缺。
fn copy_onnxruntime_dylib() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let runtime_dir = Path::new(&manifest_dir).join("resources").join("runtime");
    println!("cargo:rerun-if-changed={}", runtime_dir.display());

    // 平台对应文件名（跟 ort load-dynamic 默认搜索名一致）
    let libnames: &[&str] = if cfg!(target_os = "windows") {
        &["onnxruntime.dll"]
    } else if cfg!(target_os = "macos") {
        &["libonnxruntime.dylib"]
    } else {
        &["libonnxruntime.so"]
    };

    let Some(target_dir) = locate_target_dir() else {
        println!("cargo:warning=无法定位 target 目录，跳过 onnxruntime 复制");
        return;
    };

    for name in libnames {
        let src = runtime_dir.join(name);
        if !src.exists() {
            println!(
                "cargo:warning=未找到 {}；运行期 ort 会 panic。请跑 \
                 `pwsh src-tauri/scripts/fetch-onnxruntime.ps1` 拉取 DLL。",
                src.display()
            );
            continue;
        }
        let dst = target_dir.join(name);
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!(
                "cargo:warning=复制 {} → {} 失败: {e}",
                src.display(),
                dst.display()
            );
        }
    }
}

/// `OUT_DIR` 在 `target/<profile>/build/<crate-hash>/out`，逐级 parent 找到 profile 目录。
/// 例：OUT_DIR=...\target\debug\build\hindsight-xxx\out → 返回 ...\target\debug
fn locate_target_dir() -> Option<std::path::PathBuf> {
    let out_dir = std::env::var_os("OUT_DIR")?;
    let mut p = std::path::PathBuf::from(out_dir);
    // 弹掉 out / <hash> / build → 剩下 profile 目录
    p.pop();
    p.pop();
    p.pop();
    Some(p)
}
