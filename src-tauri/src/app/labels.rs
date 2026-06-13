//! 应用窗口标签的单一来源。
//! 集中维护 settings/gallery/carousel/desktop_background 等 Tauri window label。
//! 只放稳定标识和简单分类判断，不放窗口创建或生命周期逻辑。

pub(crate) const SETTINGS_LABEL: &str = "settings";
pub(crate) const GALLERY_LABEL: &str = "gallery";
pub(crate) const CAROUSEL_LABEL: &str = "carousel";
pub(crate) const DESKTOP_BACKGROUND_LABEL: &str = "desktop_background";

pub(crate) fn is_gallery_window(label: &str) -> bool {
    label == GALLERY_LABEL || label == CAROUSEL_LABEL || label == DESKTOP_BACKGROUND_LABEL
}
