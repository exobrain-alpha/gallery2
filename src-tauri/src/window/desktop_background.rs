//! 桌面背景窗口的专用生命周期模块。
//! 负责创建、显示、隐藏 desktop_background 窗口，并在 macOS/Windows 上调整窗口层级。
//! 普通应用窗口逻辑留在 window::standard，避免桌面挂载细节扩散。

use super::{apply_gallery_window_preferences, gallery_background_color};
use crate::{
    app::labels::DESKTOP_BACKGROUND_LABEL,
    storage::{asset_scope::refresh_asset_scope, config::get_gallery_preferences_from_app},
};
use tauri::{
    Manager, PhysicalPosition, PhysicalSize, Position, Size, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

fn desktop_bounds(window: &WebviewWindow) -> Result<(i32, i32, u32, u32), String> {
    let monitors = window
        .available_monitors()
        .map_err(|err| format!("Failed to read desktop monitors: {err}"))?;
    let first = monitors
        .first()
        .ok_or_else(|| "No monitor available for desktop background".to_string())?;
    let mut left = first.position().x;
    let mut top = first.position().y;
    let mut right = first.position().x + i32::try_from(first.size().width).unwrap_or(i32::MAX);
    let mut bottom = first.position().y + i32::try_from(first.size().height).unwrap_or(i32::MAX);

    for monitor in monitors.iter().skip(1) {
        let position = monitor.position();
        let size = monitor.size();
        left = left.min(position.x);
        top = top.min(position.y);
        right = right.max(position.x + i32::try_from(size.width).unwrap_or(i32::MAX));
        bottom = bottom.max(position.y + i32::try_from(size.height).unwrap_or(i32::MAX));
    }

    Ok((
        left,
        top,
        u32::try_from((right - left).max(1)).unwrap_or(u32::MAX),
        u32::try_from((bottom - top).max(1)).unwrap_or(u32::MAX),
    ))
}

pub(crate) fn apply_desktop_background_window_role(window: &WebviewWindow) -> Result<(), String> {
    window
        .set_decorations(false)
        .map_err(|err| format!("Failed to disable desktop background decorations: {err}"))?;
    let _ = window.set_shadow(false);
    let _ = window.set_skip_taskbar(true);
    let _ = window.set_focusable(false);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = window.set_always_on_bottom(true);
    let _ = window.set_visible_on_all_workspaces(true);
    #[cfg(not(target_os = "windows"))]
    let _ = window.set_ignore_cursor_events(true);

    let (left, top, width, height) = desktop_bounds(window)?;
    window
        .set_position(Position::Physical(PhysicalPosition::new(left, top)))
        .map_err(|err| format!("Failed to position desktop background: {err}"))?;
    window
        .set_size(Size::Physical(PhysicalSize::new(width, height)))
        .map_err(|err| format!("Failed to size desktop background: {err}"))?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn apply_desktop_background_platform_role(window: &WebviewWindow) -> Result<(), String> {
    if let Ok(hwnd) = window.hwnd() {
        let (_, _, width, height) = desktop_bounds(window).unwrap_or((0, 0, 1, 1));
        unsafe {
            attach_window_to_windows_desktop(hwnd.0 as isize, width, height)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_desktop_background_platform_role(window: &WebviewWindow) -> Result<(), String> {
    if let Ok(ns_window) = window.ns_window() {
        unsafe {
            set_macos_desktop_backdrop_window_level(ns_window);
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn apply_desktop_background_platform_role(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe fn set_macos_desktop_backdrop_window_level(ns_window: *mut std::ffi::c_void) {
    // Keep the backdrop below Finder's desktop icons without using AppKit ordering calls.
    const CG_DESKTOP_ICON_WINDOW_LEVEL_KEY: i32 = 18;
    const NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const NS_WINDOW_COLLECTION_BEHAVIOR_STATIONARY: usize = 1 << 4;
    const NS_WINDOW_COLLECTION_BEHAVIOR_IGNORES_CYCLE: usize = 1 << 6;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGWindowLevelForKey(key: i32) -> i32;
    }

    #[link(name = "objc")]
    extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        #[link_name = "objc_msgSend"]
        fn objc_msg_send_with_isize_arg(
            receiver: *mut std::ffi::c_void,
            selector: *mut std::ffi::c_void,
            arg: isize,
        );
    }

    let backdrop_level = unsafe { CGWindowLevelForKey(CG_DESKTOP_ICON_WINDOW_LEVEL_KEY) } - 1;
    let set_level = unsafe { sel_registerName(b"setLevel:\0".as_ptr().cast()) };
    unsafe {
        objc_msg_send_with_isize_arg(ns_window, set_level, backdrop_level as isize);
    }

    let behavior = NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES
        | NS_WINDOW_COLLECTION_BEHAVIOR_STATIONARY
        | NS_WINDOW_COLLECTION_BEHAVIOR_IGNORES_CYCLE;
    let set_collection_behavior =
        unsafe { sel_registerName(b"setCollectionBehavior:\0".as_ptr().cast()) };
    unsafe {
        objc_msg_send_with_isize_arg(ns_window, set_collection_behavior, behavior as isize);
    }
}

#[cfg(target_os = "windows")]
unsafe fn attach_window_to_windows_desktop(
    hwnd: isize,
    width: u32,
    height: u32,
) -> Result<(), String> {
    type EnumWindowsProc = unsafe extern "system" fn(isize, isize) -> i32;

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class_name: *const u16, window_name: *const u16) -> isize;
        fn FindWindowExW(
            parent: isize,
            child_after: isize,
            class_name: *const u16,
            window_name: *const u16,
        ) -> isize;
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
        fn EnumWindows(callback: Option<EnumWindowsProc>, lparam: isize) -> i32;
        fn SetParent(child: isize, parent: isize) -> isize;
        fn ShowWindow(hwnd: isize, cmd_show: i32) -> i32;
        fn SetWindowPos(
            hwnd: isize,
            insert_after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    #[derive(Default)]
    struct DesktopHostSearch {
        workerw: isize,
    }

    unsafe extern "system" fn find_workerw(hwnd: isize, lparam: isize) -> i32 {
        let shell_view = unsafe {
            FindWindowExW(
                hwnd,
                0,
                wide_null("SHELLDLL_DefView").as_ptr(),
                std::ptr::null(),
            )
        };
        if shell_view != 0 {
            let workerw =
                unsafe { FindWindowExW(0, hwnd, wide_null("WorkerW").as_ptr(), std::ptr::null()) };
            if workerw != 0 {
                unsafe {
                    (*(lparam as *mut DesktopHostSearch)).workerw = workerw;
                }
                return 0;
            }
        }
        1
    }

    const WM_SPAWN_WORKERW: u32 = 0x052c;
    const SMTO_NORMAL: u32 = 0;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SW_SHOWNA: i32 = 8;
    const HWND_TOP: isize = 0;

    let progman = unsafe { FindWindowW(wide_null("Progman").as_ptr(), std::ptr::null()) };
    if progman == 0 {
        return Err("Failed to find Windows Progman desktop host".to_string());
    }

    for (wparam, lparam) in [(0, 0), (0x0d, 0), (0x0d, 1)] {
        let mut message_result = 0usize;
        let _ = unsafe {
            SendMessageTimeoutW(
                progman,
                WM_SPAWN_WORKERW,
                wparam,
                lparam,
                SMTO_NORMAL,
                1000,
                &mut message_result,
            )
        };
    }

    let mut search = DesktopHostSearch::default();
    let _ = unsafe {
        EnumWindows(
            Some(find_workerw),
            &mut search as *mut DesktopHostSearch as isize,
        )
    };
    let workerw = if search.workerw != 0 {
        search.workerw
    } else {
        unsafe { FindWindowExW(progman, 0, wide_null("WorkerW").as_ptr(), std::ptr::null()) }
    };
    if workerw == 0 {
        return Err("Failed to find Windows WorkerW wallpaper host".to_string());
    }

    let _ = unsafe { SetParent(hwnd, workerw) };

    let width = i32::try_from(width).unwrap_or(i32::MAX);
    let height = i32::try_from(height).unwrap_or(i32::MAX);
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        )
    };
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
    Ok(())
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn show_desktop_background_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(DESKTOP_BACKGROUND_LABEL) {
        refresh_asset_scope(app)?;
        let preferences = get_gallery_preferences_from_app(app)?;
        apply_gallery_window_preferences(&window, &preferences)?;
        apply_desktop_background_window_role(&window)?;
        window
            .show()
            .map_err(|err| format!("Failed to show desktop background: {err}"))?;
        apply_desktop_background_platform_role(&window)?;
        return Ok(());
    }

    refresh_asset_scope(app)?;
    let preferences = get_gallery_preferences_from_app(app)?;
    let url = format!("index.html?view=desktop&theme={}", preferences.theme);
    let background_color = gallery_background_color(&preferences);

    let window =
        WebviewWindowBuilder::new(app, DESKTOP_BACKGROUND_LABEL, WebviewUrl::App(url.into()))
            .title("Desktop Background")
            .inner_size(1240.0, 860.0)
            .decorations(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .closable(false)
            .skip_taskbar(true)
            .focusable(false)
            .always_on_bottom(!cfg!(any(target_os = "macos", target_os = "windows")))
            .visible_on_all_workspaces(true)
            .shadow(false)
            .visible(false)
            .background_color(background_color)
            .theme(Some(if preferences.theme == "black" {
                Theme::Dark
            } else {
                Theme::Light
            }))
            .build()
            .map_err(|err| format!("Failed to build desktop background window: {err}"))?;

    apply_gallery_window_preferences(&window, &preferences)?;
    apply_desktop_background_window_role(&window)?;
    window
        .show()
        .map_err(|err| format!("Failed to show desktop background: {err}"))?;
    apply_desktop_background_platform_role(&window)?;
    Ok(())
}

pub(crate) fn spawn_show_desktop_background_window(app: tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(move || {
            if let Err(err) = show_desktop_background_window(&app) {
                eprintln!("Failed to show desktop background: {err}");
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Err(err) = show_desktop_background_window(&app) {
            eprintln!("Failed to show desktop background: {err}");
        }
    }
}

pub(crate) fn toggle_desktop_background_window(app: &tauri::AppHandle) -> Result<bool, String> {
    if let Some(window) = app.get_webview_window(DESKTOP_BACKGROUND_LABEL) {
        if !window.is_visible().unwrap_or(true) {
            spawn_show_desktop_background_window(app.clone());
            return Ok(true);
        }

        window
            .hide()
            .map_err(|err| format!("Failed to close desktop background: {err}"))?;
        return Ok(false);
    }

    spawn_show_desktop_background_window(app.clone());
    Ok(true)
}
