//! macOS-only：ScreenCaptureKit 一次性截图焦点窗口。
//!
//! 为什么不用 xcap：
//! - xcap 0.4 macOS 路径用的是 `CGWindowListCopyWindowInfo` + `CGWindowListCreateImage`，
//!   都在 macOS 14 (Sonoma) 标记为 deprecated，release/signed/hardened-runtime build 下
//!   表现退化（社区广泛 confirm）；
//! - `OptionOnScreenOnly` 模式跨 Space 拿不到窗口——Hindsight 在副屏、Chrome 在主屏
//!   独立 fullscreen Space 时，xcap 完全摸不到 Chrome 窗口；
//! - 之前给 xcap fallback 加的 `NSScreen.mainScreen` 又强制要 `MainThreadMarker`，
//!   tokio worker 拿不到 → fallback 彻底死。
//!
//! SCK (`SCScreenshotManager.captureImage`，macOS 14.0+) 是 Apple 给现代录屏类 app
//! 设计的官方路径：跨 Space + 跨监视器枚举 + 截图都 OK，对线程无要求。
//!
//! 调用方应在 `tokio::spawn_blocking` 里跑——`SCScreenshotManager::capture_image`
//! 内部用 dispatch_semaphore 同步等结果，单次 50-200ms 阻塞。

use image::RgbaImage;
use screencapturekit::{
    screenshot_manager::SCScreenshotManager,
    shareable_content::SCShareableContent,
    stream::{configuration::SCStreamConfiguration, content_filter::SCContentFilter},
};

use crate::error::{Error, Result};

/// 截取 `frontmost_pid` 所属 app 当前最大可见窗口的内容，返回 RGBA pixel buffer。
///
/// `frontmost_pid` 由调用方先用 `NSWorkspace.frontmostApplication.processIdentifier()`
/// 拿到（系统层"真正最前 app"PID，跨屏跨 Space 都对）。
///
/// 失败场景：
/// - 用户没授 Screen Recording 权限：第一次调用时 OS 弹"打开系统设置"对话框，本次返 Err；
///   用户授权后下次 tick 即可成功
/// - 目标 app 没有可见窗口（极少见——比如 menubar-only app）：返 Err，调用方跳过该 tick
pub fn capture_focused_window(frontmost_pid: u32) -> Result<RgbaImage> {
    // 1. 枚举当前所有可见窗口（含跨 Space 跨监视器）
    let content = SCShareableContent::get()
        .map_err(|e| Error::Capture(format!("SCShareableContent::get: {e:?}")))?;

    // 2. 在 windows 列表里挑 PID 匹配、屏上可见的；同 app 多窗时取最大那个
    //    （主窗口启发式——配置面板 / utility panel 通常较小）
    //
    // `window_layer() == 0` 过滤至关重要：Apple 的 NSWindow level 体系里
    //   - 0           = kCGNormalWindowLevel，普通应用窗口
    //   - 负数 / 大正数 = 桌面图标窗口 / dock / menubar / 屏保 / 系统模态等
    // Finder 同时拥有"文件浏览器窗口（layer=0）"和"桌面图标窗口（layer 极大负数、
    // 全屏尺寸）"——不加这条过滤的话 `max_by(area)` 永远挑桌面窗口，但
    // `SCContentFilter::with_window()` 底层走 `desktop_independent_window`，
    // 把桌面窗口塞给一个专门排除桌面的 filter，渲出来必是黑屏。其它 app
    // （Chrome / Slack 等）的主窗口也都是 layer=0，这条过滤对它们无影响。
    let windows = content.windows();
    let target = windows
        .iter()
        .filter(|w| w.is_on_screen())
        .filter(|w| w.window_layer() == 0)
        .filter(|w| {
            w.owning_application()
                .map(|app| app.process_id() == frontmost_pid as i32)
                .unwrap_or(false)
        })
        .max_by(|a, b| {
            let area_a = a.frame().width * a.frame().height;
            let area_b = b.frame().width * b.frame().height;
            area_a.partial_cmp(&area_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| Error::Capture(format!("no normal SCWindow for pid {frontmost_pid}")))?;

    // 3. SCContentFilter 圈定那一个窗口（with_window 内部走
    //    sc_content_filter_create_with_desktop_independent_window，得到的是
    //    "窗口本身"的内容滤镜，不带桌面 / 同屏其它窗口）
    let frame = target.frame();
    let w_px = frame.width as u32;
    let h_px = frame.height as u32;
    if w_px == 0 || h_px == 0 {
        return Err(Error::Capture("target SCWindow has zero size".into()));
    }
    let filter = SCContentFilter::create().with_window(target).build();

    // 4. Stream config：分辨率跟窗口物理尺寸对齐
    let config = SCStreamConfiguration::new()
        .with_width(w_px)
        .with_height(h_px);

    // 5. 同步阻塞调，拿 CGImage（crate 内部 dispatch_semaphore 等回调）
    let cg_image = SCScreenshotManager::capture_image(&filter, &config)
        .map_err(|e| Error::Capture(format!("SCScreenshotManager::capture_image: {e:?}")))?;

    // 6. CGImage → RgbaImage：crate 自带 rgba_data() 已经把 row-padding / BGRA→RGBA
    //    都处理好了，直接 from_raw
    let w = cg_image.width() as u32;
    let h = cg_image.height() as u32;
    let rgba = cg_image
        .rgba_data()
        .map_err(|e| Error::Capture(format!("CGImage::rgba_data: {e:?}")))?;

    RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| Error::Capture("RgbaImage::from_raw failed (size mismatch)".into()))
}
