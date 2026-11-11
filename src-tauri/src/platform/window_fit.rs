//! 让主窗口适应 Windows 的「文本大小」和屏幕工作区。
//!
//! 界面按固定的页面尺寸设计（tauri.conf.json 里主窗口的 960 宽、640～820 高），窗口也按这个
//! 逻辑尺寸创建。Windows 上有两种情况会让页面拿不到这个尺寸：
//!
//! - 「设置 → 辅助功能 → 文本大小」调大时，WebView2 按「显示缩放 × 文本大小」放大内容，而
//!   窗口只按显示缩放算大小。文本大小 160% 时窗口不变、内容大 1.6 倍，页面只剩 600 宽，
//!   各处控件被挤压、裁切。
//! - 显示缩放很高的小屏幕（如 1080p 开 175%）放不下 960×640 的窗口，超出的部分落在屏幕外。
//!
//! 这里在窗口创建后把它调成「页面仍是设计尺寸」：窗口跟着内容的放大倍数变大；屏幕放不下时，
//! 再把 WebView 缩小到刚好放下。只在创建时算一次，运行中改了文本大小要重启应用才生效。
//!
//! 岛那样自己按显示缩放算位置和大小的窗口不跟着变大，改用 `ignore_text_scale`。

/// 主窗口的设计尺寸（页面的 CSS 像素），来自 tauri.conf.json。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Design {
    width: f64,
    height: f64,
    min_height: f64,
    max_height: f64,
}

/// 算出来的窗口尺寸（物理像素）和 WebView 缩放。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fit {
    /// WebView 缩放：屏幕放得下时是 1，放不下时小于 1
    zoom: f64,
    width: f64,
    height: f64,
    min_height: f64,
    max_height: f64,
}

/// 按内容的放大倍数和屏幕工作区算窗口尺寸。
///
/// - `raster`：缩放为 1 时，WebView2 的一个 CSS 像素占多少物理像素（显示缩放 × 文本大小）
/// - `work_area`：窗口所在屏幕的工作区，物理像素的（宽, 高）
///
/// 结果保证：页面宽度等于设计宽度，页面高度不小于设计的最小高度，窗口的宽和最小高度都不超出
/// 工作区。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn fit(design: Design, raster: f64, work_area: (f64, f64)) -> Fit {
    let zoom = (work_area.0 / (design.width * raster))
        .min(work_area.1 / (design.min_height * raster))
        .min(1.0);
    // 一个 CSS 像素最终占多少物理像素
    let px = raster * zoom;
    Fit {
        zoom,
        width: design.width * px,
        height: (design.height * px).min(work_area.1),
        min_height: design.min_height * px,
        max_height: design.max_height * px,
    }
}

/// 缩放为 1 时，WebView2 把一个 CSS 像素画成多少物理像素（显示缩放 × 文本大小）。
/// 这是页面实际被放大的倍数，比自己去读系统设置可靠。
#[cfg(target_os = "windows")]
fn rasterization_scale(window: &tauri::WebviewWindow) -> Option<f64> {
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = window.with_webview(move |pw| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller3;
        use windows_core::Interface;

        let mut raster = 0.0;
        let read = pw
            .controller()
            .cast::<ICoreWebView2Controller3>()
            .and_then(|c| c.RasterizationScale(&mut raster));
        let _ = tx.send(read.map(|()| raster));
    });
    match sent.map(|()| rx.recv()) {
        Ok(Ok(Ok(raster))) => Some(raster),
        other => {
            log::warn!("读不到 WebView2 的放大倍数: {other:?}");
            None
        }
    }
}

/// 主窗口创建后调用（显示之前）：按 WebView2 实际的放大倍数和屏幕工作区调整窗口尺寸、
/// 尺寸上下限和 WebView 缩放，窗口超出工作区时挪回来。
#[cfg(target_os = "windows")]
pub fn fit_main_window(window: &tauri::WebviewWindow) {
    use tauri::{LogicalSize, Manager, PhysicalPosition};

    let Some(design) = window
        .app_handle()
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == window.label())
        .and_then(|c| {
            Some(Design {
                width: c.width,
                height: c.height,
                min_height: c.min_height?,
                max_height: c.max_height?,
            })
        })
    else {
        log::warn!("主窗口配置里没有高度上下限，跳过窗口适配");
        return;
    };

    let Some(raster) = rasterization_scale(window) else {
        return;
    };
    let (Ok(Some(monitor)), Ok(scale), Ok(outer), Ok(inner)) = (
        window.current_monitor(),
        window.scale_factor(),
        window.outer_position(),
        window.inner_position(),
    ) else {
        log::warn!("读不到主窗口所在的屏幕，跳过窗口适配");
        return;
    };
    let area = monitor.work_area();
    let f = fit(
        design,
        raster,
        (area.size.width as f64, area.size.height as f64),
    );

    // 尺寸按逻辑像素设：窗口换到显示缩放不同的屏幕时，tao 会按逻辑尺寸重算，页面尺寸不变
    let logical = |h: f64| LogicalSize::new(f.width / scale, h / scale);
    let _ = window.set_min_size(Some(logical(f.min_height)));
    let _ = window.set_max_size(Some(logical(f.max_height)));
    let _ = window.set_size(logical(f.height));
    let _ = window.set_zoom(f.zoom);

    // 窗口变大后可能伸出工作区：把页面区域挪到刚好放下的位置。窗口外沿比页面区域多一圈
    // 看不见的缩放边框，所以按页面区域算，再换算回窗口位置
    let fit_in = |pos: i32, start: i32, len: u32, size: f64| {
        pos.min(start + len as i32 - size.ceil() as i32).max(start)
    };
    let x = fit_in(inner.x, area.position.x, area.size.width, f.width);
    let y = fit_in(inner.y, area.position.y, area.size.height, f.height);
    if (x, y) != (inner.x, inner.y) {
        let _ = window.set_position(PhysicalPosition::new(
            outer.x + x - inner.x,
            outer.y + y - inner.y,
        ));
    }

    log::info!(
        "主窗口适配：内容放大 {raster:.3} 倍（显示缩放 {scale}），WebView 缩放 {:.3}，窗口 {:.0}×{:.0} 物理像素",
        f.zoom,
        f.width,
        f.height
    );
}

/// 其它平台：no-op（文本大小和工作区的问题只在 Windows + WebView2 上有）。
#[cfg(not(target_os = "windows"))]
pub fn fit_main_window(_window: &tauri::WebviewWindow) {}

/// 岛这种自己按显示缩放摆位置、算大小的窗口：让内容也只按显示缩放放大，不跟着「文本大小」
/// 变大。否则页面量出来的尺寸乘上显示缩放后比实际画出来的小，内容会被窗口裁掉。
#[cfg(target_os = "windows")]
pub fn ignore_text_scale(window: &tauri::WebviewWindow) {
    if let (Some(raster), Ok(scale)) = (rasterization_scale(window), window.scale_factor()) {
        let _ = window.set_zoom(scale / raster);
    }
}

#[cfg(test)]
mod tests {
    use super::{fit, Design};

    /// tauri.conf.json 里主窗口的尺寸
    const MAIN: Design = Design {
        width: 960.0,
        height: 720.0,
        min_height: 640.0,
        max_height: 820.0,
    };

    /// 页面的 CSS 尺寸：（宽, 默认高, 最小高）
    fn page(raster: f64, work_area: (f64, f64)) -> (f64, f64, f64) {
        let f = fit(MAIN, raster, work_area);
        let px = raster * f.zoom;
        // 窗口的宽和最小高度都放得进工作区
        assert!(f.width <= work_area.0 + 1e-6);
        assert!(f.min_height <= work_area.1 + 1e-6);
        assert!(f.height <= work_area.1 + 1e-6);
        (f.width / px, f.height / px, f.min_height / px)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    // 2560×1440，显示缩放 125%，文本大小 100%：窗口和现在一样
    #[test]
    fn window_keeps_its_size_without_text_scaling() {
        let f = fit(MAIN, 1.25, (2560.0, 1380.0));
        assert_eq!(f.zoom, 1.0);
        assert!(close(f.width, 1200.0) && close(f.height, 900.0));
        assert!(close(f.min_height, 800.0) && close(f.max_height, 1025.0));
    }

    // 同一块屏幕，文本大小 125%：窗口变成 1500×1125，页面仍是 960×720
    #[test]
    fn window_grows_with_text_size_and_page_keeps_the_design_size() {
        let f = fit(MAIN, 1.25 * 1.25, (2560.0, 1380.0));
        assert_eq!(f.zoom, 1.0);
        assert!(close(f.width, 1500.0) && close(f.height, 1125.0));
        let (w, h, _) = page(1.25 * 1.25, (2560.0, 1380.0));
        assert!(close(w, 960.0) && close(h, 720.0));
    }

    // 文本大小 160%：默认高度要 1440，工作区只有 1380 高，用满工作区
    #[test]
    fn default_height_is_capped_to_the_work_area() {
        let f = fit(MAIN, 2.0, (2560.0, 1380.0));
        assert_eq!(f.zoom, 1.0);
        assert!(close(f.width, 1920.0) && close(f.height, 1380.0));
        let (w, h, min_h) = page(2.0, (2560.0, 1380.0));
        assert!(close(w, 960.0) && close(h, 690.0) && close(min_h, 640.0));
    }

    // 文本大小 225%：最小高度要 1800，工作区只有 1380 高，缩小 WebView 补上
    #[test]
    fn webview_zooms_out_when_the_screen_is_too_small() {
        let f = fit(MAIN, 1.25 * 2.25, (2560.0, 1380.0));
        assert!(close(f.zoom, 1380.0 / 1800.0));
        let (w, h, min_h) = page(1.25 * 2.25, (2560.0, 1380.0));
        assert!(close(w, 960.0) && close(h, 640.0) && close(min_h, 640.0));
    }

    // 显示缩放很高的小屏幕：窗口放得下，页面仍是设计尺寸
    #[test]
    fn small_screens_at_high_display_scale_fit() {
        // 1920×1080 开 175%，文本大小 100%：工作区逻辑高度只有 569，放不下 640
        let (w, h, min_h) = page(1.75, (1920.0, 996.0));
        assert!(close(w, 960.0) && close(h, 640.0) && close(min_h, 640.0));
        // 1366×768 开 125%
        let (w, _, min_h) = page(1.25, (1366.0, 708.0));
        assert!(close(w, 960.0) && close(min_h, 640.0));
    }

    // 1080×1920 竖屏开 125%：逻辑宽度只有 864，放不下 960，按宽度缩小
    #[test]
    fn portrait_screen_is_limited_by_width() {
        let f = fit(MAIN, 1.25, (1080.0, 1860.0));
        assert!(close(f.zoom, 0.9) && close(f.width, 1080.0));
        let (w, h, _) = page(1.25, (1080.0, 1860.0));
        assert!(close(w, 960.0) && close(h, 720.0));
    }
}
