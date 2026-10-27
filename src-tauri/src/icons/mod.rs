//! 图标：应用图标（[`app`]，每个系统一个实现）和网站图标（[`site`]，所有系统共用）。

pub(crate) mod app;
pub(crate) mod site;

/// Scales `img` down to fit `max_px` × `max_px`, keeping its proportions. A smaller image is
/// returned as it is: enlarging adds no detail.
///
/// Triangle: on icons this small, sharper filters such as Lanczos3 look no different and
/// cost more.
pub(crate) fn shrink_to_fit(img: image::DynamicImage, max_px: u32) -> image::DynamicImage {
    if img.width() <= max_px && img.height() <= max_px {
        return img;
    }
    img.resize(max_px, max_px, image::imageops::FilterType::Triangle)
}
