//! 窗口系统模块。
//! 管理普通应用窗口和桌面背景窗口两类生命周期。
//! 需要区分普通窗口流程与平台桌面挂载逻辑，避免两者互相混杂。

pub(crate) mod desktop_background;
mod standard;

pub(crate) use standard::{
    apply_gallery_window_preferences, gallery_background_color, run_window_task,
    set_current_window_fullscreen, show_window, show_window_from_settings,
};
