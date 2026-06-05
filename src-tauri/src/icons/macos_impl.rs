use std::path::{Path, PathBuf};

use crate::error::Result;

/// macOS 实现：找到 exe 所在 .app bundle → 读 Info.plist 拿 CFBundleIconFile →
/// 解码 .icns 取最大变体编码 PNG。bundle 找不到返回 `Ok(None)`。
pub fn extract_png(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    let bundle = match find_bundle(exe_path) {
        Some(b) => b,
        None => return Ok(None),
    };

    let plist_path = bundle.join("Contents/Info.plist");
    if !plist_path.exists() {
        return Ok(None);
    }

    let icon_name = match read_icon_name(&plist_path) {
        Some(n) => n,
        None => return Ok(None),
    };

    let icns_path = resolve_icns(&bundle, &icon_name);
    if !icns_path.exists() {
        return Ok(None);
    }

    extract_largest_png(&icns_path)
}

fn find_bundle(exe_path: &Path) -> Option<PathBuf> {
    // 情况 1：`exe_path` 自身就是 `.app` —— [`capture::window`] 通过 NSWorkspace 拿
    // 到的 `bundleURL` 是 `/Applications/Google Chrome.app` 这种形式，process_paths
    // 表里存的也是它，所以本路径最常见。
    if exe_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.ends_with(".app"))
        .unwrap_or(false)
    {
        return Some(exe_path.to_path_buf());
    }
    // 情况 2：`exe_path` 是 bundle 里的 binary（`.../Foo.app/Contents/MacOS/Foo`）
    // —— 兼容历史调用方 / 第三方 process_paths 来源。往父目录走找 `.app`。
    let mut cur = exe_path.to_path_buf();
    while let Some(parent) = cur.parent() {
        if parent
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".app"))
            .unwrap_or(false)
        {
            return Some(parent.to_path_buf());
        }
        cur = parent.to_path_buf();
    }
    None
}

fn read_icon_name(plist_path: &Path) -> Option<String> {
    let value = plist::Value::from_file(plist_path).ok()?;
    let dict = value.as_dictionary()?;
    let raw = dict.get("CFBundleIconFile")?.as_string()?.to_string();
    Some(raw)
}

fn resolve_icns(bundle: &Path, icon_name: &str) -> PathBuf {
    let resources = bundle.join("Contents/Resources");
    if icon_name.to_ascii_lowercase().ends_with(".icns") {
        resources.join(icon_name)
    } else {
        resources.join(format!("{icon_name}.icns"))
    }
}

fn extract_largest_png(icns_path: &Path) -> Result<Option<Vec<u8>>> {
    // 上界：UI 实际 size=18px，过去取最大 .icns 变体直出 PNG（512/1024px）让单
    // 图标 PNG 达 ~188 KB，缩放后 64px 通常 3–8 KB（10–30× 体积差），同时大幅降
    // WKWebView 端解码后位图的 RAM 占用。128 留余量给 HiDPI。
    const MAX_DIM: u32 = 128;

    let file = std::fs::File::open(icns_path)?;
    let family = match icns::IconFamily::read(file) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    // 仍然挑最大变体（最高分辨率源）—— resize 自高质量源比从低质量源放大好。
    let mut best: Option<(u32, icns::Image)> = None;
    for ty in family.available_icons() {
        if let Ok(image) = family.get_icon_with_type(ty) {
            let w = image.width();
            if best.as_ref().is_none_or(|(bw, _)| w > *bw) {
                best = Some((w, image));
            }
        }
    }
    let (src_w, src_img) = match best {
        Some(b) => b,
        None => return Ok(None),
    };

    // icns 变体可能是 RGB / GrayAlpha / Gray，统一转 RGBA 让 image crate 接住
    let rgba = src_img.convert_to(icns::PixelFormat::RGBA);
    let dyn_img = match image::RgbaImage::from_raw(rgba.width(), rgba.height(), rgba.data().to_vec()) {
        Some(buf) => image::DynamicImage::ImageRgba8(buf),
        None => return Ok(None),
    };

    // Triangle filter：速度 / 质量平衡好；CatmullRom / Lanczos3 在小图标上视觉差异不明显，多耗 CPU
    let final_img = if src_w > MAX_DIM {
        dyn_img.resize(MAX_DIM, MAX_DIM, image::imageops::FilterType::Triangle)
    } else {
        dyn_img
    };

    let mut buf = std::io::Cursor::new(Vec::new());
    final_img
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| crate::error::Error::Capture(format!("icns PNG encode: {e}")))?;
    Ok(Some(buf.into_inner()))
}
