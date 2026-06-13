//! Tauri 应用的组装层。
//! 负责初始化全局状态、插件、托盘菜单和启动时窗口行为。
//! 具体命令和窗口实现应放在 app::commands 和 window 系统模块中。

mod commands;
pub(crate) mod labels;
pub(crate) mod state;
#[cfg(desktop)]
mod updates;

use crate::{
    library::source_paths::start_resource_cleanup,
    shared::models::ThumbnailProgress,
    storage::{
        asset_scope::refresh_asset_scope,
        config::{
            launched_from_desktop_background_startup, sync_windows_startup_registry_from_config,
        },
    },
    window::desktop_background::{
        spawn_show_desktop_background_window, toggle_desktop_background_window,
    },
    window::show_window,
};
use labels::{CAROUSEL_LABEL, GALLERY_LABEL, SETTINGS_LABEL};
use state::{KeepAwake, KeepAwakeState, ThumbnailProgressState, WindowsFullscreenRestoreState};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

const SETTINGS_MENU_ID: &str = "open_settings";
const GALLERY_MENU_ID: &str = "open_gallery";
const CAROUSEL_MENU_ID: &str = "open_carousel";
const DESKTOP_BACKGROUND_MENU_ID: &str = "toggle_desktop_background";
const QUIT_MENU_ID: &str = "quit";

pub(crate) fn tray_icon() -> Option<tauri::image::Image<'static>> {
    let icon = image::load_from_memory(include_bytes!("../../icons/bar-icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = icon.dimensions();
    Some(tauri::image::Image::new_owned(
        icon.into_raw(),
        width,
        height,
    ))
}

#[cfg(target_os = "windows")]
pub(crate) fn claim_single_instance() -> bool {
    use std::{
        ffi::{c_void, OsStr},
        os::windows::ffi::OsStrExt,
        ptr,
    };

    type Handle = *mut c_void;
    const ERROR_ALREADY_EXISTS: u32 = 183;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            lp_mutex_attributes: *mut c_void,
            b_initial_owner: i32,
            lp_name: *const u16,
        ) -> Handle;
        fn GetLastError() -> u32;
    }

    let name = OsStr::new("Local\\com.wang.gallery2.single-instance")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    unsafe {
        let handle = CreateMutexW(ptr::null_mut(), 1, name.as_ptr());
        !handle.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn claim_single_instance() -> bool {
    true
}

pub(crate) fn initial_thumbnail_progress() -> ThumbnailProgress {
    ThumbnailProgress {
        running: false,
        stage: "idle".to_string(),
        processed: 0,
        total: 0,
        generated: 0,
        skipped: 0,
        message: String::new(),
        error: String::new(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if !claim_single_instance() {
        return;
    }

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(initial_thumbnail_progress())) as ThumbnailProgressState)
        .manage(Arc::new(Mutex::new(HashSet::<String>::new())) as WindowsFullscreenRestoreState)
        .manage(Arc::new(Mutex::new(KeepAwake::default())) as KeepAwakeState)
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Err(err) = refresh_asset_scope(&app.handle().clone()) {
                eprintln!("Failed to refresh asset scope: {err}");
            }
            start_resource_cleanup(app.handle().clone());
            sync_windows_startup_registry_from_config(&app.handle().clone());

            let settings = MenuItem::with_id(app, SETTINGS_MENU_ID, "设置", true, None::<&str>)?;
            let gallery = MenuItem::with_id(app, GALLERY_MENU_ID, "瀑布流", true, None::<&str>)?;
            let carousel = MenuItem::with_id(app, CAROUSEL_MENU_ID, "走马灯", true, None::<&str>)?;
            let desktop_background = MenuItem::with_id(
                app,
                DESKTOP_BACKGROUND_MENU_ID,
                "打开桌面背景",
                true,
                None::<&str>,
            )?;
            let desktop_background_top_separator = PredefinedMenuItem::separator(app)?;
            let desktop_background_bottom_separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, QUIT_MENU_ID, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &settings,
                    &gallery,
                    &carousel,
                    &desktop_background_top_separator,
                    &desktop_background,
                    &desktop_background_bottom_separator,
                    &quit,
                ],
            )?;
            let desktop_background_item = desktop_background.clone();
            let mut tray_builder = TrayIconBuilder::with_id("gallery")
                .tooltip("Gallery")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| {
                    let id = event.id().as_ref();
                    if id == QUIT_MENU_ID {
                        app.exit(0);
                        return;
                    }
                    if id == DESKTOP_BACKGROUND_MENU_ID {
                        match toggle_desktop_background_window(app) {
                            Ok(open) => {
                                let _ = desktop_background_item.set_text(if open {
                                    "关闭桌面背景"
                                } else {
                                    "打开桌面背景"
                                });
                            }
                            Err(err) => eprintln!("Failed to toggle desktop background: {err}"),
                        }
                        return;
                    }

                    let label = match id {
                        SETTINGS_MENU_ID => Some(SETTINGS_LABEL),
                        GALLERY_MENU_ID => Some(GALLERY_LABEL),
                        CAROUSEL_MENU_ID => Some(CAROUSEL_LABEL),
                        _ => None,
                    };
                    if let Some(label) = label {
                        let _ = show_window(app, label);
                    }
                });

            if let Some(icon) = tray_icon() {
                tray_builder = tray_builder.icon(icon).icon_as_template(false);
            } else if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;
            #[cfg(desktop)]
            app.manage(updates::initialize(&app.handle().clone()));
            if launched_from_desktop_background_startup() {
                let _ = desktop_background.set_text("关闭桌面背景");
                spawn_show_desktop_background_window(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_app_window,
            commands::open_gallery_from_settings,
            commands::open_carousel_from_settings,
            commands::set_current_window_fullscreen,
            commands::get_settings,
            commands::get_gallery_preferences,
            commands::save_gallery_preferences,
            commands::save_source_paths,
            commands::save_windows_close_behavior,
            commands::save_windows_startup_settings,
            commands::save_xai_settings,
            commands::get_xai_key_status,
            commands::save_thumbnail_settings,
            commands::start_thumbnail_generation,
            commands::get_thumbnail_progress,
            commands::pick_source_folders,
            commands::pick_duplicate_folder,
            commands::pick_generated_content_folder,
            commands::pick_thumbnail_folder,
            commands::pick_xai_reference_images,
            commands::scan_library,
            commands::deduplicate_resources,
            commands::repair_image_extensions,
            commands::read_image_data_uri,
            commands::save_generated_image,
            commands::archive_xai_edit,
            commands::edit_image_with_xai,
            #[cfg(desktop)]
            commands::check_app_update,
            #[cfg(desktop)]
            commands::install_app_update,
            commands::load_editor_session,
            commands::save_editor_session,
            commands::list_images,
            commands::list_random_images
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
