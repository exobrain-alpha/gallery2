//! 前端 invoke 调用的命令适配层。
//! 负责参数整理、阻塞任务调度和把调用转发到具体业务模块。
//! 复杂业务逻辑应下沉到 library/editor/window/storage 等系统模块。

#[cfg(desktop)]
use super::updates;
use super::{
    labels::{CAROUSEL_LABEL, DESKTOP_BACKGROUND_LABEL, GALLERY_LABEL},
    state::{KeepAwakeState, ThumbnailProgressState, WindowsFullscreenRestoreState},
};
use crate::{
    editor::{session, xai},
    library::{
        gallery,
        media::is_supported_image,
        scanner,
        source_paths::{collect_roots, replace_source_paths, user_path_strings},
    },
    shared::{
        models::*,
        path_utils::{canonical_user_path, user_path_buf, user_path_string},
    },
    storage::{
        asset_scope::{refresh_asset_scope_with_conn, source_roots_from_conn},
        config::{
            configured_generated_content_dir, configured_thumbnail_dir, current_platform,
            default_thumbnail_dir, get_gallery_preferences_from_app,
            get_gallery_preferences_from_conn, normalize_gallery_mode,
            normalize_gallery_preferences, persist_windows_close_behavior,
            persist_windows_startup_settings, thumbnail_enabled, windows_close_behavior,
            windows_startup_settings,
        },
        db::{open_db, read_config, write_config},
        paths::db_path,
    },
    window::{self, desktop_background},
};
use std::{fs, path::PathBuf};
use tauri::{Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub(crate) fn set_current_window_fullscreen(
    window: WebviewWindow,
    fullscreen: bool,
    fullscreen_restore_state: State<'_, WindowsFullscreenRestoreState>,
    keep_awake_state: State<'_, KeepAwakeState>,
) -> Result<(), String> {
    window::set_current_window_fullscreen(
        window,
        fullscreen,
        fullscreen_restore_state,
        keep_awake_state,
    )
}

#[tauri::command]
pub(crate) async fn open_app_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    window::run_window_task(app, "open window", move |app| {
        window::show_window(&app, &label)
    })
    .await
}

#[tauri::command]
pub(crate) async fn open_gallery_from_settings(app: tauri::AppHandle) -> Result<(), String> {
    window::run_window_task(app, "open gallery from settings", move |app| {
        window::show_window_from_settings(&app, GALLERY_LABEL)
    })
    .await
}

#[tauri::command]
pub(crate) async fn open_carousel_from_settings(app: tauri::AppHandle) -> Result<(), String> {
    window::run_window_task(app, "open carousel from settings", move |app| {
        window::show_window_from_settings(&app, CAROUSEL_LABEL)
    })
    .await
}

#[tauri::command]
pub(crate) fn get_settings(app: tauri::AppHandle) -> Result<SettingsState, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT path FROM source_paths ORDER BY path COLLATE NOCASE")
        .map_err(|err| format!("Failed to read source paths: {err}"))?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("Failed to query source paths: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect source paths: {err}"))?
        .into_iter()
        .map(|path| user_path_string(&PathBuf::from(path)))
        .collect();
    let image_count = conn
        .query_row("SELECT COUNT(*) FROM images", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("Failed to count images: {err}"))?;
    let preferences = get_gallery_preferences_from_conn(&conn)?;
    let startup_settings = windows_startup_settings(&conn)?;

    Ok(SettingsState {
        platform: current_platform(),
        paths,
        image_count,
        db_path: user_path_string(&db_path(&app)?),
        generated_content_dir: user_path_string(&configured_generated_content_dir(&app, &conn)?),
        app_version: app.package_info().version.to_string(),
        thumbnail_enabled: thumbnail_enabled(&conn)?,
        thumbnail_dir: user_path_string(&configured_thumbnail_dir(&app, &conn)?),
        xai_key: crate::storage::config::read_xai_key_config(&app, &conn)?,
        gallery_mode: normalize_gallery_mode(read_config(&conn, "gallery_mode", "")?, &preferences),
        gallery_has_gap: preferences.has_gap,
        gallery_theme: preferences.theme,
        min_column_width: preferences.min_column_width,
        windows_close_behavior: windows_close_behavior(&conn)?,
        windows_startup_enabled: startup_settings.startup_enabled,
        windows_startup_desktop_background: startup_settings.startup_desktop_background,
    })
}

#[tauri::command]
pub(crate) fn get_gallery_preferences(app: tauri::AppHandle) -> Result<GalleryPreferences, String> {
    get_gallery_preferences_from_app(&app)
}

#[tauri::command]
pub(crate) fn save_gallery_preferences(
    app: tauri::AppHandle,
    mode: String,
    has_gap: bool,
    theme: String,
    min_column_width: u32,
) -> Result<GalleryPreferences, String> {
    let mut preferences = normalize_gallery_preferences(has_gap, theme);
    preferences.min_column_width = min_column_width.clamp(100, 600);
    let conn = open_db(&app)?;
    write_config(
        &conn,
        "gallery_mode",
        &normalize_gallery_mode(mode, &preferences),
    )?;
    write_config(
        &conn,
        "gallery_has_gap",
        if preferences.has_gap { "true" } else { "false" },
    )?;
    write_config(&conn, "gallery_theme", &preferences.theme)?;
    write_config(
        &conn,
        "min_column_width",
        &preferences.min_column_width.to_string(),
    )?;

    if let Some(window) = app.get_webview_window(GALLERY_LABEL) {
        window::apply_gallery_window_preferences(&window, &preferences)?;
        window
            .eval("window.location.reload()")
            .map_err(|err| format!("Failed to reload gallery window: {err}"))?;
    }
    if let Some(window) = app.get_webview_window(CAROUSEL_LABEL) {
        window::apply_gallery_window_preferences(&window, &preferences)?;
        window
            .eval("window.location.reload()")
            .map_err(|err| format!("Failed to reload carousel window: {err}"))?;
    }
    if let Some(window) = app.get_webview_window(DESKTOP_BACKGROUND_LABEL) {
        window::apply_gallery_window_preferences(&window, &preferences)?;
        desktop_background::apply_desktop_background_window_role(&window)?;
        window
            .eval("window.location.reload()")
            .map_err(|err| format!("Failed to reload desktop background window: {err}"))?;
    }

    Ok(preferences)
}

#[tauri::command]
pub(crate) fn save_source_paths(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<SourcePathsUpdate, String> {
    let roots = collect_roots(&paths);
    let mut conn = open_db(&app)?;
    let existing_paths = user_path_strings(&source_roots_from_conn(&conn)?);
    let incoming_paths = user_path_strings(&roots);
    let changed = incoming_paths != existing_paths;
    let stored_paths = if changed {
        let stored_paths = replace_source_paths(&mut conn, &roots)?;
        refresh_asset_scope_with_conn(&app, &conn)?;
        stored_paths
    } else {
        existing_paths
    };

    Ok(SourcePathsUpdate {
        paths: stored_paths,
        changed,
    })
}

#[tauri::command]
pub(crate) fn save_windows_close_behavior(
    app: tauri::AppHandle,
    close_behavior: String,
) -> Result<String, String> {
    persist_windows_close_behavior(&app, close_behavior)
}

#[tauri::command]
pub(crate) fn save_windows_startup_settings(
    app: tauri::AppHandle,
    startup_enabled: bool,
    startup_desktop_background: bool,
) -> Result<WindowsStartupSettings, String> {
    persist_windows_startup_settings(&app, startup_enabled, startup_desktop_background)
}

#[tauri::command]
pub(crate) fn save_xai_settings(
    app: tauri::AppHandle,
    xai_key: String,
    generated_content_dir: String,
) -> Result<(), String> {
    xai::save_xai_settings(app, xai_key, generated_content_dir)
}

#[tauri::command]
pub(crate) fn get_xai_key_status(app: tauri::AppHandle) -> Result<XaiKeyStatus, String> {
    xai::get_xai_key_status(app)
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn check_app_update(
    app: tauri::AppHandle,
    update_state: State<'_, updates::AppUpdateRuntimeState>,
) -> Result<AppUpdateInfo, String> {
    updates::check(app, update_state.inner()).await
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn install_app_update(
    app: tauri::AppHandle,
    update_state: State<'_, updates::AppUpdateRuntimeState>,
) -> Result<(), String> {
    updates::install(app, update_state.inner()).await
}

#[tauri::command]
pub(crate) fn save_thumbnail_settings(
    app: tauri::AppHandle,
    thumbnail_enabled: bool,
    thumbnail_dir: String,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    if !thumbnail_enabled {
        write_config(&conn, "thumbnail_enabled", "false")?;
        refresh_asset_scope_with_conn(&app, &conn)?;
        return Ok(());
    }

    let dir = if thumbnail_dir.trim().is_empty() {
        default_thumbnail_dir(&app)?
    } else {
        PathBuf::from(thumbnail_dir.trim())
    };
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create thumbnail directory: {err}"))?;
    let dir = canonical_user_path(&dir)
        .map_err(|err| format!("Failed to canonicalize thumbnail directory: {err}"))?;
    write_config(
        &conn,
        "thumbnail_enabled",
        if thumbnail_enabled { "true" } else { "false" },
    )?;
    write_config(&conn, "thumbnail_dir", &user_path_string(&dir))?;
    refresh_asset_scope_with_conn(&app, &conn)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn start_thumbnail_generation(app: tauri::AppHandle) -> Result<(), String> {
    let _ = app;
    Err("缩略图生成已禁用".to_string())
}

#[tauri::command]
pub(crate) fn get_thumbnail_progress(app: tauri::AppHandle) -> Result<ThumbnailProgress, String> {
    app.state::<ThumbnailProgressState>()
        .lock()
        .map(|state| state.clone())
        .map_err(|_| "Failed to read thumbnail progress".to_string())
}

#[tauri::command]
pub(crate) async fn pick_source_folders(window: tauri::Window) -> Result<Vec<String>, String> {
    let folders = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择素材文件夹")
            .blocking_pick_folders()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    let mut paths = folders
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.into_path().ok())
        .filter(|path| path.is_dir())
        .map(|path| user_path_string(&path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[tauri::command]
pub(crate) async fn pick_duplicate_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择重复资源保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve duplicate folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
pub(crate) async fn pick_generated_content_folder(
    window: tauri::Window,
) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择生成内容保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve generated content folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
pub(crate) async fn pick_thumbnail_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择缩略图保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve thumbnail folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
pub(crate) async fn pick_xai_reference_images(
    window: tauri::Window,
) -> Result<Vec<PickedImage>, String> {
    let files = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择参考图片")
            .blocking_pick_files()
    })
    .await
    .map_err(|err| format!("Failed to open image picker: {err}"))?;

    files
        .unwrap_or_default()
        .into_iter()
        .take(3)
        .filter_map(|path| path.into_path().ok())
        .filter(|path| path.is_file() && is_supported_image(path))
        .map(|path| {
            let path = user_path_buf(path);
            let data_uri = xai::read_image_data_uri(user_path_string(&path))?;
            Ok(PickedImage {
                path: user_path_string(&path),
                data_uri,
            })
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn scan_library(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<ScanSummary, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::scan_library(app, paths))
        .await
        .map_err(|err| format!("Failed to run scan: {err}"))?
}

#[tauri::command]
pub(crate) async fn deduplicate_resources(
    app: tauri::AppHandle,
    paths: Vec<String>,
    destination_path: String,
) -> Result<DedupeSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scanner::deduplicate_resources(app, paths, destination_path)
    })
    .await
    .map_err(|err| format!("Failed to run duplicate detection: {err}"))?
}

#[tauri::command]
pub(crate) async fn repair_image_extensions(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<ExtensionRepairSummary, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::repair_image_extensions(app, paths))
        .await
        .map_err(|err| format!("Failed to run extension repair: {err}"))?
}

#[tauri::command]
pub(crate) fn read_image_data_uri(path: String) -> Result<String, String> {
    xai::read_image_data_uri(path)
}

#[tauri::command]
pub(crate) fn save_generated_image(
    app: tauri::AppHandle,
    data_uri: String,
    source_path: String,
) -> Result<SavedGeneratedImage, String> {
    xai::save_generated_image(app, data_uri, source_path)
}

#[tauri::command]
pub(crate) fn archive_xai_edit(
    app: tauri::AppHandle,
    entry: XaiEditArchiveEntry,
) -> Result<(), String> {
    xai::archive_xai_edit(app, entry)
}

#[tauri::command]
pub(crate) async fn edit_image_with_xai(
    app: tauri::AppHandle,
    source_paths: Vec<String>,
    source_data_uris: Vec<String>,
    prompt: String,
    aspect_ratio: Option<String>,
    resolution: Option<String>,
    image_count: Option<u8>,
) -> Result<XaiEditResult, String> {
    xai::edit_image_with_xai(
        app,
        source_paths,
        source_data_uris,
        prompt,
        aspect_ratio,
        resolution,
        image_count,
    )
    .await
}

#[tauri::command]
pub(crate) fn load_editor_session(app: tauri::AppHandle) -> Result<EditorSessionState, String> {
    session::load_editor_session(app)
}

#[tauri::command]
pub(crate) fn save_editor_session(
    app: tauri::AppHandle,
    session_id: String,
    messages: Vec<EditorSessionMessage>,
) -> Result<(), String> {
    session::save_editor_session(app, session_id, messages)
}

#[tauri::command]
pub(crate) fn list_images(
    app: tauri::AppHandle,
    cursor: Option<ImageCursor>,
    limit: i64,
) -> Result<ImagePage, String> {
    gallery::list_images(app, cursor, limit)
}

#[tauri::command]
pub(crate) fn list_random_images(
    app: tauri::AppHandle,
    limit: i64,
) -> Result<Vec<ImageRecord>, String> {
    gallery::list_random_images(app, limit)
}
