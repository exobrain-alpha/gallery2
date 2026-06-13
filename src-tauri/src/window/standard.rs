//! 普通应用窗口生命周期控制。
//! 负责 settings/gallery/carousel 窗口的创建、显示、关闭策略、全屏恢复和走马灯保持唤醒。
//! 桌面背景窗口独立放在 window::desktop_background，避免平台挂载逻辑混入普通窗口流程。

#[cfg(target_os = "windows")]
use crate::storage::{
    config::{
        persist_windows_close_behavior, windows_close_behavior, WINDOWS_CLOSE_BEHAVIOR_ASK,
        WINDOWS_CLOSE_BEHAVIOR_EXIT, WINDOWS_CLOSE_BEHAVIOR_TRAY,
    },
    db::open_db,
};
use crate::{
    app::{
        labels::{
            is_gallery_window, CAROUSEL_LABEL, DESKTOP_BACKGROUND_LABEL, GALLERY_LABEL,
            SETTINGS_LABEL,
        },
        state::{KeepAwakeState, WindowsFullscreenRestoreState},
    },
    shared::models::GalleryPreferences,
    storage::{asset_scope::refresh_asset_scope, config::get_gallery_preferences_from_app},
};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc,
    time::Duration,
};
#[cfg(target_os = "macos")]
use tauri::TitleBarStyle;
use tauri::{
    window::Color, LogicalSize, Manager, Size, State, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
#[cfg(target_os = "windows")]
use tauri_plugin_dialog::DialogExt;
#[cfg(target_os = "windows")]
use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogKind};

pub(crate) fn gallery_background_color(preferences: &GalleryPreferences) -> Color {
    if preferences.theme == "black" {
        Color(26, 27, 30, 255)
    } else {
        Color(255, 255, 255, 255)
    }
}

pub(crate) fn apply_gallery_window_preferences(
    window: &WebviewWindow,
    preferences: &GalleryPreferences,
) -> Result<(), String> {
    let color = gallery_background_color(preferences);
    window
        .set_background_color(Some(color))
        .map_err(|err| format!("Failed to set gallery background color: {err}"))?;
    window
        .set_theme(Some(if preferences.theme == "black" {
            Theme::Dark
        } else {
            Theme::Light
        }))
        .map_err(|err| format!("Failed to set gallery theme: {err}"))?;
    Ok(())
}

pub(crate) fn show_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    if label == DESKTOP_BACKGROUND_LABEL {
        return Err("Desktop background can only be controlled from the tray".to_string());
    }

    if let Some(window) = app.get_webview_window(label) {
        if is_gallery_window(label) {
            refresh_asset_scope(app)?;
            let preferences = get_gallery_preferences_from_app(app)?;
            apply_gallery_window_preferences(&window, &preferences)?;
        } else if label == SETTINGS_LABEL {
            window
                .eval("window.location.reload()")
                .map_err(|err| format!("Failed to reload settings window: {err}"))?;
        }
        bring_window_to_front(&window)?;
        let keep_awake_state = app.state::<KeepAwakeState>().inner().clone();
        update_carousel_keep_awake(&window, &keep_awake_state);
        return Ok(());
    }

    let (title, view, width, height) = match label {
        SETTINGS_LABEL => ("Gallery Settings", "settings", 760.0, 620.0),
        GALLERY_LABEL => ("Gallery", "gallery", 1240.0, 860.0),
        CAROUSEL_LABEL => ("Carousel", "carousel", 1240.0, 860.0),
        _ => return Err(format!("Unknown window label: {label}")),
    };

    let gallery_preferences = if is_gallery_window(label) {
        refresh_asset_scope(app)?;
        Some(get_gallery_preferences_from_app(app)?)
    } else {
        None
    };
    let url = if let Some(preferences) = &gallery_preferences {
        format!("index.html?view={view}&theme={}", preferences.theme)
    } else {
        format!("index.html?view={view}")
    };
    let background_color = if is_gallery_window(label) {
        gallery_background_color(
            gallery_preferences
                .as_ref()
                .ok_or_else(|| "Missing gallery preferences".to_string())?,
        )
    } else {
        Color(246, 246, 244, 255)
    };

    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(520.0, 420.0)
        .resizable(true)
        .maximizable(true)
        .background_color(background_color)
        .center();

    if is_gallery_window(label) {
        let preferences = gallery_preferences
            .as_ref()
            .ok_or_else(|| "Missing gallery preferences".to_string())?;
        #[cfg(target_os = "macos")]
        {
            builder = builder.title_bar_style(TitleBarStyle::Visible);
        }
        builder = builder.theme(Some(if preferences.theme == "black" {
            Theme::Dark
        } else {
            Theme::Light
        }));
    }

    let window = builder
        .build()
        .map_err(|err| format!("Failed to build window: {err}"))?;
    if let Some(preferences) = &gallery_preferences {
        apply_gallery_window_preferences(&window, &preferences)?;
    }
    attach_close_handler(&window);
    let fullscreen_restore_state = app.state::<WindowsFullscreenRestoreState>().inner().clone();
    let keep_awake_state = app.state::<KeepAwakeState>().inner().clone();
    attach_windows_fullscreen_handler(&window, fullscreen_restore_state, keep_awake_state.clone());
    attach_carousel_keep_awake_handler(&window, keep_awake_state.clone());
    update_carousel_keep_awake(&window, &keep_awake_state);

    bring_window_to_front(&window)?;
    Ok(())
}

pub(crate) fn show_window_from_settings(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    let settings_window = app.get_webview_window(SETTINGS_LABEL);
    if let Some(window) = &settings_window {
        window
            .hide()
            .map_err(|err| format!("Failed to hide settings window: {err}"))?;
    }

    if let Err(err) = show_window(app, label) {
        if let Some(window) = settings_window {
            let _ = bring_window_to_front(&window);
        }
        return Err(err);
    }

    Ok(())
}

pub(crate) async fn run_window_task(
    app: tauri::AppHandle,
    label: &'static str,
    task: impl FnOnce(tauri::AppHandle) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        let result = catch_unwind(AssertUnwindSafe(|| task(app_for_task)))
            .unwrap_or_else(|_| Err(format!("{label} panicked")));
        let _ = sender.send(result);
    })
    .map_err(|err| format!("Failed to schedule {label}: {err}"))?;

    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .map_err(|err| format!("Failed to receive {label}: {err}"))?
    })
    .await
    .map_err(|err| format!("Failed to wait for {label}: {err}"))?
}

fn attach_close_handler(window: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        let window_for_close = window.clone();
        let prompt_open = Arc::new(Mutex::new(false));
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window_for_close.app_handle().clone();
                let behavior = open_db(&app)
                    .and_then(|conn| windows_close_behavior(&conn))
                    .unwrap_or_else(|_| WINDOWS_CLOSE_BEHAVIOR_ASK.to_string());
                if behavior == WINDOWS_CLOSE_BEHAVIOR_EXIT {
                    app.exit(0);
                    return;
                }
                if behavior == WINDOWS_CLOSE_BEHAVIOR_TRAY {
                    let _ = window_for_close.hide();
                    return;
                }

                let Ok(mut is_prompt_open) = prompt_open.lock() else {
                    return;
                };
                if *is_prompt_open {
                    return;
                }
                *is_prompt_open = true;
                drop(is_prompt_open);

                let window_for_dialog = window_for_close.clone();
                let window_for_action = window_for_close.clone();
                let prompt_open_after_close = Arc::clone(&prompt_open);
                window_for_dialog
                    .dialog()
                    .message("本次关闭方式将作为默认选择保存，之后可在设置中调整。")
                    .parent(&window_for_dialog)
                    .title("关闭 Gallery")
                    .kind(MessageDialogKind::Info)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "退出应用".to_string(),
                        "保留托盘".to_string(),
                    ))
                    .show(move |should_exit| {
                        if let Ok(mut is_prompt_open) = prompt_open_after_close.lock() {
                            *is_prompt_open = false;
                        }
                        let close_behavior = if should_exit {
                            WINDOWS_CLOSE_BEHAVIOR_EXIT
                        } else {
                            WINDOWS_CLOSE_BEHAVIOR_TRAY
                        };
                        if let Err(err) =
                            persist_windows_close_behavior(&app, close_behavior.to_string())
                        {
                            eprintln!("Failed to persist Windows close behavior: {err}");
                        }
                        let app_for_action = app.clone();
                        let fallback_window = window_for_action.clone();
                        let fallback_app = app.clone();
                        if should_exit {
                            if let Err(err) = app.run_on_main_thread(move || {
                                app_for_action.exit(0);
                            }) {
                                eprintln!("Failed to schedule app exit: {err}");
                                fallback_app.exit(0);
                            }
                        } else {
                            if let Err(err) = app.run_on_main_thread(move || {
                                let _ = window_for_action.hide();
                            }) {
                                eprintln!("Failed to schedule window hide: {err}");
                                let _ = fallback_window.hide();
                            }
                        }
                    });
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let window_to_hide = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_to_hide.hide();
            }
        });
    }
}

#[cfg(target_os = "windows")]
fn attach_windows_fullscreen_handler(
    window: &WebviewWindow,
    fullscreen_restore_state: WindowsFullscreenRestoreState,
    keep_awake_state: KeepAwakeState,
) {
    let label = window.label().to_string();
    let window_for_fullscreen = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Resized(_) = event {
            let is_restoring = fullscreen_restore_state
                .lock()
                .map(|state| state.contains(&label))
                .unwrap_or(false);
            if is_restoring {
                return;
            }
            let is_maximized = window_for_fullscreen.is_maximized().unwrap_or(false);
            let is_fullscreen = window_for_fullscreen.is_fullscreen().unwrap_or(false);
            if is_maximized && !is_fullscreen {
                let _ = window_for_fullscreen.set_fullscreen(true);
                if label == CAROUSEL_LABEL {
                    set_carousel_keep_awake_active(&keep_awake_state, true);
                }
            }
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn attach_windows_fullscreen_handler(
    _window: &WebviewWindow,
    _fullscreen_restore_state: WindowsFullscreenRestoreState,
    _keep_awake_state: KeepAwakeState,
) {
}

fn update_carousel_keep_awake(window: &WebviewWindow, keep_awake_state: &KeepAwakeState) {
    let active = window.label() == CAROUSEL_LABEL && window.is_fullscreen().unwrap_or(false);
    set_carousel_keep_awake_active(keep_awake_state, active);
}

fn set_carousel_keep_awake_active(keep_awake_state: &KeepAwakeState, active: bool) {
    if let Ok(mut keep_awake) = keep_awake_state.lock() {
        if let Err(err) = keep_awake.set_active(active) {
            eprintln!("Failed to update keep-awake state: {err}");
        }
    }
}

fn attach_carousel_keep_awake_handler(window: &WebviewWindow, keep_awake_state: KeepAwakeState) {
    if window.label() != CAROUSEL_LABEL {
        return;
    }

    let window_for_event = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => {
            update_carousel_keep_awake(&window_for_event, &keep_awake_state);
        }
        WindowEvent::CloseRequested { .. } => {
            set_carousel_keep_awake_active(&keep_awake_state, false);
        }
        _ => {}
    });
}

fn bring_window_to_front(window: &WebviewWindow) -> Result<(), String> {
    window
        .show()
        .map_err(|err| format!("Failed to show window: {err}"))?;
    window
        .unminimize()
        .map_err(|err| format!("Failed to restore window: {err}"))?;
    window
        .set_focus()
        .map_err(|err| format!("Failed to focus window: {err}"))?;
    Ok(())
}

pub(crate) fn set_current_window_fullscreen(
    window: WebviewWindow,
    fullscreen: bool,
    fullscreen_restore_state: State<'_, WindowsFullscreenRestoreState>,
    keep_awake_state: State<'_, KeepAwakeState>,
) -> Result<(), String> {
    let label = window.label().to_string();
    if !fullscreen {
        if let Ok(mut state) = fullscreen_restore_state.lock() {
            state.insert(label.clone());
        }
    }
    window
        .set_fullscreen(fullscreen)
        .map_err(|err| format!("Failed to set fullscreen: {err}"))?;
    set_carousel_keep_awake_active(
        keep_awake_state.inner(),
        label == CAROUSEL_LABEL && fullscreen,
    );
    if !fullscreen {
        std::thread::sleep(Duration::from_millis(120));
        let _ = window.set_decorations(true);
        let _ = window.unmaximize();
        window
            .set_size(Size::Logical(LogicalSize::new(1240.0, 860.0)))
            .map_err(|err| format!("Failed to restore window size: {err}"))?;
        window
            .center()
            .map_err(|err| format!("Failed to restore window position: {err}"))?;
        if let Ok(mut state) = fullscreen_restore_state.lock() {
            state.remove(&label);
        }
    }
    Ok(())
}
