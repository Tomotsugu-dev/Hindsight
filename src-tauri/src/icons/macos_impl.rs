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
    let file = std::fs::File::open(icns_path)?;
    let family = match icns::IconFamily::read(file) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    let mut best: Option<(u32, Vec<u8>)> = None;
    for ty in family.available_icons() {
        if let Ok(image) = family.get_icon_with_type(ty) {
            let w = image.width();
            let pick = match &best {
                Some((bw, _)) => w > *bw,
                None => true,
            };
            if pick {
                let mut buf = std::io::Cursor::new(Vec::new());
                if image.write_png(&mut buf).is_ok() {
                    best = Some((w, buf.into_inner()));
                }
            }
        }
    }
    Ok(best.map(|(_, b)| b))
}
