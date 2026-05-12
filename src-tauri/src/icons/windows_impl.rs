use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use image::{ImageBuffer, Rgba};

use crate::error::Result;

const ICON_SIZE: i32 = 48;

/// Windows 实现：用 ExtractIconExW 拿 exe 关联的最大尺寸图标，DC + GDI 渲染到位图后编码 PNG。
/// 文件不存在或无图标都返回 `Ok(None)`。
pub fn extract_png(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    if !exe_path.exists() {
        return Ok(None);
    }
    // SAFETY: `extract_inner` 内全部走 Win32 公开 API（Shell32 / GDI32 / User32），
    // 调用前先 zero-init 结构体、正确设 cbSize，调用后所有 GDI 句柄（HICON / HBITMAP
    // / HDC）都通过 DestroyIcon / DeleteObject / DeleteDC / ReleaseDC 配对释放。
    unsafe { extract_inner(exe_path) }
}

/// # Safety
///
/// 调用方必须保证 `exe_path` 当前存在（前置检查已在 [`extract_png`] 完成）。
/// 函数内每个分配出的 Windows 资源（HICON / HBITMAP / HDC）都在所有 early-return
/// 路径上正确释放，无泄漏 / double-free 风险。
unsafe fn extract_inner(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    use winapi::shared::windef::{HBITMAP, HDC};
    use winapi::um::shellapi::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use winapi::um::wingdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
    };
    use winapi::um::winuser::{DestroyIcon, DrawIconEx, GetDC, ReleaseDC};
    const DI_NORMAL: u32 = 0x0003;

    let wide: Vec<u16> = std::ffi::OsStr::new(exe_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut sfi: SHFILEINFOW = std::mem::zeroed();
    let result = SHGetFileInfoW(
        wide.as_ptr(),
        0,
        &mut sfi,
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON,
    );
    if result == 0 || sfi.hIcon.is_null() {
        return Ok(None);
    }

    let hicon = sfi.hIcon;

    let screen_dc: HDC = GetDC(std::ptr::null_mut());
    if screen_dc.is_null() {
        DestroyIcon(hicon);
        return Ok(None);
    }
    let mem_dc = CreateCompatibleDC(screen_dc);
    if mem_dc.is_null() {
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        DestroyIcon(hicon);
        return Ok(None);
    }
    let bmp: HBITMAP = CreateCompatibleBitmap(screen_dc, ICON_SIZE, ICON_SIZE);
    if bmp.is_null() {
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        DestroyIcon(hicon);
        return Ok(None);
    }

    let old_bmp = SelectObject(mem_dc, bmp as *mut _);

    // CreateCompatibleBitmap 的内容未定义，先铺一层
    let _ = BitBlt(
        mem_dc,
        0,
        0,
        ICON_SIZE,
        ICON_SIZE,
        std::ptr::null_mut(),
        0,
        0,
        SRCCOPY,
    );

    let drew = DrawIconEx(
        mem_dc,
        0,
        0,
        hicon,
        ICON_SIZE,
        ICON_SIZE,
        0,
        std::ptr::null_mut(),
        DI_NORMAL,
    );

    if drew == 0 {
        SelectObject(mem_dc, old_bmp);
        DeleteObject(bmp as *mut _);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        DestroyIcon(hicon);
        return Ok(None);
    }

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = ICON_SIZE;
    // 负值代表 top-down DIB（默认是 bottom-up）
    bmi.bmiHeader.biHeight = -ICON_SIZE;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    let mut pixels: Vec<u8> = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    let read = GetDIBits(
        mem_dc,
        bmp,
        0,
        ICON_SIZE as u32,
        pixels.as_mut_ptr() as *mut _,
        &mut bmi,
        DIB_RGB_COLORS,
    );

    SelectObject(mem_dc, old_bmp);
    DeleteObject(bmp as *mut _);
    DeleteDC(mem_dc);
    ReleaseDC(std::ptr::null_mut(), screen_dc);
    DestroyIcon(hicon);

    if read == 0 {
        return Ok(None);
    }

    // GDI 给的是 BGRA，转成 image crate 期望的 RGBA
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(ICON_SIZE as u32, ICON_SIZE as u32, pixels).ok_or(
            crate::error::Error::Capture("icon: PNG buffer 构造失败".into()),
        )?;

    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| crate::error::Error::Capture(format!("icon: PNG encode: {e}")))?;
    Ok(Some(out.into_inner()))
}
